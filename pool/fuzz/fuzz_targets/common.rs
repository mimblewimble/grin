// Copyright 2021 The Grin Developers
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

//! Common test functions

use self::chain::types::{NoopAdapter, Options};
use self::chain::Chain;
use self::core::consensus;
use self::core::core::hash::Hash;
use self::core::core::verifier_cache::{LruVerifierCache, VerifierCache};
use self::core::core::{Block, BlockHeader, BlockSums, KernelFeatures, Transaction};
use self::core::genesis;
use self::core::global;
use self::core::libtx::{build, reward, ProofBuilder, DEFAULT_BASE_FEE};
use self::core::pow;
use self::keychain::{ExtKeychain, ExtKeychainPath, Keychain};
use self::pool::types::*;
use self::pool::TransactionPool;
use self::util::RwLock;
use chrono::Duration;
use grin_chain as chain;
use grin_core as core;
use grin_keychain as keychain;
use grin_pool as pool;
use grin_util as util;
use std::fs;
use std::sync::Arc;

/// Build genesis block with reward (non-empty, like we have in mainnet).
// Same as from pool/tests/common.rs
pub fn genesis_block<K>(keychain: &K) -> Block
where
	K: Keychain,
{
	let key_id = keychain::ExtKeychain::derive_key_id(1, 0, 0, 0, 0);
	let reward = reward::output(keychain, &ProofBuilder::new(keychain), &key_id, 0, false).unwrap();

	genesis::genesis_dev().with_reward(reward.0, reward.1)
}

pub fn fuzz_tx_source(nonce: u8) -> TxSource {
	if nonce < 51 {
		TxSource::PushApi
	} else if nonce < 102 {
		TxSource::Broadcast
	} else if nonce < 153 {
		TxSource::Fluff
	} else if nonce < 204 {
		TxSource::EmbargoExpired
	} else {
		TxSource::Deaggregate
	}
}

// Same as from pool/tests/common.rs
#[derive(Clone)]
pub struct ChainAdapter {
	pub chain: Arc<Chain>,
}

// Same as from pool/tests/common.rs
impl BlockChain for ChainAdapter {
	fn chain_head(&self) -> Result<BlockHeader, PoolError> {
		self.chain
			.head_header()
			.map_err(|_| PoolError::Other("failed to get chain head".into()))
	}

	fn get_block_header(&self, hash: &Hash) -> Result<BlockHeader, PoolError> {
		self.chain
			.get_block_header(hash)
			.map_err(|_| PoolError::Other("failed to get block header".into()))
	}

	fn get_block_sums(&self, hash: &Hash) -> Result<BlockSums, PoolError> {
		self.chain
			.get_block_sums(hash)
			.map_err(|_| PoolError::Other("failed to get block sums".into()))
	}

	fn validate_tx(&self, tx: &Transaction) -> Result<(), pool::PoolError> {
		self.chain.validate_tx(tx).map_err(|e| match e.kind() {
			chain::Error::Transaction(txe) => txe.into(),
			chain::Error::NRDRelativeHeight => PoolError::NRDKernelRelativeHeight,
			_ => PoolError::Other("failed to validate tx".into()),
		})
	}

	fn verify_coinbase_maturity(&self, tx: &Transaction) -> Result<(), PoolError> {
		self.chain
			.verify_coinbase_maturity(tx)
			.map_err(|_| PoolError::ImmatureCoinbase)
	}

	fn verify_tx_lock_height(&self, tx: &Transaction) -> Result<(), PoolError> {
		self.chain
			.verify_tx_lock_height(tx)
			.map_err(|_| PoolError::ImmatureTransaction)
	}
}

// Same as from pool/tests/common.rs
pub fn clean_output_dir(db_root: String) {
	if let Err(e) = fs::remove_dir_all(db_root) {
		println!("cleaning output dir failed - {:?}", e)
	}
}

pub struct PoolFuzzer {
	pub chain: Arc<Chain>,
	pub keychain: ExtKeychain,
	pub pool: TransactionPool<ChainAdapter, NoopPoolAdapter, LruVerifierCache>,
}

impl PoolFuzzer {
	pub fn new(db_root: &str) -> Self {
		let keychain: ExtKeychain = Keychain::from_random_seed(false).unwrap();

		clean_output_dir(db_root.into());

		let genesis = genesis_block(&keychain);
		let chain = Arc::new(Self::init_chain(db_root, genesis));
		let verifier_cache = Arc::new(RwLock::new(LruVerifierCache::new()));

		// Initialize a new pool with our chain adapter.
		let pool = Self::init_transaction_pool(
			Arc::new(ChainAdapter {
				chain: chain.clone(),
			}),
			verifier_cache.clone(),
		);

		let ret = Self {
			chain,
			keychain,
			pool,
		};

		ret.add_some_blocks(3);

		ret
	}

	// Same as from pool/tests/common.rs, with interface change
	pub fn test_transaction_spending_coinbase(&self, output_values: Vec<u64>) -> Transaction {
		let header = self.chain.get_header_by_height(1).unwrap();

		let mut output_sum = 0u64;
		for &s in output_values.iter() {
			output_sum = output_sum.overflowing_add(s).0;
		}

		let coinbase_reward: u64 = 60_000_000_000;

		let fees = coinbase_reward.overflowing_sub(output_sum).0;

		let mut tx_elements = Vec::new();

		// single input spending a single coinbase (deterministic key_id aka height)
		{
			let key_id = ExtKeychain::derive_key_id(1, header.height as u32, 0, 0, 0);
			tx_elements.push(build::coinbase_input(coinbase_reward, key_id));
		}

		for output_value in output_values {
			let key_id = ExtKeychain::derive_key_id(1, output_value as u32, 0, 0, 0);
			tx_elements.push(build::output(output_value, key_id));
		}

		build::transaction(
			KernelFeatures::Plain { fee: fees },
			tx_elements,
			&self.keychain,
			&ProofBuilder::new(&self.keychain),
		)
		.unwrap()
	}

	// Same as from pool/tests/common.rs,
	//   with changes for summing inputs and outputs
	pub fn test_transaction(&self, input_values: Vec<u64>, output_values: Vec<u64>) -> Transaction {
		let mut input_sum = 0u64;
		for &s in input_values.iter() {
			input_sum = input_sum.overflowing_add(s).0;
		}

		let mut output_sum = 0u64;
		for &s in output_values.iter() {
			output_sum = output_sum.overflowing_add(s).0;
		}

		let fees = input_sum.overflowing_sub(output_sum).0;

		self.test_transaction_with_kernel_features(
			input_values,
			output_values,
			KernelFeatures::Plain { fee: fees },
		)
	}

	pub fn test_transaction_with_kernel_features(
		&self,
		input_values: Vec<u64>,
		output_values: Vec<u64>,
		kernel_features: KernelFeatures,
	) -> Transaction {
		let mut tx_elements = Vec::new();

		for input_value in input_values {
			let key_id = ExtKeychain::derive_key_id(1, input_value as u32, 0, 0, 0);
			tx_elements.push(build::input(input_value, key_id));
		}

		for output_value in output_values {
			let key_id = ExtKeychain::derive_key_id(1, output_value as u32, 0, 0, 0);
			tx_elements.push(build::output(output_value, key_id));
		}

		let keychain = &self.keychain;

		build::transaction(
			kernel_features,
			tx_elements,
			keychain,
			&ProofBuilder::new(keychain),
		)
		.unwrap()
	}

	fn init_chain(dir_name: &str, genesis: Block) -> Chain {
		let verifier_cache = Arc::new(RwLock::new(LruVerifierCache::new()));
		Chain::init(
			dir_name.to_string(),
			Arc::new(NoopAdapter {}),
			genesis,
			pow::verify_size,
			verifier_cache,
			false,
		)
		.unwrap()
	}

	// Same as from pool/tests/common.rs
	fn init_transaction_pool<B, V>(
		chain: Arc<B>,
		verifier_cache: Arc<RwLock<V>>,
	) -> TransactionPool<B, NoopPoolAdapter, V>
	where
		B: BlockChain,
		V: VerifierCache + 'static,
	{
		TransactionPool::new(
			PoolConfig {
				accept_fee_base: DEFAULT_BASE_FEE,
				max_pool_size: 50,
				max_stempool_size: 50,
				mineable_max_weight: 10_000,
			},
			chain.clone(),
			verifier_cache.clone(),
			Arc::new(NoopPoolAdapter {}),
		)
	}

	// Same as from pool/tests/common.rs, with interface change
	pub fn add_some_blocks(&self, count: u64) {
		for _ in 0..count {
			self.add_block(vec![]);
		}
	}

	// Same as from pool/tests/common.rs, with interface change
	pub fn add_block(&self, txs: Vec<Transaction>) {
		let chain = &self.chain;
		let keychain = &self.keychain;

		let prev = chain.head_header().unwrap();
		let height = prev.height + 1;
		let next_header_info = consensus::next_difficulty(height, chain.difficulty_iter().unwrap());
		let fee = txs.iter().map(|x| x.fee()).sum();
		let key_id = ExtKeychainPath::new(1, height as u32, 0, 0, 0).to_identifier();
		let reward =
			reward::output(keychain, &ProofBuilder::new(keychain), &key_id, fee, false).unwrap();

		let mut block =
			Block::new(&prev, txs, next_header_info.clone().difficulty, reward).unwrap();

		block.header.timestamp = prev.timestamp + Duration::seconds(60);
		block.header.pow.secondary_scaling = next_header_info.secondary_scaling;

		chain.set_txhashset_roots(&mut block).unwrap();

		let edge_bits = global::min_edge_bits();
		block.header.pow.proof.edge_bits = edge_bits;
		pow::pow_size(
			&mut block.header,
			next_header_info.difficulty,
			global::proofsize(),
			edge_bits,
		)
		.unwrap();

		chain.process_block(block, Options::NONE).unwrap();
	}
}
