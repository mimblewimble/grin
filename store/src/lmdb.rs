// Copyright 2020 The Grin Developers
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

use std::collections::HashMap;
use std::fs;
use std::marker;
use std::sync::Arc;

use lmdb_zero as lmdb;
use lmdb_zero::traits::CreateCursor;
use lmdb_zero::LmdbResultExt;

use crate::core::global;
use crate::core::ser::{self, ProtocolVersion};
use crate::util::{RwLock, RwLockReadGuard};

/// number of bytes to grow the database by when needed
pub const ALLOC_CHUNK_SIZE_DEFAULT: usize = 134_217_728; //128 MB
/// And for test mode, to avoid too much disk allocation on windows
pub const ALLOC_CHUNK_SIZE_DEFAULT_TEST: usize = 1_048_576; //1 MB
const RESIZE_PERCENT: f32 = 0.9;
/// Want to ensure that each resize gives us at least this %
/// of total space free
const RESIZE_MIN_TARGET_PERCENT: f32 = 0.65;

/// Main error type for this lmdb
#[derive(Clone, Eq, PartialEq, Debug, Fail)]
pub enum Error {
	/// Couldn't find what we were looking for
	#[fail(display = "DB Not Found Error: {}", _0)]
	NotFoundErr(String),
	/// Wraps an error originating from LMDB
	#[fail(display = "LMDB error: {} ", _0)]
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
pub fn option_to_not_found<T, F>(res: Result<Option<T>, Error>, field_name: F) -> Result<T, Error>
where
	F: Fn() -> String,
{
	match res {
		Ok(None) => Err(Error::NotFoundErr(field_name())),
		Ok(Some(o)) => Ok(o),
		Err(e) => Err(e),
	}
}

const DEFAULT_DB_VERSION: ProtocolVersion = ProtocolVersion(2);

/// LMDB-backed store facilitating data access and serialization. All writes
/// are done through a Batch abstraction providing atomicity.
pub struct Store {
	env: Arc<lmdb::Environment>,
	dbs: Arc<RwLock<Option<HashMap<String, Arc<lmdb::Database<'static>>>>>>,
	version: ProtocolVersion,
	alloc_chunk_size: usize,
}

impl Store {
	/// Create a new LMDB env under the provided directory.
	/// By default creates an environment named "lmdb".
	/// Be aware of transactional semantics in lmdb
	/// (transactions are per environment, not per database).
	pub fn new(
		root_path: &str,
		env_name: Option<&str>,
		db_names: Vec<&str>,
		max_readers: Option<u32>,
	) -> Result<Store, Error> {
		let name = match env_name {
			Some(n) => n.to_owned(),
			None => "lmdb".to_owned(),
		};
		let full_path = [root_path.to_owned(), name].join("/");
		fs::create_dir_all(&full_path)
			.expect("Unable to create directory 'db_root' to store chain_data");

		let mut env_builder = lmdb::EnvBuilder::new().unwrap();
		env_builder.set_maxdbs(16)?;

		if let Some(max_readers) = max_readers {
			env_builder.set_maxreaders(max_readers)?;
		}

		let alloc_chunk_size = match global::is_production_mode() {
			true => ALLOC_CHUNK_SIZE_DEFAULT,
			false => ALLOC_CHUNK_SIZE_DEFAULT_TEST,
		};

		let env = unsafe { Arc::new(env_builder.open(&full_path, lmdb::open::NOTLS, 0o600)?) };

		debug!(
			"DB Mapsize for {} is {}",
			full_path,
			env.info().as_ref().unwrap().mapsize
		);
		let mut dbs = HashMap::with_capacity(db_names.len());
		for db_name in db_names {
			dbs.insert(
				db_name.to_owned(),
				Arc::new(lmdb::Database::open(
					env.clone(),
					Some(db_name),
					&lmdb::DatabaseOptions::new(lmdb::db::CREATE),
				)?),
			);
		}

		let res = Store {
			env,
			dbs: Arc::new(RwLock::new(Some(dbs))),
			version: DEFAULT_DB_VERSION,
			alloc_chunk_size,
		};

		Ok(res)
	}

	/// Get db by name
	pub fn db(
		dbs: &RwLockReadGuard<Option<HashMap<String, Arc<lmdb::Database<'static>>>>>,
		name: &str,
	) -> Result<Arc<lmdb::Database<'static>>, Error> {
		dbs.as_ref()
			.ok_or_else(|| Error::NotFoundErr(format!("db {} is not initialized", name)))?
			.get(name)
			.ok_or_else(|| Error::NotFoundErr(format!("db {} does not exist", name)))
			.map(Clone::clone)
	}

	/// Construct a new store using a specific protocol version.
	/// Permits access to the db with legacy protocol versions for db migrations.
	pub fn with_version(&self, version: ProtocolVersion) -> Store {
		let alloc_chunk_size = match global::is_production_mode() {
			true => ALLOC_CHUNK_SIZE_DEFAULT,
			false => ALLOC_CHUNK_SIZE_DEFAULT_TEST,
		};
		Store {
			env: self.env.clone(),
			dbs: self.dbs.clone(),
			version: version,
			alloc_chunk_size,
		}
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
			|| env_info.mapsize < self.alloc_chunk_size
		{
			trace!("Resize threshold met (percent-based)");
			Ok(true)
		} else {
			trace!("Resize threshold not met (percent-based)");
			Ok(false)
		}
	}

	/// Increments the database size by as many ALLOC_CHUNK_SIZES
	/// to give a minimum threshold of free space
	pub fn do_resize(&self) -> Result<(), Error> {
		let env_info = self.env.info()?;
		let stat = self.env.stat()?;
		let size_used = stat.psize as usize * env_info.last_pgno;

		let new_mapsize = if env_info.mapsize < self.alloc_chunk_size {
			self.alloc_chunk_size
		} else {
			let mut tot = env_info.mapsize;
			while size_used as f32 / tot as f32 > RESIZE_MIN_TARGET_PERCENT {
				tot += self.alloc_chunk_size;
			}
			tot
		};

		let mut dbs = self.dbs.write();

		let db_names: Vec<String> = dbs
			.as_ref()
			.ok_or_else(|| Error::NotFoundErr("dbs field is None".to_owned()))?
			.keys()
			.cloned()
			.collect(); // close
		*dbs = None;

		unsafe {
			self.env.set_mapsize(new_mapsize)?;
		}

		let mut reopened_dbs = HashMap::with_capacity(db_names.len());
		for db_name in db_names {
			reopened_dbs.insert(
				db_name.to_owned(),
				Arc::new(lmdb::Database::open(
					self.env.clone(),
					Some(&db_name),
					&lmdb::DatabaseOptions::new(lmdb::db::CREATE),
				)?),
			);
		}

		*dbs = Some(reopened_dbs);

		info!(
			"Resized database from {} to {}",
			env_info.mapsize, new_mapsize
		);
		Ok(())
	}

	/// Gets a value from the db, provided its key
	pub fn get_with<K, T, F>(&self, db: &str, key: K, f: F) -> Result<Option<T>, Error>
	where
		F: Fn(&[u8]) -> T,
		K: AsRef<[u8]>,
	{
		let lock = self.dbs.read();
		let db = Self::db(&lock, db)?;
		let txn = lmdb::ReadTransaction::new(self.env.clone())?;
		let access = txn.access();
		let res = access.get(&db, key.as_ref());
		res.map(f).to_opt().map_err(From::from)
	}

	/// Gets a `Readable` value from the db, provided its key. Encapsulates
	/// serialization.
	pub fn get_ser<K, T>(&self, db: &str, key: K) -> Result<Option<T>, Error>
	where
		T: ser::Readable,
		K: AsRef<[u8]>,
	{
		let lock = self.dbs.read();
		let db = Self::db(&lock, db)?;
		let txn = lmdb::ReadTransaction::new(self.env.clone())?;
		let access = txn.access();
		self.get_ser_access(key.as_ref(), &access, &db)
	}

	fn get_ser_access<K, T>(
		&self,
		key: K,
		access: &lmdb::ConstAccessor<'_>,
		db: &lmdb::Database<'static>,
	) -> Result<Option<T>, Error>
	where
		T: ser::Readable,
		K: AsRef<[u8]>,
	{
		let res: lmdb::error::Result<&[u8]> = access.get(&db, key.as_ref());
		match res.to_opt() {
			Ok(Some(mut res)) => match ser::deserialize(&mut res, self.version) {
				Ok(res) => Ok(Some(res)),
				Err(e) => Err(Error::SerErr(format!("{}", e))),
			},
			Ok(None) => Ok(None),
			Err(e) => Err(From::from(e)),
		}
	}

	/// Whether the provided key exists
	pub fn exists<K>(&self, db: &str, key: K) -> Result<bool, Error>
	where
		K: AsRef<[u8]>,
	{
		let lock = self.dbs.read();
		let db = Self::db(&lock, db)?;
		let txn = lmdb::ReadTransaction::new(self.env.clone())?;
		let access = txn.access();
		let res: lmdb::error::Result<&lmdb::Ignore> = access.get(&db, key.as_ref());
		res.to_opt().map(|r| r.is_some()).map_err(From::from)
	}

	/// Produces an iterator of (key, value) pairs, where values are `Readable` types
	/// moving forward from the provided key.
	pub fn iter<K, T>(&self, db: &str, from: K) -> Result<SerIterator<T>, Error>
	where
		T: ser::Readable,
		K: AsRef<[u8]>,
	{
		let lock = self.dbs.read();
		let db = Self::db(&lock, db)?;
		let tx = Arc::new(lmdb::ReadTransaction::new(self.env.clone())?);
		let cursor = Arc::new(tx.cursor(db.clone())?);
		Ok(SerIterator {
			tx,
			cursor,
			seek: false,
			prefix: from.as_ref().to_vec(),
			version: self.version,
			_marker: marker::PhantomData,
		})
	}

	/// Builds a new batch to be used with this store.
	pub fn batch(&self) -> Result<Batch<'_>, Error> {
		// check if the db needs resizing before returning the batch
		if self.needs_resize()? {
			self.do_resize()?;
		}
		let tx = lmdb::WriteTransaction::new(self.env.clone())?;
		Ok(Batch { store: self, tx })
	}
}

/// Batch to write multiple Writeables to db in an atomic manner.
pub struct Batch<'a> {
	store: &'a Store,
	tx: lmdb::WriteTransaction<'a>,
}

impl<'a> Batch<'a> {
	/// Writes a single key/value pair to the db
	pub fn put<K>(&self, db: &str, key: K, value: &[u8]) -> Result<(), Error>
	where
		K: AsRef<[u8]>,
	{
		let lock = self.store.dbs.read();
		let db = Store::db(&lock, db)?;
		self.tx
			.access()
			.put(&db, key.as_ref(), value, lmdb::put::Flags::empty())?;
		Ok(())
	}

	/// Writes a single key and its `Writeable` value to the db.
	/// Encapsulates serialization using the (default) version configured on the store instance.
	pub fn put_ser<K, W>(&self, db: &str, key: K, value: &W) -> Result<(), Error>
	where
		K: AsRef<[u8]>,
		W: ser::Writeable,
	{
		self.put_ser_with_version(db, key.as_ref(), value, self.store.version)
	}

	/// Writes a single key and its `Writeable` value to the db.
	/// Encapsulates serialization using the specified protocol version.
	pub fn put_ser_with_version<K, W>(
		&self,
		db: &str,
		key: K,
		value: &W,
		version: ProtocolVersion,
	) -> Result<(), Error>
	where
		K: AsRef<[u8]>,
		W: ser::Writeable,
	{
		let ser_value = ser::ser_vec(value, version);
		match ser_value {
			Ok(data) => self.put(db, key.as_ref(), &data),
			Err(err) => Err(Error::SerErr(format!("{}", err))),
		}
	}

	/// gets a value from the db, provided its key
	pub fn get_with<F, K, T>(&self, db: &str, key: K, f: F) -> Result<Option<T>, Error>
	where
		F: Fn(&[u8]) -> T,
		K: AsRef<[u8]>,
	{
		self.store.get_with(db, key.as_ref(), f)
	}

	/// Whether the provided key exists
	pub fn exists<K>(&self, db: &str, key: K) -> Result<bool, Error>
	where
		K: AsRef<[u8]>,
	{
		self.store.exists(db, key.as_ref())
	}

	/// Produces an iterator of `Readable` types moving forward from the
	/// provided key.
	pub fn iter<K, T>(&self, db: &str, from: K) -> Result<SerIterator<T>, Error>
	where
		K: AsRef<[u8]>,
		T: ser::Readable,
	{
		self.store.iter(db, from)
	}

	/// Gets a `Readable` value from the db, provided its key, taking the
	/// content of the current batch into account.
	pub fn get_ser<K, T>(&self, db: &str, key: K) -> Result<Option<T>, Error>
	where
		K: AsRef<[u8]>,
		T: ser::Readable,
	{
		let access = self.tx.access();
		let lock = self.store.dbs.read();
		let db = Store::db(&lock, db)?;
		self.store.get_ser_access(key, &access, &db)
	}

	/// Deletes a key/value pair from the db
	pub fn delete<K>(&self, db: &str, key: K) -> Result<(), Error>
	where
		K: AsRef<[u8]>,
	{
		let lock = self.store.dbs.read();
		let db = Store::db(&lock, db)?;
		self.tx.access().del_key(&db, key.as_ref())?;
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

/// An iterator that produces Readable instances back. Wraps the lower level
/// DBIterator and deserializes the returned values.
pub struct SerIterator<T>
where
	T: ser::Readable,
{
	tx: Arc<lmdb::ReadTransaction<'static>>,
	cursor: Arc<lmdb::Cursor<'static, 'static>>,
	seek: bool,
	prefix: Vec<u8>,
	version: ProtocolVersion,
	_marker: marker::PhantomData<T>,
}

impl<T> Iterator for SerIterator<T>
where
	T: ser::Readable,
{
	type Item = (Vec<u8>, T);

	fn next(&mut self) -> Option<(Vec<u8>, T)> {
		let access = self.tx.access();
		let kv = if self.seek {
			Arc::get_mut(&mut self.cursor).unwrap().next(&access)
		} else {
			self.seek = true;
			Arc::get_mut(&mut self.cursor)
				.unwrap()
				.seek_range_k(&access, &self.prefix[..])
		};
		match kv {
			Ok((k, v)) => self.deser_if_prefix_match(k, v),
			Err(_) => None,
		}
	}
}

impl<T> SerIterator<T>
where
	T: ser::Readable,
{
	fn deser_if_prefix_match(&self, key: &[u8], value: &[u8]) -> Option<(Vec<u8>, T)> {
		let plen = self.prefix.len();
		if plen == 0 || (key.len() >= plen && key[0..plen] == self.prefix[..]) {
			if let Ok(value) = ser::deserialize(&mut &value[..], self.version) {
				Some((key.to_vec(), value))
			} else {
				None
			}
		} else {
			None
		}
	}
}
