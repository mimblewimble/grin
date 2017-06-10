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
use time;

use adapters::{ChainToPoolAndNetAdapter, PoolToChainAdapter};
use api;
use core::consensus;
use core::core;
use core::core::hash::{Hash, Hashed};
use core::pow::cuckoo;
use core::ser;
use chain;
use secp;
use pool;
use types::{MinerConfig, Error};
use util;

// Max number of transactions this miner will assemble in a block
const MAX_TX: u32 = 5000;

pub struct Miner {
	config: MinerConfig,
	chain_head: Arc<Mutex<chain::Tip>>,
	chain_store: Arc<chain::ChainStore>,
	/// chain adapter to net
	chain_adapter: Arc<ChainToPoolAndNetAdapter>,
	tx_pool: Arc<RwLock<pool::TransactionPool<PoolToChainAdapter>>>,
}

impl Miner {
	/// Creates a new Miner. Needs references to the chain state and its
	/// storage.
	pub fn new(config: MinerConfig,
	           chain_head: Arc<Mutex<chain::Tip>>,
	           chain_store: Arc<chain::ChainStore>,
	           chain_adapter: Arc<ChainToPoolAndNetAdapter>,
	           tx_pool: Arc<RwLock<pool::TransactionPool<PoolToChainAdapter>>>)
	           -> Miner {
		Miner {
			config: config,
			chain_head: chain_head,
			chain_store: chain_store,
			chain_adapter: chain_adapter,
			tx_pool: tx_pool,
		}
	}

	/// Starts the mining loop, building a new block on top of the existing
	/// chain anytime required and looking for PoW solution.
	pub fn run_loop(&self) {
		info!("Starting miner loop.");
		let mut coinbase = self.get_coinbase();
		loop {
			// get the latest chain state and build a block on top of it
			let head: core::BlockHeader;
			let mut latest_hash: Hash;
			{
				head = self.chain_store.head_header().unwrap();
				latest_hash = self.chain_head.lock().unwrap().last_block_h;
			}
			let mut b = self.build_block(&head, coinbase.clone());

			// look for a pow for at most 2 sec on the same block (to give a chance to new
			// transactions) and as long as the head hasn't changed
			let deadline = time::get_time().sec + 2;
			let mut sol = None;
			debug!("Mining at Cuckoo{} for at most 2 secs on block {} at difficulty {}.",
			       b.header.cuckoo_len,
			       latest_hash,
			       b.header.difficulty);
			let mut iter_count = 0;
			while head.hash() == latest_hash && time::get_time().sec < deadline {
				let pow_hash = b.hash();
				let mut miner = cuckoo::Miner::new(&pow_hash[..],
				                                   consensus::EASINESS,
				                                   b.header.cuckoo_len as u32);
				if let Ok(proof) = miner.mine() {
					if proof.to_difficulty() >= b.header.difficulty {
						sol = Some(proof);
						break;
					}
				}
				b.header.nonce += 1;
				{
					latest_hash = self.chain_head.lock().unwrap().last_block_h;
				}
				iter_count += 1;
			}

			// if we found a solution, push our block out
			if let Some(proof) = sol {
				info!("Found valid proof of work, adding block {}.", b.hash());
				b.header.pow = proof;
				let res = chain::process_block(&b,
				                               self.chain_store.clone(),
				                               self.chain_adapter.clone(),
				                               chain::NONE);
				if let Err(e) = res {
					error!("Error validating mined block: {:?}", e);
				} else if let Ok(Some(tip)) = res {
					let chain_head = self.chain_head.clone();
					let mut head = chain_head.lock().unwrap();
					coinbase = self.get_coinbase();
					*head = tip;
				}
			} else {
				debug!("No solution found after {} iterations, continuing...",
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
		let (difficulty, cuckoo_len) =
			consensus::next_target(now_sec, head_sec, head.difficulty.clone(), head.cuckoo_len);

		let txs_box = self.tx_pool.read().unwrap().prepare_mineable_transactions(MAX_TX);
		let txs = txs_box.iter().map(|tx| tx.as_ref()).collect();
		let (output, kernel) = coinbase;
		let mut b = core::Block::with_reward(head, txs, output, kernel).unwrap();

		let mut rng = rand::OsRng::new().unwrap();
		b.header.nonce = rng.gen();
		b.header.cuckoo_len = cuckoo_len;
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
			let url = format!("{}/v1/receive_coinbase",
			                  self.config.wallet_receiver_url.as_str());
			let res: CbData = api::client::post(url.as_str(),
			                                    &CbAmount { amount: consensus::REWARD })
				.expect("Wallet receiver unreachable, could not claim reward. Is it running?");
			let out_bin = util::from_hex(res.output).unwrap();
			let kern_bin = util::from_hex(res.kernel).unwrap();
			let output = ser::deserialize(&mut &out_bin[..]).unwrap();
			let kernel = ser::deserialize(&mut &kern_bin[..]).unwrap();

			(output, kernel)
		}
	}
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct CbAmount {
	amount: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct CbData {
	output: String,
	kernel: String,
}
