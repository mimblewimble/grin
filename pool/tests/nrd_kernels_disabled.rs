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
use self::core::core::{HeaderVersion, KernelFeatures, NRDRelativeHeight};
use self::core::global;
use self::keychain::{ExtKeychain, Keychain};
use self::pool::types::PoolError;
use crate::common::*;
use grin_core as core;
use grin_keychain as keychain;
use grin_pool as pool;
use grin_util as util;
use std::sync::Arc;

#[test]
fn test_nrd_kernels_disabled() {
	util::init_test_logger();
	global::set_local_chain_type(global::ChainTypes::AutomatedTesting);
	global::set_local_nrd_enabled(false);

	let keychain: ExtKeychain = Keychain::from_random_seed(false).unwrap();

	let db_root = "target/.nrd_kernels_disabled";
	clean_output_dir(db_root.into());

	let genesis = genesis_block(&keychain);
	let chain = Arc::new(init_chain(db_root, genesis));

	// Initialize a new pool with our chain adapter.
	let mut pool = init_transaction_pool(Arc::new(ChainAdapter {
		chain: chain.clone(),
	}));

	// Add some blocks.
	add_some_blocks(&chain, 3, &keychain);

	// Spend the initial coinbase.
	let header_1 = chain.get_header_by_height(1).unwrap();
	let tx =
		test_transaction_spending_coinbase(&keychain, &header_1, vec![1_000, 2_000, 3_000, 4_000]);
	add_block(&chain, &[tx], &keychain);

	let tx_1 = test_transaction_with_kernel_features(
		&keychain,
		vec![1_000, 2_000],
		vec![2_400],
		KernelFeatures::NoRecentDuplicate {
			fee: 600.into(),
			relative_height: NRDRelativeHeight::new(1440).unwrap(),
		},
	);

	let header = chain.head_header().unwrap();
	assert!(header.version < HeaderVersion(4));

	assert_eq!(
		pool.add_to_pool(test_source(), tx_1.clone(), false, &header),
		Err(PoolError::NRDKernelNotEnabled)
	);

	// Now mine several more blocks out to HF3
	add_some_blocks(&chain, 5, &keychain);
	let header = chain.head_header().unwrap();
	assert_eq!(header.height, 3 * consensus::TESTING_HARD_FORK_INTERVAL);
	assert_eq!(header.version, HeaderVersion(4));

	// NRD kernel support not enabled via feature flag, so not valid.
	assert_eq!(
		pool.add_to_pool(test_source(), tx_1.clone(), false, &header),
		Err(PoolError::NRDKernelNotEnabled)
	);

	assert_eq!(pool.total_size(), 0);
	let txs = pool.prepare_mineable_transactions().unwrap();
	assert_eq!(txs.len(), 0);

	// Cleanup db directory
	clean_output_dir(db_root.into());
}
