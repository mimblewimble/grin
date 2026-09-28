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

//! Storage of core types using RocksDB.

#![deny(non_upper_case_globals)]
#![deny(non_camel_case_types)]
#![deny(non_snake_case)]
#![deny(unused_mut)]
#![warn(missing_docs)]

#[macro_use]
extern crate log;
#[macro_use]
extern crate grin_core;
extern crate grin_util as util;

pub mod leaf_set;
pub mod lmdb;
pub mod pmmr;
pub mod prune_list;
pub mod types;

const SEP: u8 = b':';

use byteorder::{BigEndian, WriteBytesExt};

/// Build a db key from a prefix and a byte vector identifier.
pub fn to_key<K: AsRef<[u8]>>(prefix: u8, k: K) -> Vec<u8> {
	let k = k.as_ref();
	let mut res = Vec::with_capacity(k.len() + 2);
	res.push(prefix);
	res.push(SEP);
	res.extend_from_slice(k);
	res
}

/// Build a db key from a prefix and a byte vector identifier and numeric identifier
pub fn to_key_u64<K: AsRef<[u8]>>(prefix: u8, k: K, val: u64) -> Vec<u8> {
	let k = k.as_ref();
	let mut res = Vec::with_capacity(k.len() + 10);
	res.push(prefix);
	res.push(SEP);
	res.extend_from_slice(k);
	res.write_u64::<BigEndian>(val).unwrap();
	res
}
/// Build a db key from a prefix and a numeric identifier.
pub fn u64_to_key(prefix: u8, val: u64) -> Vec<u8> {
	let mut res = Vec::with_capacity(10);
	res.push(prefix);
	res.push(SEP);
	res.write_u64::<BigEndian>(val).unwrap();
	res
}

pub use crate::lmdb::*;

use std::ffi::OsStr;
use std::fs::{remove_file, rename, File, OpenOptions};
use std::path::Path;

/// Returns true if the I/O error indicates the filesystem is out of space.
///
/// Covers `ErrorKind::StorageFull` and platform-specific `ENOSPC` (28 on most Unix).
pub fn is_out_of_disk_space(err: &io::Error) -> bool {
	if err.kind() == io::ErrorKind::StorageFull {
		return true;
	}
	// ENOSPC on Unix; also match common string forms from lower layers.
	if err.raw_os_error() == Some(28) {
		return true;
	}
	let msg = err.to_string().to_lowercase();
	msg.contains("no space left") || msg.contains("disk full") || msg.contains("not enough space")
}

/// Map an I/O error into a clear StorageFull error when out of disk space.
pub fn map_io_err(err: io::Error) -> io::Error {
	if is_out_of_disk_space(&err) {
		io::Error::new(
			io::ErrorKind::StorageFull,
			format!(
				"out of disk space: {}. Free disk space and restart; avoid wiping chain data unless recovery fails.",
				err
			),
		)
	} else {
		err
	}
}

/// Creates temporary file with name created by adding `temp_suffix` to `path`.
/// Applies writer function to it and renames temporary file into original specified by `path`.
///
/// Durability notes (see also #3352 / #3425):
/// - Writer errors leave the original file untouched (temp is removed).
/// - Temp file is fsync'd before rename.
/// - Parent directory is fsync'd after rename so the rename itself is durable.
pub fn save_via_temp_file<F, P, E>(path: P, temp_suffix: E, mut writer: F) -> Result<(), io::Error>
where
	F: FnMut(&mut File) -> Result<(), io::Error>,
	P: AsRef<Path>,
	E: AsRef<OsStr>,
{
	let temp_suffix = temp_suffix.as_ref();
	assert!(!temp_suffix.is_empty());

	let original = path.as_ref();
	let mut temp_os = original.as_os_str().to_os_string();
	temp_os.push(temp_suffix);
	let temp_path = Path::new(&temp_os);

	if temp_path.exists() {
		remove_file(&temp_path)?;
	}

	let write_result = (|| {
		let mut temp_file = File::create(&temp_path)?;
		writer(&mut temp_file).map_err(map_io_err)?;
		// force an fsync on the temp file to ensure bytes are on disk
		temp_file.sync_all().map_err(map_io_err)?;
		// drop file handle before rename (important on Windows)
		drop(temp_file);
		rename(&temp_path, &original).map_err(map_io_err)?;

		// Fsync the parent directory so the rename is durable.
		if let Some(parent) = original.parent() {
			// On some platforms (e.g. certain Windows configs) directory sync
			// may not be supported; treat that as best-effort.
			if let Ok(dir) = OpenOptions::new().read(true).open(parent) {
				let _ = dir.sync_all();
			}
		}
		Ok(())
	})();

	if let Err(e) = write_result {
		// Best-effort cleanup of the temp file so a failed write cannot
		// leave a partial .tmp that confuses a later retry.
		let _ = remove_file(&temp_path);
		return Err(e);
	}

	Ok(())
}

use croaring::Bitmap;
use std::io::{self, Read};
/// Read Bitmap from a file
pub fn read_bitmap<P: AsRef<Path>>(file_path: P) -> io::Result<Bitmap> {
	let mut bitmap_file = File::open(file_path)?;
	let f_md = bitmap_file.metadata()?;
	let mut buffer = Vec::with_capacity(f_md.len() as usize);
	bitmap_file.read_to_end(&mut buffer)?;
	Ok(Bitmap::deserialize::<croaring::Portable>(&buffer))
}
