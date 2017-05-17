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
extern crate grin_core as core;
extern crate rocksdb;
extern crate tokio_io;
extern crate bytes;
extern crate secp256k1zkp as secp;
extern crate num_bigint;
extern crate time;

const SEP: u8 = ':' as u8;

use std::fmt;
use std::iter::Iterator;
use std::marker::PhantomData;
use std::sync::RwLock;
use tokio_io::codec::{Encoder,Decoder};
use bytes::BytesMut;
use bytes::buf::{FromBuf, IntoBuf};

use byteorder::{WriteBytesExt, BigEndian};
use rocksdb::{DB, WriteBatch, DBCompactionStyle, DBIterator, IteratorMode, Direction};

use core::ser;

pub mod codec;
use codec::{BlockCodec, BlockHasher, TxCodec};

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
	/// Wraps an Io Error
	Io(std::io::Error),
}

impl fmt::Display for Error {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		match self {
			&Error::NotFoundErr => write!(f, "Not Found"),
			&Error::RocksDbErr(ref s) => write!(f, "RocksDb Error: {}", s),
			&Error::SerErr(ref e) => write!(f, "Serialization Error: {}", e.to_string()),
			&Error::Io(ref e) => write!(f, "Codec Error: {}", e)
		}
	}
}

impl From<rocksdb::Error> for Error {
	fn from(e: rocksdb::Error) -> Error {
		Error::RocksDbErr(e.to_string())
	}
}

impl From<std::io::Error> for Error {
	fn from(e: std::io::Error) -> Error {
		Error::Io(e)
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
		Ok(Store { rdb: RwLock::new(db) })
	}

	/// Writes a single key/value pair to the db
	pub fn put(&self, key: &[u8], value: Vec<u8>) -> Result<(), Error> {
		let db = self.rdb.write().unwrap();
		db.put(key, &value[..]).map_err(&From::from)
	}

	/// Writes a single key and a value using a given encoder.
	pub fn put_enc<E: Encoder>(&self, encoder: &mut E, key: &[u8], value: E::Item) -> Result<(), Error> 
		where Error: From<E::Error> {

		let mut data = BytesMut::with_capacity(0);
		encoder.encode(value, &mut data)?;
		self.put(key, data.to_vec())
	}

	/// Gets a value from the db, provided its key and corresponding decoder
	pub fn get_dec<D: Decoder>(&self, decoder: &mut D, key: &[u8]) -> Result<Option<D::Item>, Error> 
		where Error: From<D::Error> {	
			
		let data = self.get(key)?;
		if let Some(buf) = data {
			let mut buf = BytesMut::from_buf(buf);
			decoder.decode(&mut buf).map_err(From::from)
		} else {
			Ok(None)
		}
	}

	/// Gets a value from the db, provided its key
	pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, Error> {
		let db = self.rdb.read().unwrap();
		db.get(key).map(|r| r.map(|o| o.to_vec())).map_err(From::from)
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

	/// Produces an iterator of items decoded by a decoder moving forward from the provided key.
	pub fn iter_dec<D: Decoder>(&self, codec: D, from: &[u8]) -> DecIterator<D> {
		let db = self.rdb.read().unwrap();
		DecIterator {
			iter: db.iterator(IteratorMode::From(from, Direction::Forward)),
			codec: codec
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

	/// Using a given encoder, Writes a single key and a value to the batch.
	pub fn put_enc<E: Encoder>(mut self, encoder: &mut E, key: &[u8], value: E::Item) -> Result<Batch<'a>, Error> where Error: From<E::Error> {
		let mut data = BytesMut::with_capacity(0);
		encoder.encode(value, &mut data)?;
		self.batch.put(key, &data)?;
		Ok(self)
	}

	/// Writes the batch to RocksDb.
	pub fn write(self) -> Result<(), Error> {
		self.store.write(self.batch)
	}
}

/// An iterator that produces items from a `DBIterator` instance with a given `Decoder`.
/// Iterates and decodes returned values
pub struct DecIterator<D> where D: Decoder {
	iter: DBIterator,
	codec: D
}

impl <D> Iterator for DecIterator<D> where D: Decoder {
	type Item = D::Item;
	fn next(&mut self) -> Option<Self::Item> {
		let next = self.iter.next();
		next.and_then(|(_, v)| {
			self.codec.decode(&mut BytesMut::from(v.as_ref())).ok()
		}).unwrap_or(None)
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
