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

//! Storage of core types using RocksDB.

#![deny(non_upper_case_globals)]
#![deny(non_camel_case_types)]
#![deny(non_snake_case)]
#![deny(unused_mut)]
#![warn(missing_docs)]

extern crate byteorder;
extern crate croaring;
extern crate env_logger;
extern crate libc;
extern crate lmdb_zero;
extern crate memmap;
extern crate serde;
#[macro_use]
extern crate slog;

#[macro_use]
extern crate grin_core as core;
extern crate grin_util as util;

pub mod leaf_set;
mod lmdb;
pub mod pmmr;
pub mod rm_log;
pub mod types;

const SEP: u8 = ':' as u8;

use byteorder::{BigEndian, WriteBytesExt};

pub use lmdb::*;

/// An iterator thad produces Readable instances back. Wraps the lower level
/// DBIterator and deserializes the returned values.
// pub struct SerIterator<T>
// where
// 	T: ser::Readable,
// {
// 	iter: DBIterator,
// 	_marker: marker::PhantomData<T>,
// }
//
// impl<T> Iterator for SerIterator<T>
// where
// 	T: ser::Readable,
// {
// 	type Item = T;
//
// 	fn next(&mut self) -> Option<T> {
// 		let next = self.iter.next();
// 		next.and_then(|r| {
// 			let (_, v) = r;
// 			ser::deserialize(&mut &v[..]).ok()
// 		})
// 	}
// }

/// Build a db key from a prefix and a byte vector identifier.
pub fn to_key(prefix: u8, k: &mut Vec<u8>) -> Vec<u8> {
	let mut res = Vec::with_capacity(k.len() + 2);
	res.push(prefix);
	res.push(SEP);
	res.append(k);
	res
}

/// Build a db key from a prefix and a numeric identifier.
pub fn u64_to_key<'a>(prefix: u8, val: u64) -> Vec<u8> {
	let mut u64_vec = vec![];
	u64_vec.write_u64::<BigEndian>(val).unwrap();
	u64_vec.insert(0, SEP);
	u64_vec.insert(0, prefix);
	u64_vec
}
