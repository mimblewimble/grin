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
use std::sync::{Arc, Mutex};
use time;

use adapters::ChainToNetAdapter;
use core::consensus;
use core::core;
use core::core::hash::{Hash, Hashed};
use core::core::target::Difficulty;
use core::pow;
use core::pow::cuckoo;
use chain;
use secp;

pub struct Miner {
	chain_head: Arc<Mutex<chain::Tip>>,
	chain_store: Arc<chain::ChainStore>,
	/// chain adapter to net
	chain_adapter: Arc<ChainToNetAdapter>,
}

impl Miner {
	/// Creates a new Miner. Needs references to the chain state and its
	/// storage.
	pub fn new(chain_head: Arc<Mutex<chain::Tip>>,
	           chain_store: Arc<chain::ChainStore>,
	           chain_adapter: Arc<ChainToNetAdapter>)
	           -> Miner {
		Miner {
			chain_head: chain_head,
			chain_store: chain_store,
			chain_adapter: chain_adapter,
		}
	}

	/// Starts the mining loop, building a new block on top of the existing
	/// chain anytime required and looking for PoW solution.
	pub fn run_loop(&self) {
		info!("Starting miner loop.");
		loop {
			// get the latest chain state and build a block on top of it
			let head: core::BlockHeader;
			let mut latest_hash: Hash;
			{
				head = self.chain_store.head_header().unwrap();
				latest_hash = self.chain_head.lock().unwrap().last_block_h;
			}
			let mut b = self.build_block(&head);
			let mut pow_header = pow::PowHeader::from_block(&b);

			// look for a pow for at most 2 sec on the same block (to give a chance to new
			// transactions) and as long as the head hasn't changed
			let deadline = time::get_time().sec + 2;
			let mut sol = None;
			debug!("Mining at Cuckoo{} for at most 2 secs on block {}.",
			       b.header.cuckoo_len,
			       latest_hash);
			let mut iter_count = 0;
			while head.hash() == latest_hash && time::get_time().sec < deadline {
				let pow_hash = pow_header.hash();
				let mut miner = cuckoo::Miner::new(pow_hash.to_slice(),
				                                   consensus::EASINESS,
				                                   b.header.cuckoo_len as u32);
				if let Ok(proof) = miner.mine() {
					if proof.to_difficulty() >= b.header.difficulty {
						sol = Some(proof);
						break;
					}
				}
				pow_header.nonce += 1;
				{
					latest_hash = self.chain_head.lock().unwrap().last_block_h;
				}
				iter_count += 1;
			}

			// if we found a solution, push our block out
			if let Some(proof) = sol {
				info!("Found valid proof of work, adding block {}.", b.hash());
				b.header.pow = proof;
				b.header.nonce = pow_header.nonce;
				let res = chain::process_block(&b,
				                               self.chain_store.clone(),
				                               self.chain_adapter.clone(),
				                               chain::NONE);
				if let Err(e) = res {
					error!("Error validating mined block: {:?}", e);
				} else if let Ok(Some(tip)) = res {
					let chain_head = self.chain_head.clone();
					let mut head = chain_head.lock().unwrap();
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
	fn build_block(&self, head: &core::BlockHeader) -> core::Block {
		let mut now_sec = time::get_time().sec;
		let head_sec = head.timestamp.to_timespec().sec;
		if now_sec == head_sec {
			now_sec += 1;
		}
		let (difficulty, cuckoo_len) =
			consensus::next_target(now_sec, head_sec, head.difficulty.clone(), head.cuckoo_len);

		let mut rng = rand::OsRng::new().unwrap();
		let secp_inst = secp::Secp256k1::with_caps(secp::ContextFlag::Commit);
		// TODO get a new key from the user's wallet or something
		let skey = secp::key::SecretKey::new(&secp_inst, &mut rng);

		// TODO populate inputs and outputs from pool transactions
		let mut b = core::Block::new(head, vec![], skey).unwrap();
		b.header.nonce = rng.gen();
		b.header.cuckoo_len = cuckoo_len;
		b.header.difficulty = difficulty;
		b.header.timestamp = time::at(time::Timespec::new(now_sec, 0));
		b
	}
}
