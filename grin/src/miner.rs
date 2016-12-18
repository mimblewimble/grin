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
use std::ops::Deref;
use time;

use core::consensus;
use core::core;
use core::core::hash::{Hash, Hashed};
use core::pow;
use core::pow::cuckoo;
use chain;
use secp;

pub struct Miner {
	chain_head: Arc<Mutex<chain::Tip>>,
	chain_store: Arc<chain::ChainStore>,
}

impl Miner {
	/// Creates a new Miner. Needs references to the chain state and its
	/// storage.
	pub fn new(chain_head: Arc<Mutex<chain::Tip>>, chain_store: Arc<chain::ChainStore>) -> Miner {
		Miner {
			chain_head: chain_head,
			chain_store: chain_store,
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
			let b = self.build_block(&head);
			let mut pow_header = pow::PowHeader::from_block(&b);

			// look for a pow for at most 2 sec on the same block (to give a chance to new
			// transactions) and as long as the head hasn't changed
			let deadline = time::get_time().sec + 2;
			let mut sol = None;
			debug!("Mining at Cuckoo{} for at most 2 secs.",
			       b.header.cuckoo_len);
			while head.hash() == latest_hash && time::get_time().sec < deadline {
				let pow_hash = pow_header.hash();
				let mut miner = cuckoo::Miner::new(pow_hash.to_slice(),
				                                   consensus::EASINESS,
				                                   b.header.cuckoo_len as u32);
				if let Ok(proof) = miner.mine() {
					if proof.to_target() <= b.header.target {
						sol = Some(proof);
						break;
					}
				}
				pow_header.nonce += 1;
				{
					latest_hash = self.chain_head.lock().unwrap().last_block_h;
				}
			}

			// if we found a solution, push our block out
			if let Some(proof) = sol {
				info!("Found valid proof of work, adding block {}.", b.hash());
				if let Err(e) = chain::process_block(&b, self.chain_store.clone(), chain::NONE) {
					error!("Error validating mined block: {:?}", e);
				}
			} else {
				debug!("No solution found, continuing...")
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
		let (target, cuckoo_len) =
			consensus::next_target(now_sec, head_sec, head.target, head.cuckoo_len);

		let mut rng = rand::OsRng::new().unwrap();
		let secp_inst = secp::Secp256k1::with_caps(secp::ContextFlag::Commit);
		// TODO get a new key from the user's wallet or something
		let skey = secp::key::SecretKey::new(&secp_inst, &mut rng);

		// TODO populate inputs and outputs from pool transactions
		core::Block {
			header: core::BlockHeader {
				height: head.height + 1,
				previous: head.hash(),
				timestamp: time::at(time::Timespec::new(now_sec, 0)),
				cuckoo_len: cuckoo_len,
				target: target,
				nonce: rng.gen(),
				..Default::default()
			},
			inputs: vec![],
			outputs: vec![],
			proofs: vec![],
		}
	}
}
