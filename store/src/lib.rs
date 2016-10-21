//! Storage of core types using RocksDB.

#![deny(non_upper_case_globals)]
#![deny(non_camel_case_types)]
#![deny(non_snake_case)]
#![deny(unused_mut)]
#![warn(missing_docs)]

extern crate grin_core as core;
extern crate rocksdb;

use std::sync::RwLock;

use core::ser;

use rocksdb::{DB, Options, Writable, DBCompactionStyle};

/// Main error type for this crate.
#[derive(Debug)]
pub enum Error {
	/// Wraps an error originating from RocksDB (which unfortunately returns
	/// string errors).
	RocksDbErr(String),
	/// Wraps a serialization error for Writeable or Readable
	SerErr(ser::Error),
}

impl From<String> for Error {
	fn from(s: String) -> Error {
		Error::RocksDbErr(s)
	}
}

/// Thread-safe rocksdb wrapper
pub struct Store {
	rdb: RwLock<DB>,
}

impl Store {
	/// Opens a new RocksDB at the specified location.
	pub fn open(path: &str) -> Result<Store, Error> {
		let mut opts = Options::new();
		opts.create_if_missing(true);
		opts.set_compaction_style(DBCompactionStyle::DBUniversalCompaction);
		opts.set_max_open_files(256);
		opts.set_use_fsync(false);
		let db = try!(DB::open(&opts, &path));
		Ok(Store { rdb: RwLock::new(db) })
	}

	/// Writes a single key/value pair to the db
	pub fn put(&self, key: &[u8], value: Vec<u8>) -> Option<Error> {
		let db = self.rdb.write().unwrap();
		db.put(key, &value[..]).err().map(Error::RocksDbErr)
	}

	/// Writes a single key and its `Writeable` value to the db. Encapsulates
	/// serialization.
	pub fn put_ser(&self, key: &[u8], value: &ser::Writeable) -> Option<Error> {
		let ser_value = ser::ser_vec(value);
		match ser_value {
			Ok(data) => self.put(key, data),
			Err(err) => Some(Error::SerErr(err)),
		}
	}

	/// Gets a value from the db, provided its key
	pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, Error> {
		let db = self.rdb.read().unwrap();
		db.get(key).map(|r| r.map(|o| o.to_vec())).map_err(Error::RocksDbErr)
	}

	/// Gets a `Readable` value from the db, provided its key. Encapsulates
	/// serialization.
	pub fn get_ser<T: ser::Readable<T>>(&self, key: &[u8]) -> Result<Option<T>, Error> {
		let data = try!(self.get(key));
		match data {
			Some(val) => {
				let r = try!(ser::deserialize(&mut &val[..]).map_err(Error::SerErr));
				Ok(Some(r))
			}
			None => Ok(None),
		}
	}

	/// Deletes a key/value pair from the db
	pub fn delete(&self, key: &[u8]) -> Option<Error> {
		let db = self.rdb.write().unwrap();
		db.delete(key).err().map(Error::RocksDbErr)
	}
}
