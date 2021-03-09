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

use grin_chain as chain;
use grin_core as core;

use grin_util as util;

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::chain::store::ChainStore;
use crate::chain::txhashset;
use crate::core::core::hash::Hashed;
use crate::core::core::BlockHeader;
use crate::core::global;
use crate::util::file;

fn clean_output_dir(dir_name: &str) {
	let _ = fs::remove_dir_all(dir_name);
}

#[test]
fn test_unexpected_zip() {
	global::set_local_chain_type(global::ChainTypes::AutomatedTesting);
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
		File::create(&Path::new(&db_root).join("txhashset").join("badfile"))
			.expect("problem creating a file");
		File::create(
			&Path::new(&db_root)
				.join("txhashset")
				.join("output")
				.join("badfile"),
		)
		.expect("problem creating a file");

		let files = file::list_files(&Path::new(&db_root).join("txhashset"));
		let expected_files: Vec<_> = vec![
			"badfile",
			"kernel/pmmr_data.bin",
			"kernel/pmmr_hash.bin",
			"kernel/pmmr_size.bin",
			"output/badfile",
			"output/pmmr_data.bin",
			"output/pmmr_hash.bin",
			"rangeproof/pmmr_data.bin",
			"rangeproof/pmmr_hash.bin",
		];
		assert_eq!(
			files,
			expected_files
				.iter()
				.map(|x| PathBuf::from(x))
				.collect::<Vec<_>>()
		);

		assert!(txhashset::zip_read(db_root.clone(), &head).is_ok());
		let _ = fs::remove_dir_all(
			Path::new(&db_root).join(format!("txhashset_zip_{}", head.hash().to_string())),
		);
		let zip_file = File::open(zip_path).unwrap();
		let _ = fs::remove_dir_all(Path::new(&db_root).join("txhashset"));
		assert!(txhashset::zip_write(PathBuf::from(db_root.clone()), zip_file, &head).is_ok());

		// Check that the new txhashset dir contains *only* the expected files
		// No "badfiles" and no "size" file.
		let files = file::list_files(&Path::new(&db_root).join("txhashset"));
		let expected_files: Vec<_> = vec![
			"kernel/pmmr_data.bin",
			"kernel/pmmr_hash.bin",
			"output/pmmr_data.bin",
			"output/pmmr_hash.bin",
			"rangeproof/pmmr_data.bin",
			"rangeproof/pmmr_hash.bin",
		];
		assert_eq!(
			files,
			expected_files
				.iter()
				.map(|x| PathBuf::from(x))
				.collect::<Vec<_>>()
		);
	}
	// Cleanup chain directory
	clean_output_dir(&db_root);
}
