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

//! Main for perf test

use self::chain::types::NoopAdapter;
use self::chain::Chain;
use self::core::core::transaction::Weighting;
use self::core::core::verifier_cache::LruVerifierCache;
use self::core::core::{Block, BlockHeader, KernelFeatures, Transaction};
use self::core::global::ChainTypes;
use self::core::libtx::{self, build, ProofBuilder};
use self::core::pow::Difficulty;
use self::core::{consensus, global, pow};
use self::keychain::{ExtKeychain, ExtKeychainPath, Keychain};
use self::util::RwLock;
use chrono::Duration;
use grin_chain as chain;
use grin_core as core;
use grin_keychain as keychain;
use grin_util as util;
use std::sync::Arc;
use util::print_util::print;

/// Errors thrown by Transaction validation
#[derive(Clone, Eq, Debug, PartialEq)]
pub enum Error {
	SystemTimeError,
	TxError,
}

impl From<std::time::SystemTimeError> for Error {
	fn from(_e: std::time::SystemTimeError) -> Error {
		Error::SystemTimeError
	}
}

impl From<grin_core::core::transaction::Error> for Error {
	fn from(_e: grin_core::core::transaction::Error) -> Error {
		Error::TxError
	}
}

fn clean_output_dir(test_dir: &str) {
	let _ = std::fs::remove_dir_all(test_dir);
}

// Use diff as both diff *and* key_idx for convenience (deterministic private key for test blocks)
fn prepare_block<K>(kc: &K, prev: &BlockHeader, chain: &Chain, diff: u64) -> Block
where
	K: Keychain,
{
	let key_idx = diff as u32;
	prepare_block_key_idx(kc, prev, chain, diff, key_idx)
}

fn prepare_block_key_idx<K>(
	kc: &K,
	prev: &BlockHeader,
	chain: &Chain,
	diff: u64,
	key_idx: u32,
) -> Block
where
	K: Keychain,
{
	let mut b = prepare_block_nosum(kc, prev, diff, key_idx, &[]);
	chain.set_txhashset_roots(&mut b).unwrap();
	b
}

// Use diff as both diff *and* key_idx for convenience (deterministic private key for test blocks)
fn prepare_block_tx<K>(
	kc: &K,
	prev: &BlockHeader,
	chain: &Chain,
	diff: u64,
	txs: &[Transaction],
) -> Block
where
	K: Keychain,
{
	let key_idx = diff as u32;
	prepare_block_tx_key_idx(kc, prev, chain, diff, key_idx, txs)
}

fn prepare_block_tx_key_idx<K>(
	kc: &K,
	prev: &BlockHeader,
	chain: &Chain,
	diff: u64,
	key_idx: u32,
	txs: &[Transaction],
) -> Block
where
	K: Keychain,
{
	let mut b = prepare_block_nosum(kc, prev, diff, key_idx, txs);
	chain.set_txhashset_roots(&mut b).unwrap();
	b
}

fn prepare_block_nosum<K>(
	kc: &K,
	prev: &BlockHeader,
	diff: u64,
	key_idx: u32,
	txs: &[Transaction],
) -> Block
where
	K: Keychain,
{
	let proof_size = global::proofsize();
	let key_id = ExtKeychainPath::new(1, key_idx, 0, 0, 0).to_identifier();

	let height = prev.height + 1;
	let fees = txs.iter().map(|tx| tx.fee(height)).sum();
	let reward =
		libtx::reward::output(kc, &libtx::ProofBuilder::new(kc), &key_id, fees, false).unwrap();
	let mut b = match core::core::Block::new(prev, txs, Difficulty::from_num(diff), reward) {
		Err(e) => panic!("{:?}", e),
		Ok(b) => b,
	};
	b.header.timestamp = prev.timestamp + Duration::seconds(60);
	b.header.pow.total_difficulty = prev.total_difficulty() + Difficulty::from_num(diff);
	b.header.pow.proof = pow::Proof::random(proof_size);
	b
}

fn main() -> Result<(), Error> {
	print("Starting perf test".to_string());
	clean_output_dir(".grin_perf");
	global::set_local_chain_type(ChainTypes::PerfTesting);

	let verifier_cache = Arc::new(RwLock::new(LruVerifierCache::new()));
	let chain = Chain::init(
		".grin_perf".to_string(),
		Arc::new(NoopAdapter {}),
		pow::mine_genesis_block().unwrap(),
		pow::verify_size,
		verifier_cache.clone(),
		false,
	)
	.unwrap();
	let prev = chain.head_header().unwrap();
	let kc = ExtKeychain::from_random_seed(false).unwrap();
	let pb = ProofBuilder::new(&kc);

	let mut head = prev;

	// mine the first block and keep track of the block_hash
	// so we can spend the coinbase later
	let b = prepare_block(&kc, &head, &chain, 2);

	assert!(b.outputs()[0].is_coinbase());
	head = b.header.clone();
	chain
		.process_block(b.clone(), chain::Options::SKIP_POW)
		.unwrap();

	// now mine three further blocks
	for n in 3..6 {
		let b = prepare_block(&kc, &head, &chain, n);
		head = b.header.clone();
		chain.process_block(b, chain::Options::SKIP_POW).unwrap();
	}

	// create a few keys for use in txns
	let key_id2 = ExtKeychainPath::new(1, 2, 0, 0, 0).to_identifier();
	let key_id3 = ExtKeychainPath::new(1, 3, 0, 0, 0).to_identifier();

	let mut out_vec = Vec::new();
	out_vec.push(build::coinbase_input(consensus::REWARD, key_id2.clone()));
	out_vec.push(build::output(consensus::REWARD - 1000, key_id3.clone()));
	for i in 0..1000 {
		out_vec.push(build::output(
			1,
			ExtKeychainPath::new(1, 30 + i, 0, 0, 0).to_identifier(),
		));
	}

	// build a tx to validate
	print("building tx1".to_string());
	let tx1 = build::transaction(
		KernelFeatures::Plain { fee: 0.into() },
		&out_vec[..],
		&kc,
		&pb,
	)
	.unwrap();

	// build a few more that are similar with one replaced output
	out_vec[2] = build::output(1, ExtKeychainPath::new(1, 3000, 0, 0, 0).to_identifier());
	print("building tx2".to_string());
	let tx2 = build::transaction(
		KernelFeatures::Plain { fee: 0.into() },
		&out_vec[..],
		&kc,
		&pb,
	)
	.unwrap();

	out_vec[2] = build::output(1, ExtKeychainPath::new(1, 3001, 0, 0, 0).to_identifier());
	print("building tx3".to_string());
	let tx3 = build::transaction(
		KernelFeatures::Plain { fee: 0.into() },
		&out_vec[..],
		&kc,
		&pb,
	)
	.unwrap();

	out_vec[2] = build::output(1, ExtKeychainPath::new(1, 3002, 0, 0, 0).to_identifier());
	print("building tx4".to_string());
	let tx4 = build::transaction(
		KernelFeatures::Plain { fee: 0.into() },
		&out_vec[..],
		&kc,
		&pb,
	)
	.unwrap();

	out_vec[2] = build::output(1, ExtKeychainPath::new(1, 3003, 0, 0, 0).to_identifier());
	print("building tx5".to_string());
	let tx5 = build::transaction(
		KernelFeatures::Plain { fee: 0.into() },
		&out_vec[..],
		&kc,
		&pb,
	)
	.unwrap();

	print("start validate tx".to_string());
	let now = std::time::SystemTime::now();
	tx1.validate(Weighting::AsTransaction, verifier_cache.clone(), 0)?;
	tx2.validate(Weighting::AsTransaction, verifier_cache.clone(), 0)?;
	tx3.validate(Weighting::AsTransaction, verifier_cache.clone(), 0)?;
	tx4.validate(Weighting::AsTransaction, verifier_cache.clone(), 0)?;
	tx5.validate(Weighting::AsTransaction, verifier_cache.clone(), 0)?;
	let elapsed = now.elapsed()?.as_millis();
	print(format!("tx val time elapsed = {}ms", elapsed));

	// mine block with tx1
	let next = prepare_block_tx(&kc, &head, &chain, 7, &[tx1.clone()]);
	print("start process block".to_string());
	let now = std::time::SystemTime::now();
	chain
		.process_block(next.clone(), chain::Options::SKIP_POW)
		.unwrap();
	let elapsed = now.elapsed()?.as_millis();
	print(format!("block val time elapsed = {}ms", elapsed));

	chain.validate(false).unwrap();

	Ok(())
}
