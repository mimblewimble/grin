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
use std::sync::{Arc, RwLock};
use std::thread;
use std;
use std::{str};
use time;

use adapters::{PoolToChainAdapter};
use api;
use core::consensus;
use core::core;
use core::core::Proof;
use core::pow::cuckoo;
use core::core::target::Difficulty;
use core::core::{Block, BlockHeader};
use core::core::hash::{Hash, Hashed};
use core::pow::MiningWorker;
use core::ser;
use core::ser::{AsFixedBytes};

use chain;
use secp;
use pool;
use types::{MinerConfig, ServerConfig};
use util;
use wallet::{CbAmount, WalletReceiveRequest, CbData};

use plugin::PluginMiner;
use itertools::Itertools;

// Max number of transactions this miner will assemble in a block
const MAX_TX: u32 = 5000;

const PRE_NONCE_SIZE: usize = 113;

/// Serializer that outputs pre and post nonce portions of a block header
/// which can then be sent off to miner to mutate at will
pub struct HeaderPartWriter {
	//
	pub pre_nonce: Vec<u8>,
	// Post nonce is currently variable length
	// because of difficulty
	pub post_nonce: Vec<u8>,
	//which difficulty field we're on
	bytes_written: usize,
	writing_pre: bool,
}

impl Default for HeaderPartWriter {
	fn default() -> HeaderPartWriter {
		HeaderPartWriter {
			bytes_written: 0,
			writing_pre: true,
			pre_nonce: Vec::new(),
			post_nonce: Vec::new(),
		}
	}
}

impl HeaderPartWriter {
	pub fn parts_as_hex_strings(&self)->(String, String) {
		(
			String::from(format!("{:02x}", self.pre_nonce.iter().format(""))),
			String::from(format!("{:02x}", self.post_nonce.iter().format(""))),
		)
	}
}

impl ser::Writer for HeaderPartWriter {
	fn serialization_mode(&self) -> ser::SerializationMode {
		ser::SerializationMode::Hash
	}

	fn write_fixed_bytes<T: AsFixedBytes>(&mut self, bytes_in: &T) -> Result<(), ser::Error> {
		if self.writing_pre {
			for i in 0..bytes_in.len() {self.pre_nonce.push(bytes_in.as_ref()[i])};

		} else if self.bytes_written!=0 {
			for i in 0..bytes_in.len() {self.post_nonce.push(bytes_in.as_ref()[i])};
		}

		self.bytes_written+=bytes_in.len();

		if self.bytes_written==PRE_NONCE_SIZE && self.writing_pre {
			self.writing_pre=false;
			self.bytes_written=0;
		}

		Ok(())
	}
}

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

	/// Inner part of the mining loop for cuckoo-miner asynch mode
	pub fn inner_loop_async(&self, plugin_miner:&mut PluginMiner,
							difficulty:Difficulty,
							b:&mut Block,
							cuckoo_size: u32,
							head:&BlockHeader,
							latest_hash:&Hash)
		-> Option<Proof> {

		debug!("(Server ID: {}) Mining at Cuckoo{} for at most 2 secs at height {} and difficulty {}.",
			self.debug_output_id,
			cuckoo_size,
			b.header.height,
			b.header.difficulty);

		// look for a pow for at most 2 sec on the same block (to give a chance to new
		// transactions) and as long as the head hasn't changed
		// Will change this to something else at some point
		let deadline = time::get_time().sec + 2;

		//Get parts of the header
		let mut header_parts = HeaderPartWriter::default();
		ser::Writeable::write(&b.header, &mut header_parts).unwrap();
		let (pre, post) = header_parts.parts_as_hex_strings();

		//Start the miner working
	    let miner = plugin_miner.get_consumable();
    	let job_handle=miner.notify(1, &pre, &post, difficulty.into_num()).unwrap();

		let mut sol=None;

		while head.hash() == *latest_hash && time::get_time().sec < deadline {
			if let Some(s) = job_handle.get_solution()  {
				sol = Some(Proof::new(s.solution_nonces.to_vec()));
				b.header.nonce=s.get_nonce_as_u64();
				break;
			}
		}
		if sol==None {
			debug!("(Server ID: {}) No solution found after {} iterations, continuing...",
				    self.debug_output_id,
					job_handle.get_hashes_since_last_call().unwrap())
		}

		job_handle.stop_jobs();
		sol

	}

	/// The inner part of mining loop for synchronous mode
	pub fn inner_loop_sync<T: MiningWorker>(&self,
						    miner:&mut T,
							b:&mut Block,
							cuckoo_size: u32,
							head:&BlockHeader,
							latest_hash:&mut Hash)
		-> Option<Proof> {
		// look for a pow for at most 2 sec on the same block (to give a chance to new
		// transactions) and as long as the head hasn't changed
		let deadline = time::get_time().sec + 2;

		debug!("(Server ID: {}) Mining at Cuckoo{} for at most 2 secs on block {} at difficulty {}.",
		       self.debug_output_id,
		       cuckoo_size,
		       latest_hash,
		       b.header.difficulty);
		let mut iter_count = 0;

		if self.config.slow_down_in_millis != None && self.config.slow_down_in_millis.unwrap() > 0 {
			debug!("(Server ID: {}) Artificially slowing down loop by {}ms per iteration.",
			self.debug_output_id,
			self.config.slow_down_in_millis.unwrap());
		}

		let mut sol=None;
		while head.hash() == *latest_hash && time::get_time().sec < deadline {

			let pow_hash = b.hash();
			if let Ok(proof) = miner.mine(&pow_hash[..]) {
				let proof_diff=proof.clone().to_difficulty();
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
			*latest_hash = self.chain.head().unwrap().last_block_h;
			iter_count += 1;

			//Artificial slow down
			if self.config.slow_down_in_millis != None && self.config.slow_down_in_millis.unwrap() > 0 {
				thread::sleep(std::time::Duration::from_millis(self.config.slow_down_in_millis.unwrap()));
			}
		}

		if sol==None {
			debug!("(Server ID: {}) No solution found after {} iterations, continuing...",
				self.debug_output_id,
				iter_count)
		}

		sol
	}

	/// Starts the mining loop, building a new block on top of the existing
	/// chain anytime required and looking for PoW solution.
	pub fn run_loop(&self,
					miner_config:MinerConfig,
					server_config:ServerConfig,
					cuckoo_size:u32,
					proof_size:usize) {

		info!("(Server ID: {}) Starting miner loop.", self.debug_output_id);
		let mut plugin_miner=None;
		let mut miner=None;
		if miner_config.use_cuckoo_miner  {
			plugin_miner = Some(PluginMiner::new(consensus::EASINESS, cuckoo_size, proof_size));
			plugin_miner.as_mut().unwrap().init(miner_config.clone(),server_config);
		} else {
			miner = Some(cuckoo::Miner::new(consensus::EASINESS, cuckoo_size, proof_size));
		}

		let mut coinbase = self.get_coinbase();

		loop {
			// get the latest chain state and build a block on top of it
			let head = self.chain.head_header().unwrap();
			let mut latest_hash = self.chain.head().unwrap().last_block_h;
			let mut b = self.build_block(&head, coinbase.clone());

			let mut sol=None;
			let mut use_async=false;
			if let Some(c)=self.config.cuckoo_miner_async_mode {
				if c {
					use_async=true;
				}
			}
			if let Some(mut p) = plugin_miner.as_mut() {
				if use_async {
					sol = self.inner_loop_async(&mut p,
						b.header.difficulty.clone(),
						&mut b,
						cuckoo_size,
						&head,
						&latest_hash);
				} else {
					sol = self.inner_loop_sync(p,
					&mut b,
					cuckoo_size,
					&head,
					&mut latest_hash);
				}
			}
			if let Some(mut m) = miner.as_mut() {
				sol = self.inner_loop_sync(m,
					&mut b,
					cuckoo_size,
					&head,
					&mut latest_hash);
			}

			// if we found a solution, push our block out
			if let Some(proof) = sol {
				info!("(Server ID: {}) Found valid proof of work, adding block {}.",
					  self.debug_output_id, b.hash());
					b.header.pow = proof;
				let opts = if cuckoo_size < consensus::DEFAULT_SIZESHIFT as u32 {
					chain::EASY_POW
				} else {
					chain::NONE
				};
				let res = self.chain.process_block(b, opts);
				if let Err(e) = res {
					error!("(Server ID: {}) Error validating mined block: {:?}",
					self.debug_output_id, e);
				} else {
					coinbase = self.get_coinbase();
				}
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
