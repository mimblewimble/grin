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

use chrono::prelude::{DateTime, NaiveDateTime, Utc};
use itertools::Itertools;
use rand::{self, Rng};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;

use chain;
use common::types::Error;
use core::core::verifier_cache::VerifierCache;
use core::ser::{self, AsFixedBytes};
use core::{consensus, core};
use keychain::{ExtKeychain, Identifier, Keychain};
use pool;
use util::{self, LOGGER};
use wallet::{self, BlockFees};

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
	tx_pool: &Arc<RwLock<pool::TransactionPool>>,
	verifier_cache: Arc<RwLock<VerifierCache>>,
	key_id: Option<Identifier>,
	wallet_listener_url: Option<String>,
) -> (core::Block, BlockFees) {
	let wallet_retry_interval = 5;
	// get the latest chain state and build a block on top of it
	let mut result = build_block(
		chain,
		tx_pool,
		verifier_cache.clone(),
		key_id.clone(),
		wallet_listener_url.clone(),
	);
	while let Err(e) = result {
		match e {
			self::Error::Chain(c) => match c.kind() {
				chain::ErrorKind::DuplicateCommitment(_) => {
					debug!(
						LOGGER,
						"Duplicate commit for potential coinbase detected. Trying next derivation."
					);
				}
				_ => {
					error!(LOGGER, "Chain Error: {}", c);
				}
			},
			self::Error::Wallet(_) => {
				error!(
					LOGGER,
					"Error building new block: Can't connect to wallet listener at {:?}; will retry",
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
			verifier_cache.clone(),
			key_id.clone(),
			wallet_listener_url.clone(),
		);
	}
	return result.unwrap();
}

/// Builds a new block with the chain head as previous and eligible
/// transactions from the pool.
fn build_block(
	chain: &Arc<chain::Chain>,
	tx_pool: &Arc<RwLock<pool::TransactionPool>>,
	verifier_cache: Arc<RwLock<VerifierCache>>,
	key_id: Option<Identifier>,
	wallet_listener_url: Option<String>,
) -> Result<(core::Block, BlockFees), Error> {
	// prepare the block header timestamp
	let head = chain.head_header()?;

	let mut now_sec = Utc::now().timestamp();
	let head_sec = head.timestamp.timestamp();
	if now_sec <= head_sec {
		now_sec = head_sec + 1;
	}

	// get the difficulty our block should be at
	let diff_iter = chain.difficulty_iter();
	let difficulty = consensus::next_difficulty(diff_iter).unwrap();

	// extract current transaction from the pool
	let txs = tx_pool.read().unwrap().prepare_mineable_transactions();

	// build the coinbase and the block itself
	let fees = txs.iter().map(|tx| tx.fee()).sum();
	let height = head.height + 1;
	let block_fees = BlockFees {
		fees,
		key_id,
		height,
	};

	let (output, kernel, block_fees) = get_coinbase(wallet_listener_url, block_fees)?;
	let mut b = core::Block::with_reward(
		&head,
		txs,
		output,
		kernel,
		difficulty.clone(),
		verifier_cache.clone(),
	)?;

	// making sure we're not spending time mining a useless block
	b.validate(
		&head.total_kernel_offset,
		&head.total_kernel_sum,
		verifier_cache,
	)?;

	let mut rng = rand::OsRng::new().unwrap();
	b.header.nonce = rng.gen();
	b.header.timestamp = DateTime::<Utc>::from_utc(NaiveDateTime::from_timestamp(now_sec, 0), Utc);;

	let b_difficulty = (b.header.total_difficulty.clone() - head.total_difficulty.clone()).to_num();
	debug!(
		LOGGER,
		"Built new block with {} inputs and {} outputs, network difficulty: {}, cumulative difficulty {}",
		b.inputs().len(),
		b.outputs().len(),
		b_difficulty,
		b.header.clone().total_difficulty.to_num(),
	);

	// Now set txhashset roots and sizes on the header of the block being built.
	let roots_result = chain.set_txhashset_roots(&mut b, false);

	match roots_result {
		Ok(_) => Ok((b, block_fees)),

		// If it's a duplicate commitment, it's likely trying to use
		// a key that's already been derived but not in the wallet
		// for some reason, allow caller to retry
		Err(e) => {
			match e.kind() {
				chain::ErrorKind::DuplicateCommitment(e) => Err(Error::Chain(
					chain::ErrorKind::DuplicateCommitment(e).into(),
				)),

				//Some other issue, possibly duplicate kernel
				_ => {
					error!(
						LOGGER,
						"Error setting txhashset root to build a block: {:?}", e
					);
					Err(Error::Chain(
						chain::ErrorKind::Other(format!("{:?}", e)).into(),
					))
				}
			}
		}
	}
}

///
/// Probably only want to do this when testing.
///
fn burn_reward(block_fees: BlockFees) -> Result<(core::Output, core::TxKernel, BlockFees), Error> {
	warn!(LOGGER, "Burning block fees: {:?}", block_fees);
	let keychain = ExtKeychain::from_random_seed().unwrap();
	let key_id = keychain.derive_key_id(1).unwrap();
	let (out, kernel) =
		wallet::libtx::reward::output(&keychain, &key_id, block_fees.fees, block_fees.height)
			.unwrap();
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
			let res = wallet::create_coinbase(&wallet_listener_url, &block_fees)?;
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
