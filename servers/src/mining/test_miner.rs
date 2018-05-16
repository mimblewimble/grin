// Copyright 2018 The Grin Developers
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Mining service, gets a block to mine, and based on mining configuration
//! chooses a version of the cuckoo miner to mine the block and produce a valid
//! header with its proof-of-work.  Any valid mined blocks are submitted to the
//! network.

use std::sync::{Arc, RwLock};
use std::sync::atomic::{AtomicBool, Ordering};
use time;

use common::adapters::PoolToChainAdapter;
use core::consensus;
use core::core::Proof;
use core::core::{Block, BlockHeader};
use core::core::hash::{Hash, Hashed};
use core::pow::cuckoo;
use common::types::StratumServerConfig;
use util::LOGGER;

use chain;
use pool;
use mining::mine_block;
use core::global;

// Max number of transactions this miner will assemble in a block
const MAX_TX: u32 = 5000;

pub struct Miner {
	config: StratumServerConfig,
	chain: Arc<chain::Chain>,
	tx_pool: Arc<RwLock<pool::TransactionPool<PoolToChainAdapter>>>,
	stop: Arc<AtomicBool>,

	// Just to hold the port we're on, so this miner can be identified
	// while watching debug output
	debug_output_id: String,
}

impl Miner {
	/// Creates a new Miner. Needs references to the chain state and its
	/// storage.
	pub fn new(
		config: StratumServerConfig,
		chain_ref: Arc<chain::Chain>,
		tx_pool: Arc<RwLock<pool::TransactionPool<PoolToChainAdapter>>>,
		stop: Arc<AtomicBool>,
	) -> Miner {
		Miner {
			config: config,
			chain: chain_ref,
			tx_pool: tx_pool,
			debug_output_id: String::from("none"),
			stop: stop,
		}
	}

	/// Keeping this optional so setting in a separate function
	/// instead of in the new function
	pub fn set_debug_output_id(&mut self, debug_output_id: String) {
		self.debug_output_id = debug_output_id;
	}

	/// The inner part of mining loop for the internal miner
	/// kept around mostly for automated testing purposes
	pub fn inner_mining_loop(
		&self,
		b: &mut Block,
		head: &BlockHeader,
		attempt_time_per_block: u32,
		latest_hash: &mut Hash,
	) -> Option<Proof> {
		// look for a pow for at most 2 sec on the same block (to give a chance to new
		// transactions) and as long as the head hasn't changed
		let deadline = time::get_time().sec + attempt_time_per_block as i64;

		debug!(
			LOGGER,
			"(Server ID: {}) Mining Cuckoo{} for max {}s on {} @ {} [{}].",
			self.debug_output_id,
			global::sizeshift(),
			attempt_time_per_block,
			b.header.total_difficulty,
			b.header.height,
			latest_hash
		);
		let mut iter_count = 0;

		let mut sol = None;
		while head.hash() == *latest_hash && time::get_time().sec < deadline {
			if let Ok(proof) = cuckoo::Miner::new(
				&b.header,
				consensus::EASINESS,
				global::proofsize(),
				global::sizeshift(),
			).mine()
			{
				let proof_diff = proof.clone().to_difficulty();
				if proof_diff >= (b.header.total_difficulty.clone() - head.total_difficulty.clone())
				{
					sol = Some(proof);
					break;
				}
			}

			b.header.nonce += 1;
			*latest_hash = self.chain.head().unwrap().last_block_h;
			iter_count += 1;
		}

		if sol == None {
			debug!(
				LOGGER,
				"(Server ID: {}) No solution found after {} iterations, continuing...",
				self.debug_output_id,
				iter_count
			)
		}

		sol
	}

	/// Starts the mining loop, building a new block on top of the existing
	/// chain anytime required and looking for PoW solution.
	pub fn run_loop(&self, wallet_listener_url: Option<String>) {
		info!(
			LOGGER,
			"(Server ID: {}) Starting test miner loop.", self.debug_output_id
		);

		// iteration, we keep the returned derivation to provide it back when
		// nothing has changed. We only want to create a new key_id for each new block.
		let mut key_id = None;

		loop {
			trace!(LOGGER, "in miner loop. key_id: {:?}", key_id);

			// get the latest chain state and build a block on top of it
			let head = self.chain.head_header().unwrap();
			let mut latest_hash = self.chain.head().unwrap().last_block_h;

			let (mut b, block_fees) = mine_block::get_block(
				&self.chain,
				&self.tx_pool,
				key_id.clone(),
				MAX_TX.clone(),
				wallet_listener_url.clone(),
			);

			let sol = self.inner_mining_loop(
				&mut b,
				&head,
				self.config.attempt_time_per_block,
				&mut latest_hash,
			);

			// we found a solution, push our block through the chain processing pipeline
			if let Some(proof) = sol {
				b.header.pow = proof;
				info!(
					LOGGER,
					"(Server ID: {}) Found valid proof of work, adding block {}.",
					self.debug_output_id,
					b.hash()
				);
				let res = self.chain.process_block(b, chain::Options::MINE);
				if let Err(e) = res {
					error!(
						LOGGER,
						"(Server ID: {}) Error validating mined block: {:?}",
						self.debug_output_id,
						e
					);
				}
				trace!(LOGGER, "resetting key_id in miner to None");
				key_id = None;
			} else {
				debug!(
					LOGGER,
					"setting pubkey in miner to pubkey from block_fees - {:?}", block_fees
				);
				key_id = block_fees.key_id();
			}

			if self.stop.load(Ordering::Relaxed) {
				break;
			}
		}
	}
}
