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
extern crate grin_core as core;
extern crate grin_util as util;

//use grin_core as core;

pub mod leaf_set;
pub mod lmdb;
pub mod pmmr;
pub mod prune_list;
pub mod types;

const SEP: u8 = b':';

use byteorder::{BigEndian, WriteBytesExt};

pub use crate::lmdb::*;

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

use std::ffi::OsStr;
use std::fs::{remove_file, rename, File};
use std::path::Path;

/// Creates temporary file with name created by adding `temp_suffix` to `path`.
/// Applies writer function to it and renames temporary file into original specified by `path`.
pub fn save_via_temp_file<F, P, E>(
	path: P,
	temp_suffix: E,
	mut writer: F,
) -> Result<(), std::io::Error>
where
	F: FnMut(&mut File) -> Result<(), std::io::Error>,
	P: AsRef<Path>,
	E: AsRef<OsStr>,
{
	let temp_suffix = temp_suffix.as_ref();
	assert!(!temp_suffix.is_empty());

	let original = path.as_ref();
	let mut _original = original.as_os_str().to_os_string();
	_original.push(temp_suffix);
	// Write temporary file
	let temp_path = Path::new(&_original);
	if temp_path.exists() {
		remove_file(&temp_path)?;
	}

	let mut temp_file = File::create(&temp_path)?;

	// write the new data to the temp file
	writer(&mut temp_file)?;

	// force an fsync on the temp file to ensure bytes are on disk
	temp_file.sync_all()?;

	rename(&temp_path, &original)?;

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
	Ok(Bitmap::deserialize(&buffer))
}
