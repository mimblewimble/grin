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

use self::core::global;
use self::keychain::{ExtKeychain, Keychain};
use self::pool::types::PoolError;
use crate::common::*;
use grin_core as core;
use grin_keychain as keychain;
use grin_pool as pool;
use grin_util as util;
use std::sync::Arc;

/// Test we correctly verify coinbase maturity when adding txs to the pool.
#[test]
fn test_coinbase_maturity() {
	util::init_test_logger();
	global::set_local_chain_type(global::ChainTypes::AutomatedTesting);
	global::set_local_accept_fee_base(50_000_000);
	let keychain: ExtKeychain = Keychain::from_random_seed(false).unwrap();

	let db_root = "target/.coinbase_maturity";
	clean_output_dir(db_root.into());

	let genesis = genesis_block(&keychain);
	let chain = Arc::new(init_chain(db_root, genesis));

	// Initialize a new pool with our chain adapter.
	let mut pool = init_transaction_pool(Arc::new(ChainAdapter {
		chain: chain.clone(),
	}));

	// Add a single block, introducing coinbase output to be spent later.
	add_block(&chain, &[], &keychain);

	let header_1 = chain.get_header_by_height(1).unwrap();
	let tx = test_transaction_spending_coinbase(&keychain, &header_1, vec![100]);

	// Coinbase is not yet matured and cannot be spent.
	let header = chain.head_header().unwrap();
	assert_eq!(
		pool.add_to_pool(test_source(), tx.clone(), true, &header)
			.err(),
		Some(PoolError::ImmatureCoinbase)
	);

	// Add 2 more blocks. Original coinbase output is now matured and can be spent.
	add_some_blocks(&chain, 2, &keychain);
	let header = chain.head_header().unwrap();
	assert_eq!(
		pool.add_to_pool(test_source(), tx.clone(), true, &header),
		Ok(())
	);

	clean_output_dir(db_root.into());
}
