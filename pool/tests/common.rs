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
use self::core::core::{
	Block, BlockHeader, BlockSums, Inputs, KernelFeatures, OutputIdentifier, Transaction, TxKernel,
};
use self::core::genesis;
use self::core::global;
use self::core::libtx::{build, reward, ProofBuilder};
use self::core::pow;
use self::keychain::{BlindingFactor, ExtKeychain, ExtKeychainPath, Keychain};
use self::pool::types::*;
use self::pool::TransactionPool;
use chrono::Duration;
use grin_chain as chain;
use grin_core as core;
use grin_keychain as keychain;
use grin_pool as pool;
use std::convert::TryInto;
use std::fs;
use std::sync::Arc;

/// Build genesis block with reward (non-empty, like we have in mainnet).
pub fn genesis_block<K>(keychain: &K) -> Block
where
	K: Keychain,
{
	let key_id = keychain::ExtKeychain::derive_key_id(1, 0, 0, 0, 0);
	let reward = reward::output(keychain, &ProofBuilder::new(keychain), &key_id, 0, false).unwrap();

	genesis::genesis_dev().with_reward(reward.0, reward.1)
}

pub fn init_chain(dir_name: &str, genesis: Block) -> Chain {
	Chain::init(
		dir_name.to_string(),
		Arc::new(NoopAdapter {}),
		genesis,
		pow::verify_size,
		false,
	)
	.unwrap()
}

pub fn add_some_blocks<K>(chain: &Chain, count: u64, keychain: &K)
where
	K: Keychain,
{
	for _ in 0..count {
		add_block(chain, &[], keychain);
	}
}

pub fn add_block<K>(chain: &Chain, txs: &[Transaction], keychain: &K)
where
	K: Keychain,
{
	let prev = chain.head_header().unwrap();
	let height = prev.height + 1;
	let next_header_info = consensus::next_difficulty(height, chain.difficulty_iter().unwrap());
	let fee = txs.iter().map(|x| x.fee()).sum();
	let key_id = ExtKeychainPath::new(1, height as u32, 0, 0, 0).to_identifier();
	let reward =
		reward::output(keychain, &ProofBuilder::new(keychain), &key_id, fee, false).unwrap();

	let mut block = Block::new(&prev, txs, next_header_info.clone().difficulty, reward).unwrap();

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

#[derive(Clone)]
pub struct ChainAdapter {
	pub chain: Arc<Chain>,
}

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
		self.chain.validate_tx(tx).map_err(|e| match e {
			chain::Error::Transaction { source: txe } => txe.into(),
			chain::Error::NRDRelativeHeight => PoolError::NRDKernelRelativeHeight,
			_ => PoolError::Other("failed to validate tx".into()),
		})
	}

	fn validate_inputs(&self, inputs: &Inputs) -> Result<Vec<OutputIdentifier>, PoolError> {
		self.chain
			.validate_inputs(inputs)
			.map(|outputs| outputs.into_iter().map(|(out, _)| out).collect::<Vec<_>>())
			.map_err(|_| PoolError::Other("failed to validate inputs".into()))
	}

	fn verify_coinbase_maturity(&self, inputs: &Inputs) -> Result<(), PoolError> {
		self.chain
			.verify_coinbase_maturity(inputs)
			.map_err(|_| PoolError::ImmatureCoinbase)
	}

	fn verify_tx_lock_height(&self, tx: &Transaction) -> Result<(), PoolError> {
		self.chain
			.verify_tx_lock_height(tx)
			.map_err(|_| PoolError::ImmatureTransaction)
	}
}

pub fn init_transaction_pool<B>(chain: Arc<B>) -> TransactionPool<B, NoopPoolAdapter>
where
	B: BlockChain,
{
	TransactionPool::new(
		PoolConfig {
			accept_fee_base: default_accept_fee_base(),
			reorg_cache_period: 30,
			max_pool_size: 50,
			max_stempool_size: 50,
			mineable_max_weight: 10_000,
		},
		chain.clone(),
		Arc::new(NoopPoolAdapter {}),
	)
}

pub fn test_transaction_spending_coinbase<K>(
	keychain: &K,
	header: &BlockHeader,
	output_values: Vec<u64>,
) -> Transaction
where
	K: Keychain,
{
	let output_sum = output_values.iter().sum::<u64>() as i64;

	let coinbase_reward: u64 = 60_000_000_000;

	let fees: i64 = coinbase_reward as i64 - output_sum;
	assert!(fees >= 0);

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
		KernelFeatures::Plain {
			fee: (fees as u64).try_into().unwrap(),
		},
		&tx_elements,
		keychain,
		&ProofBuilder::new(keychain),
	)
	.unwrap()
}

pub fn test_transaction<K>(
	keychain: &K,
	input_values: Vec<u64>,
	output_values: Vec<u64>,
) -> Transaction
where
	K: Keychain,
{
	let input_sum = input_values.iter().sum::<u64>() as i64;
	let output_sum = output_values.iter().sum::<u64>() as i64;
	let fees: i64 = input_sum - output_sum;
	assert!(fees >= 0);

	test_transaction_with_kernel_features(
		keychain,
		input_values,
		output_values,
		KernelFeatures::Plain {
			fee: (fees as u64).try_into().unwrap(),
		},
	)
}

pub fn test_transaction_with_kernel_features<K>(
	keychain: &K,
	input_values: Vec<u64>,
	output_values: Vec<u64>,
	kernel_features: KernelFeatures,
) -> Transaction
where
	K: Keychain,
{
	let mut tx_elements = Vec::new();

	for input_value in input_values {
		let key_id = ExtKeychain::derive_key_id(1, input_value as u32, 0, 0, 0);
		tx_elements.push(build::input(input_value, key_id));
	}

	for output_value in output_values {
		let key_id = ExtKeychain::derive_key_id(1, output_value as u32, 0, 0, 0);
		tx_elements.push(build::output(output_value, key_id));
	}

	build::transaction(
		kernel_features,
		&tx_elements,
		keychain,
		&ProofBuilder::new(keychain),
	)
	.unwrap()
}

pub fn test_transaction_with_kernel<K>(
	keychain: &K,
	input_values: Vec<u64>,
	output_values: Vec<u64>,
	kernel: TxKernel,
	excess: BlindingFactor,
) -> Transaction
where
	K: Keychain,
{
	let mut tx_elements = Vec::new();

	for input_value in input_values {
		let key_id = ExtKeychain::derive_key_id(1, input_value as u32, 0, 0, 0);
		tx_elements.push(build::input(input_value, key_id));
	}

	for output_value in output_values {
		let key_id = ExtKeychain::derive_key_id(1, output_value as u32, 0, 0, 0);
		tx_elements.push(build::output(output_value, key_id));
	}

	build::transaction_with_kernel(
		&tx_elements,
		kernel,
		excess,
		keychain,
		&ProofBuilder::new(keychain),
	)
	.unwrap()
}

pub fn test_source() -> TxSource {
	TxSource::Broadcast
}

pub fn clean_output_dir(db_root: String) {
	if let Err(e) = fs::remove_dir_all(db_root) {
		println!("cleaning output dir failed - {:?}", e)
	}
}
