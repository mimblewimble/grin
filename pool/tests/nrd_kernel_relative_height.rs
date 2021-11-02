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

pub mod common;

use self::core::consensus;
use self::core::core::hash::Hashed;
use self::core::core::{HeaderVersion, KernelFeatures, NRDRelativeHeight, TxKernel};
use self::core::global;
use self::core::libtx::aggsig;
use self::keychain::{BlindingFactor, ExtKeychain, Keychain};
use self::pool::types::PoolError;
use crate::common::*;
use grin_core as core;
use grin_keychain as keychain;
use grin_pool as pool;
use grin_util as util;
use std::sync::Arc;

#[test]
fn test_nrd_kernel_relative_height() -> Result<(), PoolError> {
	util::init_test_logger();
	global::set_local_chain_type(global::ChainTypes::AutomatedTesting);
	global::set_local_accept_fee_base(10);
	global::set_local_nrd_enabled(true);

	let keychain: ExtKeychain = Keychain::from_random_seed(false).unwrap();

	let db_root = "target/.nrd_kernel_relative_height";
	clean_output_dir(db_root.into());

	let genesis = genesis_block(&keychain);
	let chain = Arc::new(init_chain(db_root, genesis));

	// Initialize a new pool with our chain adapter.
	let mut pool = init_transaction_pool(Arc::new(ChainAdapter {
		chain: chain.clone(),
	}));

	add_some_blocks(&chain, 3, &keychain);

	let header_1 = chain.get_header_by_height(1).unwrap();

	// Now create tx to spend an early coinbase (now matured).
	// Provides us with some useful outputs to test with.
	let initial_tx =
		test_transaction_spending_coinbase(&keychain, &header_1, vec![1_000, 2_000, 3_000, 4_000]);

	// Mine that initial tx so we can spend it with multiple txs.
	add_block(&chain, &[initial_tx], &keychain);

	// mine past HF4 to see effect of set_local_accept_fee_base
	add_some_blocks(&chain, 8, &keychain);

	let header = chain.head_header().unwrap();

	assert_eq!(header.height, 4 * consensus::TESTING_HARD_FORK_INTERVAL);
	assert_eq!(header.version, HeaderVersion(5));

	let (tx1, tx2, tx3) = {
		let mut kernel = TxKernel::with_features(KernelFeatures::NoRecentDuplicate {
			fee: 600.into(),
			relative_height: NRDRelativeHeight::new(2)?,
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

		// Generate a 2nd NRD kernel sharing the same excess commitment but with different signature.
		let mut kernel2 = kernel.clone();
		kernel2.excess_sig =
			aggsig::sign_with_blinding(&keychain.secp(), &msg, &excess, Some(&pubkey)).unwrap();
		kernel2.verify().unwrap();

		let tx1 = test_transaction_with_kernel(
			&keychain,
			vec![1_000, 2_000],
			vec![2_400],
			kernel.clone(),
			excess.clone(),
		);

		let tx2 = test_transaction_with_kernel(
			&keychain,
			vec![2_400],
			vec![1_800],
			kernel2.clone(),
			excess.clone(),
		);

		// Now reuse kernel excess for tx3 but with NRD relative_height=1 (and different fee).
		let mut kernel_short = TxKernel::with_features(KernelFeatures::NoRecentDuplicate {
			fee: 300.into(),
			relative_height: NRDRelativeHeight::new(1)?,
		});
		let msg_short = kernel_short.msg_to_sign().unwrap();
		kernel_short.excess = kernel.excess;
		kernel_short.excess_sig =
			aggsig::sign_with_blinding(&keychain.secp(), &msg_short, &excess, Some(&pubkey))
				.unwrap();
		kernel_short.verify().unwrap();

		let tx3 = test_transaction_with_kernel(
			&keychain,
			vec![1_800],
			vec![1_500],
			kernel_short.clone(),
			excess.clone(),
		);

		(tx1, tx2, tx3)
	};

	// Confirm we can successfully add tx1 with NRD kernel to stempool.
	assert_eq!(
		pool.add_to_pool(test_source(), tx1.clone(), true, &header),
		Ok(()),
	);
	assert_eq!(pool.stempool.size(), 1);

	// Confirm we cannot add tx2 to stempool while tx1 is in there (duplicate NRD kernels).
	assert_eq!(
		pool.add_to_pool(test_source(), tx2.clone(), true, &header),
		Err(PoolError::NRDKernelRelativeHeight)
	);

	// Confirm we can successfully add tx1 with NRD kernel to txpool,
	// removing existing instance of tx1 from stempool in the process.
	assert_eq!(
		pool.add_to_pool(test_source(), tx1.clone(), false, &header),
		Ok(()),
	);
	assert_eq!(pool.txpool.size(), 1);
	assert_eq!(pool.stempool.size(), 0);

	// Confirm we cannot add tx2 to stempool while tx1 is in txpool (duplicate NRD kernels).
	assert_eq!(
		pool.add_to_pool(test_source(), tx2.clone(), true, &header),
		Err(PoolError::NRDKernelRelativeHeight)
	);

	// Confirm we cannot add tx2 to txpool while tx1 is in there (duplicate NRD kernels).
	assert_eq!(
		pool.add_to_pool(test_source(), tx2.clone(), false, &header),
		Err(PoolError::NRDKernelRelativeHeight)
	);

	assert_eq!(pool.total_size(), 1);
	assert_eq!(pool.txpool.size(), 1);
	assert_eq!(pool.stempool.size(), 0);

	let txs = pool.prepare_mineable_transactions().unwrap();
	assert_eq!(txs.len(), 1);

	// Mine block containing tx1 from the txpool.
	add_block(&chain, &txs, &keychain);
	let header = chain.head_header().unwrap();
	let block = chain.get_block(&header.hash()).unwrap();

	// Confirm the stempool/txpool is empty after reconciling the new block.
	pool.reconcile_block(&block)?;
	assert_eq!(pool.total_size(), 0);
	assert_eq!(pool.txpool.size(), 0);
	assert_eq!(pool.stempool.size(), 0);

	// Confirm we cannot add tx2 to stempool with tx1 in previous block (NRD relative_height=2)
	assert_eq!(
		pool.add_to_pool(test_source(), tx2.clone(), true, &header),
		Err(PoolError::NRDKernelRelativeHeight)
	);

	// Confirm we cannot add tx2 to txpool with tx1 in previous block (NRD relative_height=2)
	assert_eq!(
		pool.add_to_pool(test_source(), tx2.clone(), false, &header),
		Err(PoolError::NRDKernelRelativeHeight)
	);

	// Add another block so NRD relative_height rule is now met.
	add_block(&chain, &[], &keychain);
	let header = chain.head_header().unwrap();

	// Confirm we can now add tx2 to stempool with NRD relative_height rule met.
	assert_eq!(
		pool.add_to_pool(test_source(), tx2.clone(), true, &header),
		Ok(())
	);
	assert_eq!(pool.total_size(), 0);
	assert_eq!(pool.txpool.size(), 0);
	assert_eq!(pool.stempool.size(), 1);

	// Confirm we cannot yet add tx3 to stempool (NRD relative_height=1)
	assert_eq!(
		pool.add_to_pool(test_source(), tx3.clone(), true, &header),
		Err(PoolError::NRDKernelRelativeHeight)
	);

	// Confirm we can now add tx2 to txpool with NRD relative_height rule met.
	assert_eq!(
		pool.add_to_pool(test_source(), tx2.clone(), false, &header),
		Ok(())
	);

	// Confirm we cannot yet add tx3 to txpool (NRD relative_height=1)
	assert_eq!(
		pool.add_to_pool(test_source(), tx3.clone(), false, &header),
		Err(PoolError::NRDKernelRelativeHeight)
	);

	assert_eq!(pool.total_size(), 1);
	assert_eq!(pool.txpool.size(), 1);
	assert_eq!(pool.stempool.size(), 0);

	let txs = pool.prepare_mineable_transactions().unwrap();
	assert_eq!(txs.len(), 1);

	// Mine block containing tx2 from the txpool.
	add_block(&chain, &txs, &keychain);
	let header = chain.head_header().unwrap();
	let block = chain.get_block(&header.hash()).unwrap();
	pool.reconcile_block(&block)?;

	assert_eq!(pool.total_size(), 0);
	assert_eq!(pool.txpool.size(), 0);
	assert_eq!(pool.stempool.size(), 0);

	// Confirm we can now add tx3 to stempool with tx2 in immediate previous block (NRD relative_height=1)
	assert_eq!(
		pool.add_to_pool(test_source(), tx3.clone(), true, &header),
		Ok(())
	);

	assert_eq!(pool.total_size(), 0);
	assert_eq!(pool.txpool.size(), 0);
	assert_eq!(pool.stempool.size(), 1);

	// Confirm we can now add tx3 to txpool with tx2 in immediate previous block (NRD relative_height=1)
	assert_eq!(
		pool.add_to_pool(test_source(), tx3.clone(), false, &header),
		Ok(())
	);

	assert_eq!(pool.total_size(), 1);
	assert_eq!(pool.txpool.size(), 1);
	assert_eq!(pool.stempool.size(), 0);

	// Cleanup db directory
	clean_output_dir(db_root.into());

	Ok(())
}
