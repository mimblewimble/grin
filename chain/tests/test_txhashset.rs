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

extern crate grin_chain as chain;
extern crate grin_core as core;
extern crate grin_keychain as keychain;
extern crate grin_store as store;
extern crate grin_util as util;
extern crate grin_wallet as wallet;

use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::iter::FromIterator;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chain::store::ChainStore;
use chain::txhashset;
use chain::types::Tip;
use core::core::{Block, BlockHeader};
use core::pow::Difficulty;
use keychain::{ExtKeychain, Keychain};
use util::file;
use wallet::libtx::{build, reward};

fn clean_output_dir(dir_name: &str) {
	let _ = fs::remove_dir_all(dir_name);
}

#[test]
fn test_some_raw_txs() {
	let db_root = format!(".grin_txhashset_raw_txs");
	clean_output_dir(&db_root);

	let db_env = Arc::new(store::new_env(db_root.clone()));

	let chain_store = ChainStore::new(db_env).unwrap();
	let store = Arc::new(chain_store);
	// open the txhashset, creating a new one if necessary
	let mut txhashset = txhashset::TxHashSet::open(db_root.clone(), store.clone(), None).unwrap();

	let keychain = ExtKeychain::from_random_seed().unwrap();
	let key_id1 = keychain.derive_key_id(1).unwrap();
	let key_id2 = keychain.derive_key_id(2).unwrap();
	let key_id3 = keychain.derive_key_id(3).unwrap();
	let key_id4 = keychain.derive_key_id(4).unwrap();
	let key_id5 = keychain.derive_key_id(5).unwrap();
	let key_id6 = keychain.derive_key_id(6).unwrap();

	// Create a simple block with a single coinbase output
	// so we have something to spend.
	let prev_header = BlockHeader::default();
	let reward_out = reward::output(&keychain, &key_id1, 0, prev_header.height).unwrap();
	let block = Block::new(&prev_header, vec![], Difficulty::one(), reward_out).unwrap();

	// Now apply this initial block to the (currently empty) MMRs.
	// Note: this results in an output MMR with a single leaf node.
	// We need to be careful with pruning while processing the txs below
	// as we cannot prune a tree with a single node in it (no sibling or parent).
	let mut batch = store.batch().unwrap();
	txhashset::extending(&mut txhashset, &mut batch, |extension| {
		extension.apply_block(&block)
	}).unwrap();

	// Make sure we setup the head in the store based on block we just accepted.
	let head = Tip::from_block(&block.header);
	batch.save_head(&head).unwrap();
	batch.commit().unwrap();

	let coinbase_reward = 60_000_000_000;

	// tx1 spends the original coinbase output from the block
	let tx1 = build::transaction(
		vec![
			build::coinbase_input(coinbase_reward, key_id1.clone()),
			build::output(100, key_id2.clone()),
			build::output(150, key_id3.clone()),
		],
		&keychain,
	).unwrap();

	// tx2 attempts to "double spend" the coinbase output from the block (conflicts
	// with tx1)
	let tx2 = build::transaction(
		vec![
			build::coinbase_input(coinbase_reward, key_id1.clone()),
			build::output(100, key_id4.clone()),
		],
		&keychain,
	).unwrap();

	// tx3 spends one output from tx1
	let tx3 = build::transaction(
		vec![
			build::input(100, key_id2.clone()),
			build::output(90, key_id5.clone()),
		],
		&keychain,
	).unwrap();

	// tx4 spends the other output from tx1 and the output from tx3
	let tx4 = build::transaction(
		vec![
			build::input(150, key_id3.clone()),
			build::input(90, key_id5.clone()),
			build::output(220, key_id6.clone()),
		],
		&keychain,
	).unwrap();

	// Now validate the txs against the txhashset (via a readonly extension).
	// Note: we use a single txhashset extension and we can continue to
	// apply txs successfully after a failure.
	let _ = txhashset::extending_readonly(&mut txhashset, |extension| {
		// Note: we pass in an increasing "height" here so we can rollback
		// each tx individually as necessary, while maintaining a long lived
		// txhashset extension.
		assert!(extension.apply_raw_tx(&tx1).is_ok());
		assert!(extension.apply_raw_tx(&tx2).is_err());
		assert!(extension.apply_raw_tx(&tx3).is_ok());
		assert!(extension.apply_raw_tx(&tx4).is_ok());
		Ok(())
	});
}

#[test]
fn test_unexpected_zip() {
	let db_root = format!(".grin_txhashset_zip");
	clean_output_dir(&db_root);
	let db_env = Arc::new(store::new_env(db_root.clone()));
	let chain_store = ChainStore::new(db_env).unwrap();
	let store = Arc::new(chain_store);
	txhashset::TxHashSet::open(db_root.clone(), store.clone(), None).unwrap();
	// First check if everything works out of the box
	assert!(txhashset::zip_read(db_root.clone(), &BlockHeader::default()).is_ok());
	let zip_path = Path::new(&db_root).join("txhashset_snapshot.zip");
	let zip_file = File::open(&zip_path).unwrap();
	assert!(txhashset::zip_write(db_root.clone(), zip_file, &BlockHeader::default()).is_ok());
	// Remove temp txhashset dir
	fs::remove_dir_all(Path::new(&db_root).join("txhashset_zip")).unwrap();
	// Then add strange files in the original txhashset folder
	write_file(db_root.clone());
	assert!(txhashset::zip_read(db_root.clone(), &BlockHeader::default()).is_ok());
	// Check that the temp dir dos not contains the strange files
	let txhashset_zip_path = Path::new(&db_root).join("txhashset_zip");
	assert!(txhashset_contains_expected_files(
		"txhashset_zip".to_string(),
		txhashset_zip_path.clone()
	));
	fs::remove_dir_all(Path::new(&db_root).join("txhashset_zip")).unwrap();

	let zip_file = File::open(zip_path).unwrap();
	assert!(txhashset::zip_write(db_root.clone(), zip_file, &BlockHeader::default()).is_ok());
	// Check that the txhashset dir dos not contains the strange files
	let txhashset_path = Path::new(&db_root).join("txhashset");
	assert!(txhashset_contains_expected_files(
		"txhashset".to_string(),
		txhashset_path.clone()
	));
	fs::remove_dir_all(Path::new(&db_root).join("txhashset")).unwrap();
}

fn write_file(db_root: String) {
	OpenOptions::new()
		.create(true)
		.write(true)
		.open(
			Path::new(&db_root)
				.join("txhashset")
				.join("kernel")
				.join("strange0"),
		)
		.unwrap();
	OpenOptions::new()
		.create(true)
		.write(true)
		.open(Path::new(&db_root).join("txhashset").join("strange1"))
		.unwrap();
	fs::create_dir(Path::new(&db_root).join("txhashset").join("strange_dir")).unwrap();
	OpenOptions::new()
		.create(true)
		.write(true)
		.open(
			Path::new(&db_root)
				.join("txhashset")
				.join("strange_dir")
				.join("strange2"),
		)
		.unwrap();
	fs::create_dir(
		Path::new(&db_root)
			.join("txhashset")
			.join("strange_dir")
			.join("strange_subdir"),
	).unwrap();
	OpenOptions::new()
		.create(true)
		.write(true)
		.open(
			Path::new(&db_root)
				.join("txhashset")
				.join("strange_dir")
				.join("strange_subdir")
				.join("strange3"),
		)
		.unwrap();
}

fn txhashset_contains_expected_files(dirname: String, path_buf: PathBuf) -> bool {
	let list_zip_files = file::list_files(path_buf.into_os_string().into_string().unwrap());
	let zip_files_hashset: HashSet<_> = HashSet::from_iter(list_zip_files.iter().cloned());
	let expected_files = vec![
		dirname,
		"output".to_string(),
		"rangeproof".to_string(),
		"kernel".to_string(),
		"pmmr_hash.bin".to_string(),
		"pmmr_data.bin".to_string(),
	];
	let expected_files_hashset = HashSet::from_iter(expected_files.iter().cloned());
	let intersection: HashSet<_> = zip_files_hashset
		.difference(&expected_files_hashset)
		.collect();
	if intersection.is_empty() {
		true
	} else {
		false
	}
}
