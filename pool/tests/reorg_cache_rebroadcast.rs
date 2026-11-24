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

//! After a reorg, txs recovered from the reorg cache must be rebroadcast so the
//! network learns about them again (not only re-added to the local mempool).

pub mod common;

use self::core::global;
use self::keychain::{ExtKeychain, Keychain};
use crate::common::*;
use grin_core as core;
use grin_keychain as keychain;
use grin_util as util;
use std::sync::Arc;

#[test]
fn reorg_cache_rebroadcasts_reaccepted_txs() {
	util::init_test_logger();
	global::set_local_chain_type(global::ChainTypes::AutomatedTesting);
	global::set_local_accept_fee_base(1);
	let keychain: ExtKeychain = Keychain::from_random_seed(false).unwrap();

	let db_root = "target/.reorg_cache_rebroadcast";
	clean_output_dir(db_root.into());

	let genesis = genesis_block(&keychain);
	let chain = Arc::new(init_chain(db_root, genesis));

	let adapter = Arc::new(RecordingPoolAdapter::new());
	let mut pool = init_transaction_pool_with_adapter(
		Arc::new(ChainAdapter {
			chain: chain.clone(),
		}),
		adapter.clone(),
	);

	// Mine past HF4 and create spendable coinbase outputs.
	add_some_blocks(&chain, 4 * 3, &keychain);
	let header_1 = chain.get_header_by_height(1).unwrap();
	let initial_tx =
		test_transaction_spending_coinbase(&keychain, &header_1, vec![1_000, 2_000, 3_000]);
	add_block(&chain, &[initial_tx], &keychain);
	let header = chain.head_header().unwrap();

	// Two independent pool txs.
	let tx_a = test_transaction(&keychain, vec![1_000], vec![800]);
	let tx_b = test_transaction(&keychain, vec![2_000], vec![1_500]);

	adapter.clear();
	pool.add_to_pool(test_source(), tx_a.clone(), false, &header)
		.unwrap();
	pool.add_to_pool(test_source(), tx_b.clone(), false, &header)
		.unwrap();
	// Initial accept calls: one per tx.
	assert_eq!(adapter.accepted_count(), 2);
	assert_eq!(pool.total_size(), 2);
	assert_eq!(pool.reorg_cache.read().len(), 2);

	// Simulate reorg: empty the pool while leaving reorg_cache intact.
	pool.txpool.entries.clear();
	assert_eq!(pool.total_size(), 0);
	assert_eq!(pool.reorg_cache.read().len(), 2);

	// Reconcile reorg cache against current tip — txs should re-enter and rebroadcast.
	adapter.clear();
	pool.reconcile_reorg_cache(&header).unwrap();
	assert_eq!(pool.total_size(), 2);
	assert_eq!(
		adapter.accepted_count(),
		2,
		"each successfully re-accepted tx must be rebroadcast"
	);

	// Second reconcile: txs already in pool fail re-add; no extra rebroadcasts.
	adapter.clear();
	pool.reconcile_reorg_cache(&header).unwrap();
	assert_eq!(pool.total_size(), 2);
	assert_eq!(
		adapter.accepted_count(),
		0,
		"already-present txs must not rebroadcast"
	);

	clean_output_dir(db_root.into());
}
