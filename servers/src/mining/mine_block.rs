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

//! Build a block to mine: gathers transactions from the pool, assembles
//! them into a block and returns it.

use std::thread;
use std::sync::{Arc, RwLock};
use time;
use std::time::Duration;
use rand::{self, Rng};
use itertools::Itertools;

use core::ser::AsFixedBytes;
use chain;
use chain::types::BlockSums;
use pool;
use core::consensus;
use core::core;
use core::core::Transaction;
use core::core::hash::Hashed;
use core::ser;
use keychain::{Identifier, Keychain};
use wallet;
use wallet::BlockFees;
use util;
use util::LOGGER;
use common::types::Error;
use common::adapters::PoolToChainAdapter;

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

// Ensure a block suitable for mining is built and returned
// If a wallet listener URL is not provided the reward will be "burnt"
// Warning: This call does not return until/unless a new block can be built
pub fn get_block(
	chain: &Arc<chain::Chain>,
	tx_pool: &Arc<RwLock<pool::TransactionPool<PoolToChainAdapter>>>,
	key_id: Option<Identifier>,
	max_tx: u32,
	wallet_listener_url: Option<String>,
) -> (core::Block, BlockFees) {
	let wallet_retry_interval = 5;
	// get the latest chain state and build a block on top of it
	let mut result = build_block(
		chain,
		tx_pool,
		key_id.clone(),
		max_tx,
		wallet_listener_url.clone(),
	);
	while let Err(e) = result {
		match e {
			self::Error::Chain(chain::Error::DuplicateCommitment(_)) => {
				debug!(
					LOGGER,
					"Duplicate commit for potential coinbase detected. Trying next derivation."
				);
			}
			self::Error::Wallet(_) => {
				error!(
					LOGGER,
					"Stratum server: Can't connect to wallet listener at {:?}; will retry",
					wallet_listener_url.as_ref().unwrap()
				);
				thread::sleep(Duration::from_secs(wallet_retry_interval));
			}
			ae => {
				warn!(LOGGER, "Error building new block: {:?}. Retrying.", ae);
			}
		}
		thread::sleep(Duration::from_millis(100));
		result = build_block(
			chain,
			tx_pool,
			key_id.clone(),
			max_tx,
			wallet_listener_url.clone(),
		);
	}
	return result.unwrap();
}

/// Builds a new block with the chain head as previous and eligible
/// transactions from the pool.
fn build_block(
	chain: &Arc<chain::Chain>,
	tx_pool: &Arc<RwLock<pool::TransactionPool<PoolToChainAdapter>>>,
	key_id: Option<Identifier>,
	max_tx: u32,
	wallet_listener_url: Option<String>,
) -> Result<(core::Block, BlockFees), Error> {
	// prepare the block header timestamp
	let head = chain.head_header()?;

	let prev_sums = if head.height == 0 {
		BlockSums::default()
	} else {
		chain.get_block_sums(&head.hash())?
	};

	let mut now_sec = time::get_time().sec;
	let head_sec = head.timestamp.to_timespec().sec;
	if now_sec <= head_sec {
		now_sec = head_sec + 1;
	}

	// get the difficulty our block should be at
	let diff_iter = chain.difficulty_iter();
	let difficulty = consensus::next_difficulty(diff_iter).unwrap();

	// extract current transaction from the pool
	let txs_box = tx_pool
		.read()
		.unwrap()
		.prepare_mineable_transactions(max_tx);
	let txs: Vec<&Transaction> = txs_box.iter().map(|tx| tx.as_ref()).collect();

	// build the coinbase and the block itself
	let fees = txs.iter().map(|tx| tx.fee()).sum();
	let height = head.height + 1;
	let block_fees = BlockFees {
		fees,
		key_id,
		height,
	};

	let (output, kernel, block_fees) = get_coinbase(wallet_listener_url, block_fees)?;
	let mut b = core::Block::with_reward(&head, txs, output, kernel, difficulty.clone())?;

	// making sure we're not spending time mining a useless block
	b.validate(&prev_sums.output_sum, &prev_sums.kernel_sum)?;

	let mut rng = rand::OsRng::new().unwrap();
	b.header.nonce = rng.gen();
	b.header.timestamp = time::at_utc(time::Timespec::new(now_sec, 0));

	let b_difficulty =
		(b.header.total_difficulty.clone() - head.total_difficulty.clone()).into_num();
	debug!(
		LOGGER,
		"Built new block with {} inputs and {} outputs, network difficulty: {}, cumulative difficulty {}",
		b.inputs.len(),
		b.outputs.len(),
		b_difficulty,
		b.header.clone().total_difficulty.clone().into_num(),
	);

	let roots_result = chain.set_txhashset_roots(&mut b, false);

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
fn burn_reward(block_fees: BlockFees) -> Result<(core::Output, core::TxKernel, BlockFees), Error> {
	warn!(LOGGER, "Burning block fees: {:?}", block_fees);
	let keychain = Keychain::from_random_seed().unwrap();
	let key_id = keychain.derive_key_id(1).unwrap();
	let (out, kernel) =
		core::Block::reward_output(&keychain, &key_id, block_fees.fees, block_fees.height).unwrap();
	Ok((out, kernel, block_fees))
}

// Connect to the wallet listener and get coinbase.
// Warning: If a wallet listener URL is not provided the reward will be "burnt"
fn get_coinbase(
	wallet_listener_url: Option<String>,
	block_fees: BlockFees,
) -> Result<(core::Output, core::TxKernel, BlockFees), Error> {
	match wallet_listener_url {
		None => {
			// Burn it
			return burn_reward(block_fees);
		}
		Some(wallet_listener_url) => {
			// Get the wallet coinbase
			let url = format!("{}/v1/receive/coinbase", wallet_listener_url.as_str());

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

			return Ok((output, kernel, block_fees));
		}
	}
}
