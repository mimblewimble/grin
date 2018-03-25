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

//! Mining service, gathers transactions from the pool, assemble them in a
//! block and mine the block to produce a valid header with its proof-of-work.

use rand::{self, Rng};
use std::sync::{Arc, RwLock};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;
use time;

use adapters::PoolToChainAdapter;
use core::consensus;
use core::core;
use core::core::Proof;
use core::core::{Block, BlockHeader, Transaction};
use core::core::hash::{Hash, Hashed};
use pow::{cuckoo, MiningWorker};
use pow::types::MinerConfig;
use core::ser;
use core::global;
use core::ser::AsFixedBytes;
use util::LOGGER;
use types::Error;
use stats::MiningStats;

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

/// Serializer that outputs the pre-pow part of the header,
/// including the nonce (last 8 bytes) that can be sent off
/// to the miner to mutate at will
pub struct HeaderPrePowWriter {
	pub pre_pow: Vec<u8>,
}

impl Default for HeaderPrePowWriter {
	fn default() -> HeaderPrePowWriter {
		HeaderPrePowWriter {
			pre_pow: Vec::new(),
		}
	}
}

impl HeaderPrePowWriter {
	pub fn as_hex_string(&self, include_nonce: bool) -> String {
		let mut result = String::from(format!("{:02x}", self.pre_pow.iter().format("")));
		if !include_nonce {
			let l = result.len() - 16;
			result.truncate(l);
		}
		result
	}
}

impl ser::Writer for HeaderPrePowWriter {
	fn serialization_mode(&self) -> ser::SerializationMode {
		ser::SerializationMode::Full
	}

	fn write_fixed_bytes<T: AsFixedBytes>(&mut self, bytes_in: &T) -> Result<(), ser::Error> {
		for i in 0..bytes_in.len() {
			self.pre_pow.push(bytes_in.as_ref()[i])
		}
		Ok(())
	}
}

pub struct Miner {
	config: MinerConfig,
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
		config: MinerConfig,
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

	/// Inner part of the mining loop for cuckoo-miner async mode
	pub fn inner_loop_async(
		&self,
		plugin_miner: &mut PluginMiner,
		b: &mut Block,
		cuckoo_size: u32,
		head: &BlockHeader,
		latest_hash: &Hash,
		attempt_time_per_block: u32,
		mining_stats: Arc<RwLock<MiningStats>>,
	) -> Option<Proof> {
		debug!(
			LOGGER,
			"(Server ID: {}) Mining Cuckoo{} for max {}s on {} @ {} [{}].",
			self.debug_output_id,
			cuckoo_size,
			attempt_time_per_block,
			b.header.total_difficulty,
			b.header.height,
			b.header.hash()
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
		let mut pre_pow_writer = HeaderPrePowWriter::default();
		b.header.write_pre_pow(&mut pre_pow_writer).unwrap();
		let pre_pow = pre_pow_writer.as_hex_string(false);

		// Start the miner working
		let miner = plugin_miner.get_consumable();
		let job_handle = miner.notify(1, &pre_pow, "", 0).unwrap();

		let mut sol = None;

		while head.hash() == *latest_hash && time::get_time().sec < deadline {
			if let Some(s) = job_handle.get_solution() {
				let proof = Proof::new(s.solution_nonces.to_vec());
				let proof_diff = proof.clone().to_difficulty();
				trace!(
					LOGGER,
					"Found cuckoo solution! nonce {} gave difficulty {} (block diff {})",
					s.get_nonce_as_u64(),
					proof_diff.into_num(),
					(b.header.total_difficulty.clone() - head.total_difficulty.clone()).into_num()
				);
				if proof_diff > (b.header.total_difficulty.clone() - head.total_difficulty.clone())
				{
					sol = Some(proof);
					b.header.nonce = s.get_nonce_as_u64();
					break;
				}
			}
			if time::get_time().sec > next_stat_output {
				let mut sps_total = 0.0;
				for i in 0..plugin_miner.loaded_plugin_count() {
					let stats = job_handle.get_stats(i);
					if let Ok(stat_vec) = stats {
						for s in stat_vec {
							if s.in_use == 0 {
								continue;
							}
							let last_solution_time_secs =
								s.last_solution_time as f64 / 1000000000.0;
							let last_hashes_per_sec = 1.0 / last_solution_time_secs;
							let status = match s.has_errored {
								0 => "OK",
								_ => "ERRORED",
							};
							debug!(
								LOGGER,
								"Mining: Plugin {} - Device {} ({}) Status: {} : Last Graph time: {}s; \
								 Graphs per second: {:.*} - Total Attempts: {}",
								i,
								s.device_id,
								s.device_name,
								status,
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
				}
				info!(LOGGER, "Mining at {} graphs per second", sps_total);
				if sps_total.is_finite() {
					let mut mining_stats = mining_stats.write().unwrap();
					mining_stats.combined_gps = sps_total;
					let mut device_vec = vec![];
					for i in 0..plugin_miner.loaded_plugin_count() {
						device_vec.push(job_handle.get_stats(i).unwrap());
					}
					mining_stats.device_stats = Some(device_vec);
				}
				next_stat_output = time::get_time().sec + stat_output_interval;
			}
			// avoid busy wait
			thread::sleep(Duration::from_millis(100));
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
		mining_stats: Arc<RwLock<MiningStats>>,
	) -> Option<Proof> {
		// look for a pow for at most attempt_time_per_block sec on the same block (to
		// give a chance to new
		// transactions) and as long as the head hasn't changed
		let deadline = time::get_time().sec + attempt_time_per_block as i64;
		let stat_check_interval = 3;
		let mut next_stat_check = time::get_time().sec + stat_check_interval;

		debug!(
			LOGGER,
			"(Server ID: {}) Mining Cuckoo{} for max {}s (will wait for last solution) \
			 on {} @ {} [{}].",
			self.debug_output_id,
			cuckoo_size,
			attempt_time_per_block,
			b.header.total_difficulty,
			b.header.height,
			latest_hash
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
			let pow_hash = b.header.pre_pow_hash();
			if let Ok(proof) = plugin_miner.mine(&pow_hash[..]) {
				let proof_diff = proof.clone().to_difficulty();
				trace!(
					LOGGER,
					"Found cuckoo solution for nonce {} of difficulty {} (cumulative diff {})",
					b.header.nonce,
					proof_diff.into_num(),
					b.header.total_difficulty.into_num()
				);
				if proof_diff > (b.header.total_difficulty.clone() - head.total_difficulty.clone())
				{
					sol = Some(proof);
					break;
				}
			}

			if time::get_time().sec >= next_stat_check {
				let stats_vec = plugin_miner.get_stats(0).unwrap();
				for s in stats_vec.into_iter() {
					if s.in_use == 0 {
						continue;
					}
					let last_solution_time_secs = s.last_solution_time as f64 / 1000000000.0;
					let last_hashes_per_sec = 1.0 / last_solution_time_secs;
					let status = match s.has_errored {
						0 => "OK",
						_ => "ERRORED",
					};
					debug!(
						LOGGER,
						"Plugin 0 - Device {} ({}) Status: {} - Last Graph time: {}; Graphs per second: {:.*}",
						s.device_id,
						s.device_name,
						status,
						last_solution_time_secs,
						3,
						last_hashes_per_sec
					);
					info!(
						LOGGER,
						"Mining at {} graphs per second", last_hashes_per_sec
					);
					if last_hashes_per_sec.is_finite() {
						let mut mining_stats = mining_stats.write().unwrap();
						mining_stats.combined_gps = last_hashes_per_sec;
						let mut device_vec = vec![];
						device_vec.push(plugin_miner.get_stats(0).unwrap());
						mining_stats.device_stats = Some(device_vec);
					}
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
				thread::sleep(Duration::from_millis(
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
	/// kept around mostly for automated testing purposes
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
			"(Server ID: {}) Mining Cuckoo{} for max {}s on {} @ {} [{}].",
			self.debug_output_id,
			cuckoo_size,
			attempt_time_per_block,
			b.header.total_difficulty,
			b.header.height,
			latest_hash
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
			let pow_hash = b.header.pre_pow_hash();
			if let Ok(proof) = miner.mine(&pow_hash[..]) {
				let proof_diff = proof.clone().to_difficulty();
				if proof_diff > (b.header.total_difficulty.clone() - head.total_difficulty.clone())
				{
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
				thread::sleep(Duration::from_millis(
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
	pub fn run_loop(
		&self,
		miner_config: MinerConfig,
		mining_stats: Arc<RwLock<MiningStats>>,
		cuckoo_size: u32,
		proof_size: usize,
	) {
		info!(
			LOGGER,
			"(Server ID: {}) Starting miner loop.", self.debug_output_id
		);

		let mut plugin_miner = None;
		let mut miner = None;
		if !global::is_automated_testing_mode() {
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

		// iteration, we keep the returned derivation to provide it back when
		// nothing has changed
		let mut key_id = None;

		{
			let mut mining_stats = mining_stats.write().unwrap();
			mining_stats.is_mining = true;
			mining_stats.cuckoo_size = cuckoo_size as u16;
		}

		loop {
			trace!(LOGGER, "in miner loop. key_id: {:?}", key_id);

			// get the latest chain state and build a block on top of it
			let head = self.chain.head_header().unwrap();
			let mut latest_hash = self.chain.head().unwrap().last_block_h;

			let mut result = self.build_block(&head, key_id.clone());
			while let Err(e) = result {
				match e {
					self::Error::Chain(chain::Error::DuplicateCommitment(_)) => {
						debug!(LOGGER, "Duplicate commit for potential coinbase detected. Trying next derivation.");
					}
					ae => {
						warn!(LOGGER, "Error building new block: {:?}. Retrying.", ae);
					}
				}
				thread::sleep(Duration::from_millis(100));
				result = self.build_block(&head, key_id.clone());
			}

			let (mut b, block_fees) = result.unwrap();
			{
				let mut mining_stats = mining_stats.write().unwrap();
				mining_stats.block_height = b.header.height;
				mining_stats.network_difficulty =
					(b.header.total_difficulty.clone() - head.total_difficulty.clone()).into_num();
			}

			let mut sol = None;
			let mut use_async = false;
			if let Some(c) = self.config.miner_async_mode {
				if c {
					use_async = true;
				}
			}
			if let Some(mut p) = plugin_miner.as_mut() {
				if use_async {
					sol = self.inner_loop_async(
						&mut p,
						&mut b,
						cuckoo_size,
						&head,
						&latest_hash,
						miner_config.attempt_time_per_block,
						mining_stats.clone(),
					);
				} else {
					sol = self.inner_loop_sync_plugin(
						p,
						&mut b,
						cuckoo_size,
						&head,
						miner_config.attempt_time_per_block,
						&mut latest_hash,
						mining_stats.clone(),
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
		let fees = txs.iter().map(|tx| tx.fee()).sum();
		let height = head.height + 1;
		let block_fees = BlockFees {
			fees,
			key_id,
			height,
		};

		let (output, kernel, block_fees) = self.get_coinbase(block_fees)?;
		let mut b = core::Block::with_reward(head, txs, output, kernel, difficulty.clone())?;

		debug!(
			LOGGER,
			"(Server ID: {}) Built new block with {} inputs and {} outputs, network difficulty: {}, cumulative difficulty {}",
			self.debug_output_id,
			b.inputs.len(),
			b.outputs.len(),
			difficulty.clone().into_num(),
			b.header.clone().total_difficulty.clone().into_num(),
		);

		// making sure we're not spending time mining a useless block
		b.validate(&head)?;

		let mut rng = rand::OsRng::new().unwrap();
		b.header.nonce = rng.gen();
		b.header.timestamp = time::at_utc(time::Timespec::new(now_sec, 0));

		let roots_result = self.chain.set_txhashset_roots(&mut b, false);

		match roots_result {
			Ok(_) => Ok((b, block_fees)),

			// If it's a duplicate commitment, it's likely trying to use
			// a key that's already been derived but not in the wallet
			// for some reason, allow caller to retry
			Err(chain::Error::DuplicateCommitment(e)) => {
				Err(Error::Chain(chain::Error::DuplicateCommitment(e)))
			}

			//Some other issue, possibly duplicate kernel
			Err(e) => {
				error!(
					LOGGER,
					"Error setting txhashset root to build a block: {:?}", e
				);
				Err(Error::Chain(chain::Error::Other(format!("{:?}", e))))
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
			core::Block::reward_output(&keychain, &key_id, block_fees.fees, block_fees.height)
				.unwrap();
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

			debug!(LOGGER, "get_coinbase: {:?}", block_fees);

			Ok((output, kernel, block_fees))
		}
	}
}
