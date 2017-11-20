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
use std::{str, thread};
use std;
use time;

use adapters::PoolToChainAdapter;
use core::consensus;
use core::core;
use core::core::Proof;
use pow::cuckoo;
use core::core::target::Difficulty;
use core::core::{Block, BlockHeader, Transaction};
use core::core::hash::{Hash, Hashed};
use pow::MiningWorker;
use pow::types::MinerConfig;
use core::ser;
use core::ser::AsFixedBytes;
use util::LOGGER;
use types::Error;

use chain;
use pool;
use util;
use keychain::{Identifier, Keychain};
use wallet;
use wallet::BlockFees;

use pow::plugin::PluginMiner;

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
	// which difficulty field we're on
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
	pub fn parts_as_hex_strings(&self) -> (String, String) {
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
			for i in 0..bytes_in.len() {
				self.pre_nonce.push(bytes_in.as_ref()[i])
			}
		} else if self.bytes_written != 0 {
			for i in 0..bytes_in.len() {
				self.post_nonce.push(bytes_in.as_ref()[i])
			}
		}

		self.bytes_written += bytes_in.len();

		if self.bytes_written == PRE_NONCE_SIZE && self.writing_pre {
			self.writing_pre = false;
			self.bytes_written = 0;
		}

		Ok(())
	}
}

pub struct Miner {
	config: MinerConfig,
	chain: Arc<chain::Chain>,
	tx_pool: Arc<RwLock<pool::TransactionPool<PoolToChainAdapter>>>,

	// Just to hold the port we're on, so this miner can be identified
	// while watching debug output
	debug_output_id: String,
}

impl Miner {
	/// Creates a new Miner. Needs references to the chain state and its
	/// storage.
	pub fn new(
		config: MinerConfig,
		chain_ref: Arc<chain::Chain>,
		tx_pool: Arc<RwLock<pool::TransactionPool<PoolToChainAdapter>>>,
	) -> Miner {
		Miner {
			config: config,
			chain: chain_ref,
			tx_pool: tx_pool,
			debug_output_id: String::from("none"),
		}
	}

	/// Keeping this optional so setting in a separate function
	/// instead of in the new function
	pub fn set_debug_output_id(&mut self, debug_output_id: String) {
		self.debug_output_id = debug_output_id;
	}

	/// Inner part of the mining loop for cuckoo-miner async mode
	pub fn inner_loop_async(
		&self,
		plugin_miner: &mut PluginMiner,
		difficulty: Difficulty,
		b: &mut Block,
		cuckoo_size: u32,
		head: &BlockHeader,
		latest_hash: &Hash,
		attempt_time_per_block: u32,
	) -> Option<Proof> {
		debug!(
			LOGGER,
			"(Server ID: {}) Mining at Cuckoo{} for at most {} secs at height {} and difficulty {}.",
			self.debug_output_id,
			cuckoo_size,
			attempt_time_per_block,
			b.header.height,
			b.header.difficulty
		);

		// look for a pow for at most attempt_time_per_block sec on the
  // same block (to give a chance to new
  // transactions) and as long as the head hasn't changed
  // Will change this to something else at some point
		let deadline = time::get_time().sec + attempt_time_per_block as i64;

		// how often to output stats
		let stat_output_interval = 2;
		let mut next_stat_output = time::get_time().sec + stat_output_interval;

		// Get parts of the header
		let mut header_parts = HeaderPartWriter::default();
		ser::Writeable::write(&b.header, &mut header_parts).unwrap();
		let (pre, post) = header_parts.parts_as_hex_strings();

		//Just test output to mine a genesis block when needed
		/*let mut header_parts = HeaderPartWriter::default();
		let gen = genesis::genesis();
		ser::Writeable::write(&gen.header, &mut header_parts).unwrap();
		let (pre, post) = header_parts.parts_as_hex_strings();
		println!("pre, post: {}, {}", pre, post);*/

		// Start the miner working
		let miner = plugin_miner.get_consumable();
		let job_handle = miner.notify(1, &pre, &post, difficulty.into_num()).unwrap();

		let mut sol = None;

		while head.hash() == *latest_hash && time::get_time().sec < deadline {
			if let Some(s) = job_handle.get_solution() {
				sol = Some(Proof::new(s.solution_nonces.to_vec()));
				b.header.nonce = s.get_nonce_as_u64();
				// debug!(LOGGER, "Nonce: {}", b.header.nonce);
				break;
			}
			if time::get_time().sec > next_stat_output {
				let mut sps_total = 0.0;
				for i in 0..plugin_miner.loaded_plugin_count() {
					let stats = job_handle.get_stats(i);
					if let Ok(stat_vec) = stats {
						for s in stat_vec {
							let last_solution_time_secs = s.last_solution_time as f64 / 1000000000.0;
							let last_hashes_per_sec = 1.0 / last_solution_time_secs;
							debug!(
								LOGGER,
								"Mining: Plugin {} - Device {} ({}): Last Graph time: {}s; \
								 Graphs per second: {:.*} - Total Attempts: {}",
								i,
								s.device_id,
								s.device_name,
								last_solution_time_secs,
								3,
								last_hashes_per_sec,
								s.iterations_completed
							);
							if last_hashes_per_sec.is_finite() {
								sps_total += last_hashes_per_sec;
							}
						}
					}
					debug!(LOGGER, "Total solutions per second: {}", sps_total);
					next_stat_output = time::get_time().sec + stat_output_interval;
				}
			}
			// avoid busy wait
			let sleep_dur = std::time::Duration::from_millis(100);
			thread::sleep(sleep_dur);
		}
		if sol == None {
			debug!(
				LOGGER,
				"(Server ID: {}) No solution found after {} seconds, continuing...",
				self.debug_output_id,
				attempt_time_per_block
			);
		}

		job_handle.stop_jobs();
		sol
	}

	/// The inner part of mining loop for cuckoo miner sync mode
	pub fn inner_loop_sync_plugin(
		&self,
		plugin_miner: &mut PluginMiner,
		b: &mut Block,
		cuckoo_size: u32,
		head: &BlockHeader,
		attempt_time_per_block: u32,
		latest_hash: &mut Hash,
	) -> Option<Proof> {
		// look for a pow for at most attempt_time_per_block sec on the same block (to
  // give a chance to new
  // transactions) and as long as the head hasn't changed
		let deadline = time::get_time().sec + attempt_time_per_block as i64;
		let stat_check_interval = 3;
		let mut next_stat_check = time::get_time().sec + stat_check_interval;

		debug!(
			LOGGER,
			"(Server ID: {}) Mining at Cuckoo{} for {} secs (will wait for last solution) \
			 on block {} at difficulty {}.",
			self.debug_output_id,
			cuckoo_size,
			attempt_time_per_block,
			latest_hash,
			b.header.difficulty
		);
		let mut iter_count = 0;

		if self.config.slow_down_in_millis != None && self.config.slow_down_in_millis.unwrap() > 0 {
			debug!(
				LOGGER,
				"(Server ID: {}) Artificially slowing down loop by {}ms per iteration.",
				self.debug_output_id,
				self.config.slow_down_in_millis.unwrap()
			);
		}

		let mut sol = None;
		while head.hash() == *latest_hash && time::get_time().sec < deadline {
			let pow_hash = b.hash();
			if let Ok(proof) = plugin_miner.mine(&pow_hash[..]) {
				let proof_diff = proof.clone().to_difficulty();
				if proof_diff >= b.header.difficulty {
					sol = Some(proof);
					break;
				}
			}

			if time::get_time().sec >= next_stat_check {
				let stats_vec = plugin_miner.get_stats(0).unwrap();
				for s in stats_vec.into_iter() {
					let last_solution_time_secs = s.last_solution_time as f64 / 1000000000.0;
					let last_hashes_per_sec = 1.0 / last_solution_time_secs;
					debug!(
						LOGGER,
						"Plugin 0 - Device {} ({}) - Last Graph time: {}; Graphs per second: {:.*}",
						s.device_id,
						s.device_name,
						last_solution_time_secs,
						3,
						last_hashes_per_sec
					);
				}
				next_stat_check = time::get_time().sec + stat_check_interval;
			}

			b.header.nonce += 1;
			*latest_hash = self.chain.head().unwrap().last_block_h;
			iter_count += 1;

			// Artificial slow down
			if self.config.slow_down_in_millis != None
				&& self.config.slow_down_in_millis.unwrap() > 0
			{
				thread::sleep(std::time::Duration::from_millis(
					self.config.slow_down_in_millis.unwrap(),
				));
			}
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

	/// The inner part of mining loop for the internal miner
	pub fn inner_loop_sync_internal<T: MiningWorker>(
		&self,
		miner: &mut T,
		b: &mut Block,
		cuckoo_size: u32,
		head: &BlockHeader,
		attempt_time_per_block: u32,
		latest_hash: &mut Hash,
	) -> Option<Proof> {
		// look for a pow for at most 2 sec on the same block (to give a chance to new
  // transactions) and as long as the head hasn't changed
		let deadline = time::get_time().sec + attempt_time_per_block as i64;

		debug!(
			LOGGER,
			"(Server ID: {}) Mining at Cuckoo{} for at most {} secs on block {} at difficulty {}.",
			self.debug_output_id,
			cuckoo_size,
			attempt_time_per_block,
			latest_hash,
			b.header.difficulty
		);
		let mut iter_count = 0;

		if self.config.slow_down_in_millis != None && self.config.slow_down_in_millis.unwrap() > 0 {
			debug!(
				LOGGER,
				"(Server ID: {}) Artificially slowing down loop by {}ms per iteration.",
				self.debug_output_id,
				self.config.slow_down_in_millis.unwrap()
			);
		}

		let mut sol = None;
		while head.hash() == *latest_hash && time::get_time().sec < deadline {
			let pow_hash = b.hash();
			if let Ok(proof) = miner.mine(&pow_hash[..]) {
				let proof_diff = proof.clone().to_difficulty();
				if proof_diff >= b.header.difficulty {
					sol = Some(proof);
					break;
				}
			}

			b.header.nonce += 1;
			*latest_hash = self.chain.head().unwrap().last_block_h;
			iter_count += 1;

			// Artificial slow down
			if self.config.slow_down_in_millis != None
				&& self.config.slow_down_in_millis.unwrap() > 0
			{
				thread::sleep(std::time::Duration::from_millis(
					self.config.slow_down_in_millis.unwrap(),
				));
			}
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
	pub fn run_loop(&self, miner_config: MinerConfig, cuckoo_size: u32, proof_size: usize) {
		info!(
			LOGGER,
			"(Server ID: {}) Starting miner loop.",
			self.debug_output_id
		);
		let mut plugin_miner = None;
		let mut miner = None;
		if miner_config.use_cuckoo_miner {
			plugin_miner = Some(PluginMiner::new(
				consensus::EASINESS,
				cuckoo_size,
				proof_size,
			));
			plugin_miner.as_mut().unwrap().init(miner_config.clone());
		} else {
			miner = Some(cuckoo::Miner::new(
				consensus::EASINESS,
				cuckoo_size,
				proof_size,
			));
		}

		// to prevent the wallet from generating a new HD key derivation for each
  // iteration, we keep the returned derivation to provide it back when
  // nothing has changed
		let mut key_id = None;

		loop {
			debug!(LOGGER, "in miner loop...");
			trace!(LOGGER, "key_id: {:?}", key_id);

			// get the latest chain state and build a block on top of it
			let head = self.chain.head_header().unwrap();
			let mut latest_hash = self.chain.head().unwrap().last_block_h;
			let mut result = self.build_block(&head, key_id.clone());
			while let Err(e) = result {
				result = self.build_block(&head, key_id.clone());
				if let self::Error::Chain(chain::Error::DuplicateCommitment(_)) = e {
					warn!(LOGGER, "Duplicate commit for potential coinbase detected. Trying next derivation.");
				} else {
					break;
				}
			}
			let (mut b, block_fees) = result.unwrap();

			let mut sol = None;
			let mut use_async = false;
			if let Some(c) = self.config.cuckoo_miner_async_mode {
				if c {
					use_async = true;
				}
			}
			if let Some(mut p) = plugin_miner.as_mut() {
				if use_async {
					sol = self.inner_loop_async(
						&mut p,
						b.header.difficulty.clone(),
						&mut b,
						cuckoo_size,
						&head,
						&latest_hash,
						miner_config.attempt_time_per_block,
					);
				} else {
					sol = self.inner_loop_sync_plugin(
						p,
						&mut b,
						cuckoo_size,
						&head,
						miner_config.attempt_time_per_block,
						&mut latest_hash,
					);
				}
			}
			if let Some(m) = miner.as_mut() {
				sol = self.inner_loop_sync_internal(
					m,
					&mut b,
					cuckoo_size,
					&head,
					miner_config.attempt_time_per_block,
					&mut latest_hash,
				);
			}

			// if we found a solution, push our block out
			if let Some(proof) = sol {
				info!(
					LOGGER,
					"(Server ID: {}) Found valid proof of work, adding block {}.",
					self.debug_output_id,
					b.hash()
				);
				b.header.pow = proof;
				let res = self.chain.process_block(b, chain::NONE);
				if let Err(e) = res {
					error!(
						LOGGER,
						"(Server ID: {}) Error validating mined block: {:?}",
						self.debug_output_id,
						e
					);
				}
				debug!(LOGGER, "resetting key_id in miner to None");
				key_id = None;
			} else {
				debug!(
					LOGGER,
					"setting pubkey in miner to pubkey from block_fees - {:?}",
					block_fees
				);
				key_id = block_fees.key_id();
			}
		}
	}

	/// Builds a new block with the chain head as previous and eligible
	/// transactions from the pool.
	fn build_block(
		&self,
		head: &core::BlockHeader,
		key_id: Option<Identifier>,
	) -> Result<(core::Block, BlockFees), Error> {
		// prepare the block header timestamp
		let mut now_sec = time::get_time().sec;
		let head_sec = head.timestamp.to_timespec().sec;
		if now_sec == head_sec {
			now_sec += 1;
		}

		// get the difficulty our block should be at
		let diff_iter = self.chain.difficulty_iter();
		let difficulty = consensus::next_difficulty(diff_iter).unwrap();

		// extract current transaction from the pool
		let txs_box = self.tx_pool
			.read()
			.unwrap()
			.prepare_mineable_transactions(MAX_TX);
		let txs: Vec<&Transaction> = txs_box.iter().map(|tx| tx.as_ref()).collect();

		// build the coinbase and the block itself
		let fees = txs.iter().map(|tx| tx.fee).sum();
		let height = head.height + 1;
		let block_fees = BlockFees {
			fees,
			key_id,
			height,
		};

		// TODO - error handling, things can go wrong with get_coinbase (wallet api
  // down etc.)
		let (output, kernel, block_fees) = self.get_coinbase(block_fees).unwrap();
		let mut b = core::Block::with_reward(head, txs, output, kernel).unwrap();

		debug!(
			LOGGER,
			"(Server ID: {}) Built new block with {} inputs and {} outputs, difficulty: {}",
			self.debug_output_id,
			b.inputs.len(),
			b.outputs.len(),
			difficulty
		);

		// making sure we're not spending time mining a useless block
		b.validate().expect("Built an invalid block!");

		let mut rng = rand::OsRng::new().unwrap();
		b.header.nonce = rng.gen();
		b.header.difficulty = difficulty;
		b.header.timestamp = time::at_utc(time::Timespec::new(now_sec, 0));
		trace!(LOGGER, "Block: {:?}", b);
		let result=self.chain.set_sumtree_roots(&mut b);
		match result {
			Ok(_) => Ok((b, block_fees)),
			//If it's a duplicate commitment, it's likely trying to use 
			//a key that's already been derived but not in the wallet
			//for some reason, allow caller to retry
			Err(chain::Error::DuplicateCommitment(e)) =>
				Err(Error::Chain(chain::Error::DuplicateCommitment(e))),
			//Some other issue is worth a panic
			Err(e) => {
				error!(LOGGER, "Error setting sumtree root to build a block: {:?}", e);
				panic!(e);
			}
		}
	}

	///
	/// Probably only want to do this when testing.
	///
	fn burn_reward(
		&self,
		block_fees: BlockFees,
	) -> Result<(core::Output, core::TxKernel, BlockFees), Error> {
		let keychain = Keychain::from_random_seed().unwrap();
		let key_id = keychain.derive_key_id(1).unwrap();
		let (out, kernel) =
			core::Block::reward_output(&keychain, &key_id, block_fees.fees).unwrap();
		Ok((out, kernel, block_fees))
	}

	fn get_coinbase(
		&self,
		block_fees: BlockFees,
	) -> Result<(core::Output, core::TxKernel, BlockFees), Error> {
		if self.config.burn_reward {
			self.burn_reward(block_fees)
		} else {
			let url = format!(
				"{}/v1/receive/coinbase",
				self.config.wallet_listener_url.as_str()
			);

			let res = wallet::client::create_coinbase(&url, &block_fees)?;

			let out_bin = util::from_hex(res.output).unwrap();
			let kern_bin = util::from_hex(res.kernel).unwrap();
			let key_id_bin = util::from_hex(res.key_id).unwrap();
			let output = ser::deserialize(&mut &out_bin[..]).unwrap();
			let kernel = ser::deserialize(&mut &kern_bin[..]).unwrap();
			let key_id = ser::deserialize(&mut &key_id_bin[..]).unwrap();
			let block_fees = BlockFees {
				key_id: Some(key_id),
				..block_fees
			};

			debug!(LOGGER, "block_fees here: {:?}", block_fees);

			Ok((output, kernel, block_fees))
		}
	}
}
