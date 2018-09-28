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

//! Storage of core types using LMDB.

use std::fs;
use std::marker;
use std::sync::Arc;

use lmdb_zero as lmdb;
use lmdb_zero::traits::CreateCursor;
use lmdb_zero::LmdbResultExt;

use core::ser;

/// Main error type for this lmdb
#[derive(Clone, Eq, PartialEq, Debug, Fail)]
pub enum Error {
	/// Couldn't find what we were looking for
	#[fail(display = "DB Not Found Error: {}", _0)]
	NotFoundErr(String),
	/// Wraps an error originating from RocksDB (which unfortunately returns
	/// string errors).
	#[fail(display = "LMDB error")]
	LmdbErr(lmdb::error::Error),
	/// Wraps a serialization error for Writeable or Readable
	#[fail(display = "Serialization Error")]
	SerErr(String),
}

impl From<lmdb::error::Error> for Error {
	fn from(e: lmdb::error::Error) -> Error {
		Error::LmdbErr(e)
	}
}

/// unwraps the inner option by converting the none case to a not found error
pub fn option_to_not_found<T>(res: Result<Option<T>, Error>, field_name: &str) -> Result<T, Error> {
	match res {
		Ok(None) => Err(Error::NotFoundErr(field_name.to_owned())),
		Ok(Some(o)) => Ok(o),
		Err(e) => Err(e),
	}
}

/// Create a new LMDB env under the provided directory to spawn various
/// databases from.
pub fn new_env(path: String) -> lmdb::Environment {
	let full_path = path + "/lmdb";
	fs::create_dir_all(&full_path).unwrap();
	unsafe {
		let mut env_builder = lmdb::EnvBuilder::new().unwrap();
		env_builder.set_maxdbs(8).unwrap();
		// half a TB should give us plenty room, will be an issue on 32 bits
		// (which we don't support anyway)
		env_builder.set_mapsize(549755813888).unwrap_or_else(|e| {
			panic!("Unable to allocate LMDB space: {:?}", e);
		});

		env_builder
			.open(&full_path, lmdb::open::Flags::empty(), 0o600)
			.unwrap()
	}
}

/// LMDB-backed store facilitating data access and serialization. All writes
/// are done through a Batch abstraction providing atomicity.
pub struct Store {
	env: Arc<lmdb::Environment>,
	db: Arc<lmdb::Database<'static>>,
}

impl Store {
	/// Creates a new store with the provided name under the specified
	/// environment
	pub fn open(env: Arc<lmdb::Environment>, name: &str) -> Store {
		let db = Arc::new(
			lmdb::Database::open(
				env.clone(),
				Some(name),
				&lmdb::DatabaseOptions::new(lmdb::db::CREATE),
			).unwrap(),
		);
		Store { env, db }
	}

	/// Gets a value from the db, provided its key
	pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, Error> {
		let txn = lmdb::ReadTransaction::new(self.env.clone())?;
		let access = txn.access();
		let res = access.get(&self.db, key);
		res.map(|res: &[u8]| res.to_vec())
			.to_opt()
			.map_err(From::from)
	}

	/// Gets a `Readable` value from the db, provided its key. Encapsulates
	/// serialization.
	pub fn get_ser<T: ser::Readable>(&self, key: &[u8]) -> Result<Option<T>, Error> {
		let txn = lmdb::ReadTransaction::new(self.env.clone())?;
		let access = txn.access();
		self.get_ser_access(key, &access)
	}

	fn get_ser_access<T: ser::Readable>(
		&self,
		key: &[u8],
		access: &lmdb::ConstAccessor,
	) -> Result<Option<T>, Error> {
		let res: lmdb::error::Result<&[u8]> = access.get(&self.db, key);
		match res.to_opt() {
			Ok(Some(mut res)) => match ser::deserialize(&mut res) {
				Ok(res) => Ok(Some(res)),
				Err(e) => Err(Error::SerErr(format!("{}", e))),
			},
			Ok(None) => Ok(None),
			Err(e) => Err(From::from(e)),
		}
	}

	/// Whether the provided key exists
	pub fn exists(&self, key: &[u8]) -> Result<bool, Error> {
		let txn = lmdb::ReadTransaction::new(self.env.clone())?;
		let access = txn.access();
		let res: lmdb::error::Result<&lmdb::Ignore> = access.get(&self.db, key);
		res.to_opt().map(|r| r.is_some()).map_err(From::from)
	}

	/// Produces an iterator of `Readable` types moving forward from the
	/// provided key.
	pub fn iter<T: ser::Readable>(&self, from: &[u8]) -> Result<SerIterator<T>, Error> {
		let txn = Arc::new(lmdb::ReadTransaction::new(self.env.clone())?);
		let cursor = Arc::new(txn.cursor(self.db.clone()).unwrap());
		Ok(SerIterator {
			tx: txn,
			cursor: cursor,
			seek: false,
			prefix: from.to_vec(),
			_marker: marker::PhantomData,
		})
	}

	/// Builds a new batch to be used with this store.
	pub fn batch(&self) -> Result<Batch, Error> {
		let txn = lmdb::WriteTransaction::new(self.env.clone())?;
		Ok(Batch {
			store: self,
			tx: txn,
		})
	}
}

/// Batch to write multiple Writeables to db in an atomic manner.
pub struct Batch<'a> {
	store: &'a Store,
	tx: lmdb::WriteTransaction<'a>,
}

impl<'a> Batch<'a> {
	/// Writes a single key/value pair to the db
	pub fn put(&self, key: &[u8], value: Vec<u8>) -> Result<(), Error> {
		self.tx
			.access()
			.put(&self.store.db, key, &value, lmdb::put::Flags::empty())?;
		Ok(())
	}

	/// Writes a single key and its `Writeable` value to the db. Encapsulates
	/// serialization.
	pub fn put_ser<W: ser::Writeable>(&self, key: &[u8], value: &W) -> Result<(), Error> {
		let ser_value = ser::ser_vec(value);
		match ser_value {
			Ok(data) => self.put(key, data),
			Err(err) => Err(Error::SerErr(format!("{}", err))),
		}
	}

	/// gets a value from the db, provided its key
	pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, Error> {
		self.store.get(key)
	}

	/// Whether the provided key exists
	pub fn exists(&self, key: &[u8]) -> Result<bool, Error> {
		self.store.exists(key)
	}

	/// Produces an iterator of `Readable` types moving forward from the
	/// provided key.
	pub fn iter<T: ser::Readable>(&self, from: &[u8]) -> Result<SerIterator<T>, Error> {
		self.store.iter(from)
	}

	/// Gets a `Readable` value from the db, provided its key, taking the
	/// content of the current batch into account.
	pub fn get_ser<T: ser::Readable>(&self, key: &[u8]) -> Result<Option<T>, Error> {
		let access = self.tx.access();
		self.store.get_ser_access(key, &access)
	}

	/// Deletes a key/value pair from the db
	pub fn delete(&self, key: &[u8]) -> Result<(), Error> {
		self.tx.access().del_key(&self.store.db, key)?;
		Ok(())
	}

	/// Writes the batch to db
	pub fn commit(self) -> Result<(), Error> {
		self.tx.commit()?;
		Ok(())
	}

	/// Creates a child of this batch. It will be merged with its parent on
	/// commit, abandoned otherwise.
	pub fn child(&mut self) -> Result<Batch, Error> {
		Ok(Batch {
			store: self.store,
			tx: self.tx.child_tx()?,
		})
	}
}

/// An iterator thad produces Readable instances back. Wraps the lower level
/// DBIterator and deserializes the returned values.
pub struct SerIterator<T>
where
	T: ser::Readable,
{
	tx: Arc<lmdb::ReadTransaction<'static>>,
	cursor: Arc<lmdb::Cursor<'static, 'static>>,
	seek: bool,
	prefix: Vec<u8>,
	_marker: marker::PhantomData<T>,
}

impl<T> Iterator for SerIterator<T>
where
	T: ser::Readable,
{
	type Item = T;

	fn next(&mut self) -> Option<T> {
		let access = self.tx.access();
		let kv = if self.seek {
			Arc::get_mut(&mut self.cursor).unwrap().next(&access)
		} else {
			self.seek = true;
			Arc::get_mut(&mut self.cursor)
				.unwrap()
				.seek_range_k(&access, &self.prefix[..])
		};
		self.deser_if_prefix_match(kv)
	}
}

impl<T> SerIterator<T>
where
	T: ser::Readable,
{
	fn deser_if_prefix_match(&self, kv: Result<(&[u8], &[u8]), lmdb::Error>) -> Option<T> {
		match kv {
			Ok((k, v)) => {
				let plen = self.prefix.len();
				if plen == 0 || k[0..plen] == self.prefix[..] {
					ser::deserialize(&mut &v[..]).ok()
				} else {
					None
				}
			}
			Err(_) => None,
		}
	}
}
