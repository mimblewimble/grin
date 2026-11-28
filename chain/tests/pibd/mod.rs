// Copyright 2026 The Grin Developers
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

use std::fs;
use std::path::PathBuf;

pub fn copy_testnet_chain() -> tempfile::TempDir {
	let source =
		PathBuf::from(std::env::var_os("GRIN_CHAIN_DATA").expect("GRIN_CHAIN_DATA must be set"));
	let data = source.join("multi_lmdb/data.mdb");
	assert!(data.is_file(), "Missing chain database: {}", data.display());

	let _lock = match fs::File::open(source.join("grin.lock")) {
		Ok(lock) => {
			fs2::FileExt::try_lock_shared(&lock)
				.expect("Stop the source node before copying its chain");
			Some(lock)
		}
		Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
		Err(err) => panic!("Could not open chain lock: {err}"),
	};

	let copy = tempfile::tempdir().expect("Could not create a temporary chain directory");
	let db_dir = copy.path().join("multi_lmdb");
	fs::create_dir(&db_dir).unwrap();
	fs::copy(data, db_dir.join("data.mdb")).unwrap();
	for dir in ["header", "txhashset"] {
		grin_util::file::copy_dir_to(&source.join(dir), &copy.path().join(dir)).unwrap();
	}
	copy
}
