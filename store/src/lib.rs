// Copyright 2016 The Grin Developers
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
extern crate env_logger;
extern crate grin_core as core;
extern crate grin_util as util;
extern crate libc;
extern crate memmap;
extern crate rocksdb;
#[macro_use]
extern crate slog;

pub mod sumtree;

const SEP: u8 = ':' as u8;

use std::fmt;
use std::iter::Iterator;
use std::marker::PhantomData;
use std::sync::RwLock;

use byteorder::{BigEndian, WriteBytesExt};
use rocksdb::{DBCompactionStyle, DBIterator, Direction, IteratorMode, WriteBatch, DB};

use core::ser;

/// Main error type for this crate.
#[derive(Debug)]
pub enum Error {
	/// Couldn't find what we were looking for
	NotFoundErr,
	/// Wraps an error originating from RocksDB (which unfortunately returns
	/// string errors).
	RocksDbErr(String),
	/// Wraps a serialization error for Writeable or Readable
	SerErr(ser::Error),
}

impl fmt::Display for Error {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		match self {
			&Error::NotFoundErr => write!(f, "Not Found"),
			&Error::RocksDbErr(ref s) => write!(f, "RocksDb Error: {}", s),
			&Error::SerErr(ref e) => write!(f, "Serialization Error: {}", e.to_string()),
		}
	}
}

impl From<rocksdb::Error> for Error {
	fn from(e: rocksdb::Error) -> Error {
		Error::RocksDbErr(e.to_string())
	}
}

/// Thread-safe rocksdb wrapper
pub struct Store {
	rdb: RwLock<DB>,
}

unsafe impl Sync for Store {}
unsafe impl Send for Store {}

impl Store {
	/// Opens a new RocksDB at the specified location.
	pub fn open(path: &str) -> Result<Store, Error> {
		let mut opts = rocksdb::Options::default();
		opts.create_if_missing(true);
		opts.set_compaction_style(DBCompactionStyle::Universal);
		opts.set_max_open_files(256);
		opts.set_use_fsync(false);
		let db = try!(DB::open(&opts, &path));
		Ok(Store {
			rdb: RwLock::new(db),
		})
	}

	/// Writes a single key/value pair to the db
	pub fn put(&self, key: &[u8], value: Vec<u8>) -> Result<(), Error> {
		let db = self.rdb.write().unwrap();
		db.put(key, &value[..]).map_err(&From::from)
	}

	/// Writes a single key and its `Writeable` value to the db. Encapsulates
	/// serialization.
	pub fn put_ser<W: ser::Writeable>(&self, key: &[u8], value: &W) -> Result<(), Error> {
		let ser_value = ser::ser_vec(value);
		match ser_value {
			Ok(data) => self.put(key, data),
			Err(err) => Err(Error::SerErr(err)),
		}
	}

	/// Gets a value from the db, provided its key
	pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, Error> {
		let db = self.rdb.read().unwrap();
		db.get(key)
			.map(|r| r.map(|o| o.to_vec()))
			.map_err(From::from)
	}

	/// Gets a `Readable` value from the db, provided its key. Encapsulates
	/// serialization.
	pub fn get_ser<T: ser::Readable>(&self, key: &[u8]) -> Result<Option<T>, Error> {
		self.get_ser_limited(key, 0)
	}

	/// Gets a `Readable` value from the db, provided its key, allowing to
	/// extract only partial data. The underlying Readable size must align
	/// accordingly. Encapsulates serialization.
	pub fn get_ser_limited<T: ser::Readable>(
		&self,
		key: &[u8],
		len: usize,
	) -> Result<Option<T>, Error> {
		let data = try!(self.get(key));
		match data {
			Some(val) => {
				let mut lval = if len > 0 { &val[..len] } else { &val[..] };
				let r = try!(ser::deserialize(&mut lval).map_err(Error::SerErr));
				Ok(Some(r))
			}
			None => Ok(None),
		}
	}

	/// Whether the provided key exists
	pub fn exists(&self, key: &[u8]) -> Result<bool, Error> {
		let db = self.rdb.read().unwrap();
		db.get(key).map(|r| r.is_some()).map_err(From::from)
	}

	/// Deletes a key/value pair from the db
	pub fn delete(&self, key: &[u8]) -> Result<(), Error> {
		let db = self.rdb.write().unwrap();
		db.delete(key).map_err(From::from)
	}

	/// Produces an iterator of `Readable` types moving forward from the
	/// provided
	/// key.
	pub fn iter<T: ser::Readable>(&self, from: &[u8]) -> SerIterator<T> {
		let db = self.rdb.read().unwrap();
		SerIterator {
			iter: db.iterator(IteratorMode::From(from, Direction::Forward)),
			_marker: PhantomData,
		}
	}

	/// Builds a new batch to be used with this store.
	pub fn batch(&self) -> Batch {
		Batch {
			store: self,
			batch: WriteBatch::default(),
		}
	}

	fn write(&self, batch: WriteBatch) -> Result<(), Error> {
		let db = self.rdb.write().unwrap();
		db.write(batch).map_err(From::from)
	}
}

/// Batch to write multiple Writeables to RocksDb in an atomic manner.
pub struct Batch<'a> {
	store: &'a Store,
	batch: WriteBatch,
}

impl<'a> Batch<'a> {
	/// Writes a single key and its `Writeable` value to the batch. The write
	/// function must be called to "commit" the batch to storage.
	pub fn put_ser<W: ser::Writeable>(mut self, key: &[u8], value: &W) -> Result<Batch<'a>, Error> {
		let ser_value = ser::ser_vec(value);
		match ser_value {
			Ok(data) => {
				self.batch.put(key, &data[..])?;
				Ok(self)
			}
			Err(err) => Err(Error::SerErr(err)),
		}
	}

	/// Delete a single key from the batch. The write function
	/// must be called to "commit" the batch to storage.
	pub fn delete(mut self, key: &[u8]) -> Result<Batch<'a>, Error> {
		self.batch.delete(key)?;
		Ok(self)
	}

	/// Writes the batch to RocksDb.
	pub fn write(self) -> Result<(), Error> {
		self.store.write(self.batch)
	}
}

/// An iterator thad produces Readable instances back. Wraps the lower level
/// DBIterator and deserializes the returned values.
pub struct SerIterator<T>
where
	T: ser::Readable,
{
	iter: DBIterator,
	_marker: PhantomData<T>,
}

impl<T> Iterator for SerIterator<T>
where
	T: ser::Readable,
{
	type Item = T;

	fn next(&mut self) -> Option<T> {
		let next = self.iter.next();
		next.and_then(|r| {
			let (_, v) = r;
			ser::deserialize(&mut &v[..]).ok()
		})
	}
}

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

/// unwraps the inner option by converting the none case to a not found error
pub fn option_to_not_found<T>(res: Result<Option<T>, Error>) -> Result<T, Error> {
	match res {
		Ok(None) => Err(Error::NotFoundErr),
		Ok(Some(o)) => Ok(o),
		Err(e) => Err(e),
	}
}
