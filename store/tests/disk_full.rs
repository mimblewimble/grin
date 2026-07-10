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

//! Tests for out-of-disk-space detection and durable temp-file writes (#3425).

use grin_store as store;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

fn temp_dir(name: &str) -> PathBuf {
	let mut dir = PathBuf::from("target");
	dir.push("disk_full_tests");
	dir.push(name);
	let _ = fs::remove_dir_all(&dir);
	fs::create_dir_all(&dir).unwrap();
	dir
}

#[test]
fn detects_storage_full_kind() {
	let err = io::Error::new(io::ErrorKind::StorageFull, "no space left on device");
	assert!(store::is_out_of_disk_space(&err));
}

#[test]
fn detects_enospc_raw_code() {
	// ENOSPC is 28 on Linux/macOS.
	let err = io::Error::from_raw_os_error(28);
	assert!(store::is_out_of_disk_space(&err));
}

#[test]
fn map_io_err_upgrades_disk_full() {
	let err = io::Error::new(io::ErrorKind::StorageFull, "disk full");
	let mapped = store::map_io_err(err);
	assert_eq!(mapped.kind(), io::ErrorKind::StorageFull);
	assert!(mapped.to_string().contains("out of disk space"));
}

#[test]
fn does_not_flag_other_errors() {
	let err = io::Error::new(io::ErrorKind::PermissionDenied, "permission denied");
	assert!(!store::is_out_of_disk_space(&err));
}

#[test]
fn save_via_temp_file_success_replaces_original() {
	let dir = temp_dir("save_ok");
	let path = dir.join("data.bin");
	fs::write(&path, b"old").unwrap();

	store::save_via_temp_file(&path, ".tmp", |f| f.write_all(b"new")).unwrap();

	assert_eq!(fs::read(&path).unwrap(), b"new");
	assert!(!dir.join("data.bin.tmp").exists());
	let _ = fs::remove_dir_all(dir);
}

#[test]
fn save_via_temp_file_keeps_original_on_writer_error() {
	let dir = temp_dir("save_fail");
	let path = dir.join("data.bin");
	fs::write(&path, b"original").unwrap();

	let err = store::save_via_temp_file(&path, ".tmp", |_f| {
		Err(io::Error::new(
			io::ErrorKind::StorageFull,
			"no space left on device",
		))
	});
	assert!(err.is_err());
	assert!(store::is_out_of_disk_space(err.as_ref().unwrap_err()));

	// Original content must be preserved; temp file cleaned up.
	assert_eq!(fs::read(&path).unwrap(), b"original");
	assert!(!dir.join("data.bin.tmp").exists());
	let _ = fs::remove_dir_all(dir);
}

#[test]
fn data_file_flush_happy_path_after_hardening() {
	// Full ENOSPC simulation needs a tiny filesystem; here we verify
	// basic append+flush still works after the #3425 changes.
	let dir = temp_dir("data_flush");
	let path = dir.join("data.dat");

	{
		let mut file = store::types::DataFile::<u64>::open(
			&path,
			store::types::SizeInfo::FixedSize(8),
			grin_core::ser::ProtocolVersion(1),
		)
		.unwrap();
		file.append(&42u64).unwrap();
		file.flush().unwrap();
		file.append(&7u64).unwrap();
		file.flush().unwrap();
		assert_eq!(file.size(), 2);
		assert_eq!(file.read(1), Some(42));
		assert_eq!(file.read(2), Some(7));
	}

	let file = store::types::DataFile::<u64>::open(
		&path,
		store::types::SizeInfo::FixedSize(8),
		grin_core::ser::ProtocolVersion(1),
	)
	.unwrap();
	assert_eq!(file.size(), 2);
	assert_eq!(file.read(1), Some(42));
	assert_eq!(file.read(2), Some(7));

	let _ = fs::remove_dir_all(dir);
}
