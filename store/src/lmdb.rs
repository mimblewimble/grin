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

use std::fs;
use std::marker;
use std::sync::Arc;

use lmdb_zero as lmdb;
use lmdb_zero::traits::CreateCursor;
use lmdb_zero::LmdbResultExt;

use crate::core::global;
use crate::core::ser::{self, ProtocolVersion};
use crate::util::RwLock;

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
	/// File handling error
	#[fail(display = "File handling Error")]
	FileErr(String),
	/// Other error
	#[fail(display = "Other Error")]
	OtherErr(String),
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

const DEFAULT_DB_VERSION: ProtocolVersion = ProtocolVersion(3);

/// LMDB-backed store facilitating data access and serialization. All writes
/// are done through a Batch abstraction providing atomicity.
pub struct Store {
	env: Arc<lmdb::Environment>,
	db: Arc<RwLock<Option<Arc<lmdb::Database<'static>>>>>,
	name: String,
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
		db_name: Option<&str>,
		max_readers: Option<u32>,
	) -> Result<Store, Error> {
		let name = match env_name {
			Some(n) => n.to_owned(),
			None => "lmdb".to_owned(),
		};
		let db_name = match db_name {
			Some(n) => n.to_owned(),
			None => "lmdb".to_owned(),
		};
		let full_path = [root_path.to_owned(), name].join("/");
		fs::create_dir_all(&full_path).map_err(|e| {
			Error::FileErr(format!(
				"Unable to create directory 'db_root' to store chain_data: {:?}",
				e
			))
		})?;

		let mut env_builder = lmdb::EnvBuilder::new()?;
		env_builder.set_maxdbs(8)?;

		if let Some(max_readers) = max_readers {
			env_builder.set_maxreaders(max_readers)?;
		}

		let alloc_chunk_size = match global::is_production_mode() {
			true => ALLOC_CHUNK_SIZE_DEFAULT,
			false => ALLOC_CHUNK_SIZE_DEFAULT_TEST,
		};

		let env = unsafe { env_builder.open(&full_path, lmdb::open::NOTLS, 0o600)? };

		debug!("DB Mapsize for {} is {}", full_path, env.info()?.mapsize);
		let res = Store {
			env: Arc::new(env),
			db: Arc::new(RwLock::new(None)),
			name: db_name,
			version: DEFAULT_DB_VERSION,
			alloc_chunk_size,
		};

		{
			let mut w = res.db.write();
			*w = Some(Arc::new(lmdb::Database::open(
				res.env.clone(),
				Some(&res.name),
				&lmdb::DatabaseOptions::new(lmdb::db::CREATE),
			)?));
		}
		Ok(res)
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
			db: self.db.clone(),
			name: self.name.clone(),
			version,
			alloc_chunk_size,
		}
	}

	/// Protocol version for the store.
	pub fn protocol_version(&self) -> ProtocolVersion {
		self.version
	}

	/// Opens the database environment
	pub fn open(&self) -> Result<(), Error> {
		let mut w = self.db.write();
		*w = Some(Arc::new(lmdb::Database::open(
			self.env.clone(),
			Some(&self.name),
			&lmdb::DatabaseOptions::new(lmdb::db::CREATE),
		)?));
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

		// close
		let mut w = self.db.write();
		*w = None;

		unsafe {
			self.env.set_mapsize(new_mapsize)?;
		}

		*w = Some(Arc::new(lmdb::Database::open(
			self.env.clone(),
			Some(&self.name),
			&lmdb::DatabaseOptions::new(lmdb::db::CREATE),
		)?));

		info!(
			"Resized database from {} to {}",
			env_info.mapsize, new_mapsize
		);
		Ok(())
	}

	/// Gets a value from the db, provided its key.
	/// Deserializes the retrieved data using the provided function.
	pub fn get_with<T, F>(
		&self,
		key: &[u8],
		access: &lmdb::ConstAccessor<'_>,
		db: &lmdb::Database<'_>,
		deserialize: F,
	) -> Result<Option<T>, Error>
	where
		F: Fn(&[u8]) -> Result<T, Error>,
	{
		let res: Option<&[u8]> = access.get(db, key).to_opt()?;
		match res {
			None => Ok(None),
			Some(res) => deserialize(res).map(|x| Some(x)),
		}
	}

	/// Gets a `Readable` value from the db, provided its key.
	/// Note: Creates a new read transaction so will *not* see any uncommitted data.
	pub fn get_ser<T: ser::Readable>(&self, key: &[u8]) -> Result<Option<T>, Error> {
		let lock = self.db.read();
		let db = lock
			.as_ref()
			.ok_or_else(|| Error::NotFoundErr("chain db is None".to_string()))?;
		let txn = lmdb::ReadTransaction::new(self.env.clone())?;
		let access = txn.access();

		self.get_with(key, &access, &db, |mut data| {
			ser::deserialize(&mut data, self.protocol_version())
				.map_err(|e| Error::SerErr(format!("{}", e)))
		})
	}

	/// Whether the provided key exists
	pub fn exists(&self, key: &[u8]) -> Result<bool, Error> {
		let lock = self.db.read();
		let db = lock
			.as_ref()
			.ok_or_else(|| Error::NotFoundErr("chain db is None".to_string()))?;
		let txn = lmdb::ReadTransaction::new(self.env.clone())?;
		let access = txn.access();

		let res: Option<&lmdb::Ignore> = access.get(db, key).to_opt()?;
		Ok(res.is_some())
	}

	/// Produces an iterator of (key, value) pairs, where values are `Readable` types
	/// moving forward from the provided key.
	pub fn iter<T: ser::Readable>(&self, from: &[u8]) -> Result<SerIterator<T>, Error> {
		let lock = self.db.read();
		let db = lock
			.as_ref()
			.ok_or_else(|| Error::NotFoundErr("chain db is None".to_string()))?;
		let tx = Arc::new(lmdb::ReadTransaction::new(self.env.clone())?);
		let cursor = Arc::new(tx.cursor(db.clone())?);
		Ok(SerIterator {
			tx,
			cursor,
			seek: false,
			prefix: from.to_vec(),
			version: self.protocol_version(),
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
	pub fn put(&self, key: &[u8], value: &[u8]) -> Result<(), Error> {
		let lock = self.store.db.read();
		let db = lock
			.as_ref()
			.ok_or_else(|| Error::NotFoundErr("chain db is None".to_string()))?;
		self.tx
			.access()
			.put(db, key, value, lmdb::put::Flags::empty())?;
		Ok(())
	}

	/// Writes a single key and its `Writeable` value to the db.
	/// Encapsulates serialization using the (default) version configured on the store instance.
	pub fn put_ser<W: ser::Writeable>(&self, key: &[u8], value: &W) -> Result<(), Error> {
		self.put_ser_with_version(key, value, self.store.protocol_version())
	}

	/// Protocol version used by this batch.
	pub fn protocol_version(&self) -> ProtocolVersion {
		self.store.protocol_version()
	}

	/// Writes a single key and its `Writeable` value to the db.
	/// Encapsulates serialization using the specified protocol version.
	pub fn put_ser_with_version<W: ser::Writeable>(
		&self,
		key: &[u8],
		value: &W,
		version: ProtocolVersion,
	) -> Result<(), Error> {
		let ser_value = ser::ser_vec(value, version);
		match ser_value {
			Ok(data) => self.put(key, &data),
			Err(err) => Err(Error::SerErr(format!("{}", err))),
		}
	}

	/// Low-level access for retrieving data by key.
	/// Takes a function for flexible deserialization.
	pub fn get_with<T, F>(&self, key: &[u8], deserialize: F) -> Result<Option<T>, Error>
	where
		F: Fn(&[u8]) -> Result<T, Error>,
	{
		let access = self.tx.access();
		let lock = self.store.db.read();
		let db = lock
			.as_ref()
			.ok_or_else(|| Error::NotFoundErr("chain db is None".to_string()))?;

		self.store.get_with(key, &access, &db, deserialize)
	}

	/// Whether the provided key exists.
	/// This is in the context of the current write transaction.
	pub fn exists(&self, key: &[u8]) -> Result<bool, Error> {
		let access = self.tx.access();
		let lock = self.store.db.read();
		let db = lock
			.as_ref()
			.ok_or_else(|| Error::NotFoundErr("chain db is None".to_string()))?;
		let res: Option<&lmdb::Ignore> = access.get(db, key).to_opt()?;
		Ok(res.is_some())
	}

	/// Produces an iterator of `Readable` types moving forward from the
	/// provided key.
	pub fn iter<T: ser::Readable>(&self, from: &[u8]) -> Result<SerIterator<T>, Error> {
		self.store.iter(from)
	}

	/// Gets a `Readable` value from the db by provided key and default deserialization strategy.
	pub fn get_ser<T: ser::Readable>(&self, key: &[u8]) -> Result<Option<T>, Error> {
		self.get_with(key, |mut data| {
			match ser::deserialize(&mut data, self.protocol_version()) {
				Ok(res) => Ok(res),
				Err(e) => Err(Error::SerErr(format!("{}", e))),
			}
		})
	}

	/// Deletes a key/value pair from the db
	pub fn delete(&self, key: &[u8]) -> Result<(), Error> {
		let lock = self.store.db.read();
		let db = lock
			.as_ref()
			.ok_or_else(|| Error::NotFoundErr("chain db is None".to_string()))?;
		self.tx.access().del_key(db, key)?;
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
