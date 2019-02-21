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

use crate::core::ser;

/// number of bytes to grow the database by when needed
pub const ALLOC_CHUNK_SIZE: usize = 134_217_728 / 64; //128 MB
const RESIZE_PERCENT: f32 = 0.9;

/// Varying allocation chunk sizes for different needs
pub enum AllocChunkSize {
	/// The Chain DB itself
	ChainDB,
	/// The Peer DB
	PeerDB,
	/// Wallet DB
	WalletDB,
}

impl AllocChunkSize {
	/// Return value
	pub fn value(&self) -> usize {
		match *self {
			AllocChunkSize::ChainDB => 134_217_728, //128 MB
			AllocChunkSize::PeerDB => 134_217_728, //128 MB
			AllocChunkSize::WalletDB => 134_217_728, //128 MB
		}
	}
}

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

/// LMDB-backed store facilitating data access and serialization. All writes
/// are done through a Batch abstraction providing atomicity.
pub struct Store {
	env: Arc<lmdb::Environment>,
	db: Option<Arc<lmdb::Database<'static>>>,
	name: String,
}

impl Store {
	/// Create a new LMDB env under the provided directory.
	/// By default creates an environment named "lmdb".
	/// Be aware of transactional semantics in lmdb
	/// (transactions are per environment, not per database).
	pub fn new(path: &str, name: Option<&str>, max_readers: Option<u32>) -> Result<Store, Error> {
		let name = match name {
			Some(n) => n.to_owned(),
			None => "lmdb".to_owned(),
		};
		let full_path = [path.to_owned(), name.clone()].join("/");
			fs::create_dir_all(&full_path)
				.expect("Unable to create directory 'db_root' to store chain_data");

			let mut env_builder = lmdb::EnvBuilder::new().unwrap();
			env_builder.set_maxdbs(8)?;

			if let Some(max_readers) = max_readers {
				env_builder
					.set_maxreaders(max_readers)?;
			}

			let env = unsafe {
				env_builder
					.open(&full_path, lmdb::open::NOTLS, 0o600)?
			};

			debug!("DB Mapsize for {} is {}", full_path, env.info().as_ref().unwrap().mapsize);
			let mut res = Store {
				env: Arc::new(env),
				db: None,
				name: name,
			};

			res.open()?;
			Ok(res)
	}

	/// Opens the database environment
	pub fn open(&mut self) -> Result<(), Error> {
		self.db = Some(Arc::new(
			lmdb::Database::open(
				self.env.clone(),
				Some(&self.name),
				&lmdb::DatabaseOptions::new(lmdb::db::CREATE),
			)?
		));
		Ok(())
	}

	/// Closes the db
	pub fn close(&mut self) -> Result<(), Error> {
		self.db = None;
		Ok(())
	}

	/// Determines whether the environment needs a resize based on a simple percentage threshold
	pub fn needs_resize(&self) -> Result<bool, Error> {
		let env_info = self.env.info()?;
		let stat = self.env.stat()?;

		let size_used = stat.psize as usize * env_info.last_pgno;
		trace!("DB map size: {}", env_info.mapsize);
		trace!("Space used: {}", size_used);
		trace!("Space remaining: {}", env_info.mapsize - size_used);
		let resize_percent = RESIZE_PERCENT;
		trace!(
			"Percent used: {:.*}  Percent threshold: {:.*}",
			4,
			size_used as f64 / env_info.mapsize as f64,
			4,
			resize_percent
		);

		if size_used as f32 / env_info.mapsize as f32 > resize_percent
			|| env_info.mapsize < ALLOC_CHUNK_SIZE
		{
			trace!("Resize threshold met (percent-based)");
			Ok(true)
		} else {
			trace!("Resize threshold not met (percent-based)");
			Ok(false)
		}
	}

	/// Increments the database size by one ALLOC_CHUNK_SIZE
	pub fn do_resize(&mut self) -> Result<(), Error> {
		let env_info = self.env.info()?;
		let new_mapsize = if env_info.mapsize < ALLOC_CHUNK_SIZE {
			ALLOC_CHUNK_SIZE
		} else {
			env_info.mapsize + ALLOC_CHUNK_SIZE
		};
		self.close()?;
		unsafe {
			self.env.set_mapsize(new_mapsize)?;
		}
		self.open()?;
		info!(
			"Resized database from {} to {}",
			env_info.mapsize, new_mapsize
		);
		Ok(())
	}

	/// Gets a value from the db, provided its key
	pub fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, Error> {
		let txn = lmdb::ReadTransaction::new(self.env.clone())?;
		let access = txn.access();
		let res = access.get(&self.db.as_ref().unwrap(), key);
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
		access: &lmdb::ConstAccessor<'_>,
	) -> Result<Option<T>, Error> {
		let res: lmdb::error::Result<&[u8]> = access.get(&self.db.as_ref().unwrap(), key);
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
		let res: lmdb::error::Result<&lmdb::Ignore> = access.get(&self.db.as_ref().unwrap(), key);
		res.to_opt().map(|r| r.is_some()).map_err(From::from)
	}

	/// Produces an iterator of `Readable` types moving forward from the
	/// provided key.
	pub fn iter<T: ser::Readable>(&self, from: &[u8]) -> Result<SerIterator<T>, Error> {
		let tx = Arc::new(lmdb::ReadTransaction::new(self.env.clone())?);
		let cursor = Arc::new(tx.cursor(self.db.as_ref().unwrap().clone()).unwrap());
		Ok(SerIterator {
			tx,
			cursor,
			seek: false,
			prefix: from.to_vec(),
			_marker: marker::PhantomData,
		})
	}

	/// Builds a new batch to be used with this store.
	pub fn batch(&mut self) -> Result<Batch<'_>, Error> {
		// check if the db needs resizing before returning the batch
		if self.needs_resize()? {
			self.do_resize()?;
		}
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
	pub fn put(&self, key: &[u8], value: &[u8]) -> Result<(), Error> {
		self.tx
			.access()
			.put(&self.store.db.as_ref().unwrap(), key, value, lmdb::put::Flags::empty())?;
		Ok(())
	}

	/// Writes a single key and its `Writeable` value to the db. Encapsulates
	/// serialization.
	pub fn put_ser<W: ser::Writeable>(&self, key: &[u8], value: &W) -> Result<(), Error> {
		let ser_value = ser::ser_vec(value);
		match ser_value {
			Ok(data) => self.put(key, &data),
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
		self.tx.access().del_key(&self.store.db.as_ref().unwrap(), key)?;
		Ok(())
	}

	/// Writes the batch to db
	pub fn commit(self) -> Result<(), Error> {
		self.tx.commit()?;
		Ok(())
	}

	/// Creates a child of this batch. It will be merged with its parent on
	/// commit, abandoned otherwise.
	pub fn child(&mut self) -> Result<Batch<'_>, Error> {
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
