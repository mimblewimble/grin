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

use chrono::prelude::Utc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

use chain;
use common::types::StratumServerConfig;
use core::core::hash::{Hash, Hashed};
use core::core::verifier_cache::VerifierCache;
use core::core::{Block, BlockHeader};
use core::global;
use core::pow::PoWContext;
use mining::mine_block;
use pool;
use util::LOGGER;

pub struct Miner {
	config: StratumServerConfig,
	chain: Arc<chain::Chain>,
	tx_pool: Arc<RwLock<pool::TransactionPool>>,
	verifier_cache: Arc<RwLock<VerifierCache>>,
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
		chain: Arc<chain::Chain>,
		tx_pool: Arc<RwLock<pool::TransactionPool>>,
		verifier_cache: Arc<RwLock<VerifierCache>>,
		stop: Arc<AtomicBool>,
	) -> Miner {
		Miner {
			config,
			chain,
			tx_pool,
			verifier_cache,
			debug_output_id: String::from("none"),
			stop,
		}
	}

	/// Keeping this optional so setting in a separate function
	/// instead of in the new function
	pub fn set_debug_output_id(&mut self, debug_output_id: String) {
		self.debug_output_id = debug_output_id;
	}

	/// The inner part of mining loop for the internal miner
	/// kept around mostly for automated testing purposes
	fn inner_mining_loop(
		&self,
		b: &mut Block,
		head: &BlockHeader,
		attempt_time_per_block: u32,
		latest_hash: &mut Hash,
	) -> bool {
		// look for a pow for at most 2 sec on the same block (to give a chance to new
		// transactions) and as long as the head hasn't changed
		let deadline = Utc::now().timestamp() + attempt_time_per_block as i64;

		debug!(
			LOGGER,
			"(Server ID: {}) Mining Cuckoo{} for max {}s on {} @ {} [{}].",
			self.debug_output_id,
			global::min_edge_bits(),
			attempt_time_per_block,
			b.header.total_difficulty(),
			b.header.height,
			latest_hash
		);
		let mut iter_count = 0;

		while head.hash() == *latest_hash && Utc::now().timestamp() < deadline {
			let mut ctx =
				global::create_pow_context::<u32>(global::min_edge_bits(), global::proofsize(), 10)
					.unwrap();
			ctx.set_header_nonce(b.header.pre_pow(), None, true)
				.unwrap();
			if let Ok(proofs) = ctx.find_cycles() {
				b.header.pow.proof = proofs[0].clone();
				let proof_diff = b.header.pow.to_difficulty();
				if proof_diff >= (b.header.total_difficulty() - head.total_difficulty()) {
					return true;
				}
			}

			b.header.pow.nonce += 1;
			*latest_hash = self.chain.head().unwrap().last_block_h;
			iter_count += 1;
		}

		debug!(
			LOGGER,
			"(Server ID: {}) No solution found after {} iterations, continuing...",
			self.debug_output_id,
			iter_count
		);
		false
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

		while !self.stop.load(Ordering::Relaxed) {
			trace!(LOGGER, "in miner loop. key_id: {:?}", key_id);

			// get the latest chain state and build a block on top of it
			let head = self.chain.head_header().unwrap();
			let mut latest_hash = self.chain.head().unwrap().last_block_h;

			let (mut b, block_fees) = mine_block::get_block(
				&self.chain,
				&self.tx_pool,
				self.verifier_cache.clone(),
				key_id.clone(),
				wallet_listener_url.clone(),
			);

			let sol = self.inner_mining_loop(
				&mut b,
				&head,
				self.config.attempt_time_per_block,
				&mut latest_hash,
			);

			// we found a solution, push our block through the chain processing pipeline
			if sol {
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
		}

		info!(
			LOGGER,
			"(Server ID: {}) test miner exit.", self.debug_output_id
		);
	}
}
