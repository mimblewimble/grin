// Copyright 2020 The Grin Developers
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

pub mod common;

use self::core::core::hash::Hashed;
use self::core::core::transaction;
use self::core::core::verifier_cache::LruVerifierCache;
use self::core::core::{
	Block, BlockHeader, HeaderVersion, KernelFeatures, NRDRelativeHeight, Transaction, TxKernel,
};
use self::core::global;
use self::core::libtx::aggsig;
use self::core::pow::Difficulty;
use self::core::{consensus, libtx};
use self::keychain::{BlindingFactor, ExtKeychain, Keychain};
use self::pool::types::PoolError;
use self::util::RwLock;
use crate::common::*;
use grin_core as core;
use grin_keychain as keychain;
use grin_pool as pool;
use grin_util as util;
use std::sync::Arc;

// TODO -
// 1) check we do not allow a tx with 2 duplicate NRD kernels
// 2) check we allow back to back txs if relative_height=1 (tx in pool and tx incoming)
// 3) check we do not allow back to back if relative_height=2 (tx in pool and tx incoming)

// relative_height=1
// what about stempool and txpool?
// 1 in txpool, do not allow it in the stempool
// 1 in stempool, do not allow in stempool
// 1 in txpool do not allow in stempool

#[test]
fn test_nrd_kernel_relative_height_rule_1() -> Result<(), PoolError> {
	util::init_test_logger();
	global::set_local_chain_type(global::ChainTypes::AutomatedTesting);
	global::set_local_nrd_enabled(true);

	let db_root = ".grin_nrd_kernel_relative_height_rule_1";
	clean_output_dir(db_root.into());

	let mut chain = ChainAdapter::init(db_root.into()).unwrap();
	let keychain: ExtKeychain = Keychain::from_random_seed(false).unwrap();
	let verifier_cache = Arc::new(RwLock::new(LruVerifierCache::new()));

	// Initialize the chain/txhashset with an initial block
	// so we have a non-empty UTXO set.
	let add_block = |prev_header: BlockHeader, txs: Vec<Transaction>, chain: &mut ChainAdapter| {
		let height = prev_header.height + 1;
		let key_id = ExtKeychain::derive_key_id(1, height as u32, 0, 0, 0);
		let fee = txs.iter().map(|x| x.fee()).sum();
		let reward = libtx::reward::output(
			&keychain,
			&libtx::ProofBuilder::new(&keychain),
			&key_id,
			fee,
			false,
		)
		.unwrap();
		let mut block = Block::new(&prev_header, txs, Difficulty::min(), reward).unwrap();

		// Set the prev_root to the prev hash for testing purposes (no MMR to obtain a root from).
		block.header.prev_root = prev_header.hash();

		chain.update_db_for_block(&block);
		block
	};

	let block = add_block(BlockHeader::default(), vec![], &mut chain);
	let header = block.header;

	// Now create tx to spend that first coinbase (now matured).
	// Provides us with some useful outputs to test with.
	let initial_tx = test_transaction_spending_coinbase(&keychain, &header, vec![10, 20, 30, 40]);

	// Mine that initial tx so we can spend it with multiple txs
	let mut block = add_block(header, vec![initial_tx], &mut chain);
	let mut header = block.header;

	// Initialize a new pool with our chain adapter.
	let mut pool = test_setup(Arc::new(chain.clone()), verifier_cache);

	// Now mine several more blocks out to HF3
	for _ in 0..7 {
		block = add_block(header, vec![], &mut chain);
		header = block.header;
	}

	assert_eq!(header.height, consensus::TESTING_THIRD_HARD_FORK);
	assert_eq!(header.version, HeaderVersion(4));

	let mut kernel = TxKernel::with_features(KernelFeatures::NoRecentDuplicate {
		fee: 6,
		relative_height: NRDRelativeHeight::new(1)?,
	});
	let msg = kernel.msg_to_sign().unwrap();

	// Generate a kernel with public excess and associated signature.
	let excess = BlindingFactor::rand(&keychain.secp());
	let skey = excess.secret_key(&keychain.secp()).unwrap();
	kernel.excess = keychain.secp().commit(0, skey).unwrap();
	let pubkey = &kernel.excess.to_pubkey(&keychain.secp()).unwrap();
	kernel.excess_sig =
		aggsig::sign_with_blinding(&keychain.secp(), &msg, &excess, Some(&pubkey)).unwrap();
	kernel.verify().unwrap();

	let tx1 = test_transaction_with_kernel(
		&keychain,
		vec![10, 20],
		vec![24],
		kernel.clone(),
		excess.clone(),
	);

	let tx2 = test_transaction_with_kernel(
		&keychain,
		vec![24],
		vec![18],
		kernel.clone(),
		excess.clone(),
	);

	// Confirm we can successfully add tx1 with NRD kernel to stempool.
	assert_eq!(
		pool.add_to_pool(test_source(), tx1.clone(), true, &header),
		Ok(()),
	);

	// Confirm we cannot add tx2 to stempool while tx1 is in there (duplicate NRD kernels).
	assert_eq!(
		pool.add_to_pool(test_source(), tx2.clone(), true, &header),
		Err(PoolError::InvalidTx(
			transaction::Error::InvalidNRDRelativeHeight
		))
	);

	// // Confirm we can successfully add tx1 with NRD kernel to txpool.
	// assert_eq!(
	// 	pool.add_to_pool(test_source(), tx1.clone(), false, &header),
	// 	Ok(()),
	// );

	// // Confirm we cannot add tx2 to txpool while tx1 is in there (duplicate NRD kernels).
	// assert_eq!(
	// 	pool.add_to_pool(test_source(), tx2.clone(), false, &header),
	// 	Err(PoolError::InvalidTx(transaction::Error::InvalidNRDRelativeHeight))
	// );

	// // Confirm we cannot add tx2 to stempool while tx1 is in txpool (duplicate NRD kernels).
	// assert_eq!(
	// 	pool.add_to_pool(test_source(), tx2.clone(), true, &header),
	// 	Err(PoolError::InvalidTx(transaction::Error::InvalidNRDRelativeHeight))
	// );

	// assert_eq!(pool.total_size(), 1);

	// let txs = pool.prepare_mineable_transactions().unwrap();
	// assert_eq!(txs.len(), 1);

	// Cleanup db directory
	clean_output_dir(db_root.into());

	Ok(())
}
