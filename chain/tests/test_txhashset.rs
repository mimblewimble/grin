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

use grin_chain as chain;
use grin_core as core;

use grin_util as util;

use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::iter::FromIterator;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::chain::store::ChainStore;
use crate::chain::txhashset;
use crate::core::core::BlockHeader;
use crate::util::file;
use grin_core::core::hash::Hashed;

fn clean_output_dir(dir_name: &str) {
	let _ = fs::remove_dir_all(dir_name);
}

#[test]
fn test_unexpected_zip() {
	let db_root = format!(".grin_txhashset_zip");
	clean_output_dir(&db_root);
	{
		let chain_store = ChainStore::new(&db_root).unwrap();
		let store = Arc::new(chain_store);
		txhashset::TxHashSet::open(db_root.clone(), store.clone(), None).unwrap();
		let head = BlockHeader::default();
		// First check if everything works out of the box
		assert!(txhashset::zip_read(db_root.clone(), &head).is_ok());
		let zip_path = Path::new(&db_root).join(format!(
			"txhashset_snapshot_{}.zip",
			head.hash().to_string()
		));
		let zip_file = File::open(&zip_path).unwrap();
		assert!(txhashset::zip_write(PathBuf::from(db_root.clone()), zip_file, &head).is_ok());
		// Remove temp txhashset dir
		let _ = fs::remove_dir_all(
			Path::new(&db_root).join(format!("txhashset_zip_{}", head.hash().to_string())),
		);
		// Then add strange files in the original txhashset folder
		write_file(db_root.clone());
		assert!(txhashset::zip_read(db_root.clone(), &head).is_ok());
		// Check that the temp dir dos not contains the strange files
		let txhashset_zip_path =
			Path::new(&db_root).join(format!("txhashset_zip_{}", head.hash().to_string()));
		assert!(txhashset_contains_expected_files(
			format!("txhashset_zip_{}", head.hash().to_string()),
			txhashset_zip_path.clone()
		));
		let _ = fs::remove_dir_all(
			Path::new(&db_root).join(format!("txhashset_zip_{}", head.hash().to_string())),
		);

		let zip_file = File::open(zip_path).unwrap();
		assert!(txhashset::zip_write(PathBuf::from(db_root.clone()), zip_file, &head).is_ok());
		// Check that the txhashset dir dos not contains the strange files
		let txhashset_path = Path::new(&db_root).join("txhashset");
		assert!(txhashset_contains_expected_files(
			"txhashset".to_string(),
			txhashset_path.clone()
		));
		let _ = fs::remove_dir_all(Path::new(&db_root).join("txhashset"));
	}
	// Cleanup chain directory
	clean_output_dir(&db_root);
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
	)
	.unwrap();
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
	intersection.is_empty()
}
