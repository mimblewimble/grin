// Copyright 2016 The Grin Developers
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

//! Mining service, gathers transactions from the pool, assemble them in a
//! block and mine the block to produce a valid header with its proof-of-work.

use rand::{self, Rng};
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use std;
use std::env;
use time;

use adapters::{ChainToPoolAndNetAdapter, PoolToChainAdapter};
use api;
use core::consensus;
use core::consensus::*;
use core::core;
use core::core::Proof;
use core::pow::cuckoo;
use core::core::target::Difficulty;
use core::core::hash::{Hash, Hashed};
use core::pow::MiningWorker;
use core::ser;
use chain;
use secp;
use pool;
use types::{MinerConfig, Error};
use util;
use wallet::{CbAmount, WalletReceiveRequest, CbData};

// Max number of transactions this miner will assemble in a block
const MAX_TX: u32 = 5000;

pub struct Miner {
	config: MinerConfig,
	chain: Arc<chain::Chain>,
	tx_pool: Arc<RwLock<pool::TransactionPool<PoolToChainAdapter>>>,

	//Just to hold the port we're on, so this miner can be identified
	//while watching debug output
	debug_output_id: String,
}

impl Miner {
	/// Creates a new Miner. Needs references to the chain state and its
	/// storage.
	pub fn new(config: MinerConfig,
	           chain_ref: Arc<chain::Chain>,
	           tx_pool: Arc<RwLock<pool::TransactionPool<PoolToChainAdapter>>>)
	           -> Miner {
		Miner {
			config: config,
			chain: chain_ref,
			tx_pool: tx_pool,
			debug_output_id: String::from("none"),
		}
	}

	/// Keeping this optional so setting in a separate funciton
	/// instead of in the new function

	pub fn set_debug_output_id(&mut self, debug_output_id: String){
		self.debug_output_id=debug_output_id;
	}


	/// Starts the mining loop, building a new block on top of the existing
	/// chain anytime required and looking for PoW solution.
	pub fn run_loop<T: MiningWorker>(&self, mut miner:T) {

		info!("(Server ID: {}) Starting miner loop.", self.debug_output_id);
		let mut coinbase = self.get_coinbase();

		loop {
			// get the latest chain state and build a block on top of it
			let head = self.chain.head_header().unwrap();
			let mut latest_hash = self.chain.head().unwrap().last_block_h;
			let mut b = self.build_block(&head, coinbase.clone());

			// look for a pow for at most 2 sec on the same block (to give a chance to new
			// transactions) and as long as the head hasn't changed
			let deadline = time::get_time().sec + 2;
			let mut sol = None;
			debug!("(Server ID: {}) Mining at Cuckoo{} for at most 2 secs on block {} at difficulty {}.",
			       self.debug_output_id,
			       self.config.cuckoo_size.unwrap(),
			       latest_hash,
			       b.header.difficulty);
			let mut iter_count = 0;
			
			if self.config.slow_down_in_millis != None && self.config.slow_down_in_millis.unwrap() > 0 {
				debug!("(Server ID: {}) Artificially slowing down loop by {}ms per iteration.",
				self.debug_output_id,
				self.config.slow_down_in_millis.unwrap());
			}
			while head.hash() == latest_hash && time::get_time().sec < deadline {
				let pow_hash = b.hash();
				if let Ok(proof) = miner.mine(&pow_hash[..]) {
					let proof_diff=proof.to_difficulty();
					/*debug!("(Server ID: {}) Header difficulty is: {}, Proof difficulty is: {}",
					self.debug_output_id,
					b.header.difficulty,
					proof_diff);*/

					if proof_diff >= b.header.difficulty {
						sol = Some(proof);
						break;
					}
				}
				b.header.nonce += 1;
				latest_hash = self.chain.head().unwrap().last_block_h;
				iter_count += 1;

				//Artificial slow down
				if self.config.slow_down_in_millis != None && self.config.slow_down_in_millis.unwrap() > 0 {
					thread::sleep(std::time::Duration::from_millis(self.config.slow_down_in_millis.unwrap()));
				}
			}

			// if we found a solution, push our block out
			if let Some(proof) = sol {
				info!("(Server ID: {}) Found valid proof of work, adding block {}.",
					  self.debug_output_id, b.hash());
					b.header.pow = proof;
				let opts = if self.config.cuckoo_size.unwrap() < consensus::DEFAULT_SIZESHIFT as u32 {
					chain::EASY_POW
				} else {
					chain::NONE
				};
				let res = self.chain.process_block(&b, opts);
				if let Err(e) = res {
					error!("(Server ID: {}) Error validating mined block: {:?}",
					self.debug_output_id, e);
				} else {
					coinbase = self.get_coinbase();
				}
			} else {
				debug!("(Server ID: {}) No solution found after {} iterations, continuing...",
				    self.debug_output_id,
					iter_count)
			}
		}
	}

	/// Builds a new block with the chain head as previous and eligible
	/// transactions from the pool.
	fn build_block(&self,
	               head: &core::BlockHeader,
	               coinbase: (core::Output, core::TxKernel))
	               -> core::Block {
		let mut now_sec = time::get_time().sec;
		let head_sec = head.timestamp.to_timespec().sec;
		if now_sec == head_sec {
			now_sec += 1;
		}

		let diff_iter = self.chain.difficulty_iter();
		let difficulty = consensus::next_difficulty(diff_iter).unwrap();

		let txs_box = self.tx_pool.read().unwrap().prepare_mineable_transactions(MAX_TX);
		let txs = txs_box.iter().map(|tx| tx.as_ref()).collect();
		let (output, kernel) = coinbase;
		let mut b = core::Block::with_reward(head, txs, output, kernel).unwrap();
		debug!("(Server ID: {}) Built new block with {} inputs and {} outputs, difficulty: {}",
			   self.debug_output_id,
		       b.inputs.len(),
		       b.outputs.len(),
			   difficulty);

		// making sure we're not spending time mining a useless block
		let secp = secp::Secp256k1::with_caps(secp::ContextFlag::Commit);
		b.validate(&secp).expect("Built an invalid block!");

		let mut rng = rand::OsRng::new().unwrap();
		b.header.nonce = rng.gen();
		b.header.difficulty = difficulty;
		b.header.timestamp = time::at(time::Timespec::new(now_sec, 0));
		b
	}

	fn get_coinbase(&self) -> (core::Output, core::TxKernel) {
		if self.config.burn_reward {
			let mut rng = rand::OsRng::new().unwrap();
			let secp_inst = secp::Secp256k1::with_caps(secp::ContextFlag::Commit);
			let skey = secp::key::SecretKey::new(&secp_inst, &mut rng);
			core::Block::reward_output(skey, &secp_inst).unwrap()
		} else {
			let url = format!("{}/v1/receive/coinbase",
			                  self.config.wallet_receiver_url.as_str());
			let request = WalletReceiveRequest::Coinbase(CbAmount{amount: consensus::REWARD});
			let res: CbData = api::client::post(url.as_str(),
			                                    &request)
				.expect(format!("(Server ID: {}) Wallet receiver unreachable, could not claim reward. Is it running?",
				self.debug_output_id.as_str()).as_str());
			let out_bin = util::from_hex(res.output).unwrap();
			let kern_bin = util::from_hex(res.kernel).unwrap();
			let output = ser::deserialize(&mut &out_bin[..]).unwrap();
			let kernel = ser::deserialize(&mut &kern_bin[..]).unwrap();

			(output, kernel)
		}
	}
}
