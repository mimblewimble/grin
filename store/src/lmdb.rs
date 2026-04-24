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

//! Storage of core types using LMDB.

use heed::types::Bytes;
use heed::{Database, Env, EnvOpenOptions, RoTxn, RwTxn, WithoutTls};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use std::{fs, thread};

use crate::grin_core::global;
use crate::grin_core::ser::{self, DeserializationMode, ProtocolVersion};
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
#[derive(Clone, Eq, PartialEq, Debug, thiserror::Error)]
pub enum Error {
	/// Couldn't find what we were looking for
	#[error("DB Not Found Error: {0}")]
	NotFoundErr(String),
	/// Wraps an error originating from LMDB
	#[error("LMDB error: {0}")]
	LmdbErr(String),
	/// Wraps a serialization error for Writeable or Readable
	#[error("Serialization Error: {0}")]
	SerErr(ser::Error),
	/// File handling error
	#[error("File handling Error: {0}")]
	FileErr(String),
	/// Other error
	#[error("Other Error: {0}")]
	OtherErr(String),
}

impl From<heed::Error> for Error {
	fn from(e: heed::Error) -> Error {
		Error::LmdbErr(e.to_string())
	}
}

impl From<ser::Error> for Error {
	fn from(e: ser::Error) -> Error {
		Error::SerErr(e)
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

const DEFAULT_ENV_NAME: &'static str = "lmdb";

/// Mapping of database path to environment.
static ENV_MAP: OnceLock<Arc<RwLock<HashMap<String, Env<WithoutTls>>>>> = OnceLock::new();
/// Mapping of database path to count of active batches to wait before resizing.
static ENV_BATCHES_COUNT: OnceLock<Arc<RwLock<HashMap<String, u32>>>> = OnceLock::new();
/// Mapping of database path to check if database is resizing.
static ENV_RESIZING: OnceLock<Arc<RwLock<HashMap<String, bool>>>> = OnceLock::new();

/// LMDB-backed store facilitating data access and serialization. All writes
/// are done through a Batch abstraction providing atomicity.
pub struct Store {
	env: Env<WithoutTls>,
	env_path: String,
	db: Arc<Database<Bytes, Bytes>>,
	name: String,
	version: ProtocolVersion,
	alloc_chunk_size: usize,
}

impl Store {
	/// Create a new LMDB env under the provided directory.
	/// By default creates an environment named "lmdb".
	/// Be aware of transactional semantics in lmdb
	/// (transactions are per environment, not per database).
	/// db with non-default `env_name` will be migrated into default environment.
	pub fn new(
		root_path: &str,
		env_name: Option<&str>,
		db_name: Option<&str>,
		max_readers: Option<u32>,
	) -> Result<Store, Error> {
		let db_name = db_name.unwrap_or_else(|| DEFAULT_ENV_NAME);

		// Database path setup.
		let full_path = Path::new(root_path)
			.join(DEFAULT_ENV_NAME)
			.to_str()
			.unwrap()
			.to_string();
		fs::create_dir_all(&full_path).map_err(|e| {
			Error::FileErr(format!(
				"Unable to create {:?} to store data: {:?}",
				full_path, e
			))
		})?;

		let alloc_chunk_size = match global::is_production_mode() {
			true => ALLOC_CHUNK_SIZE_DEFAULT,
			false => ALLOC_CHUNK_SIZE_DEFAULT_TEST,
		};

		// Environment setup.
		let env_map = ENV_MAP.get_or_init(|| Arc::new(RwLock::new(HashMap::new())));
		let has_env = {
			let r_env_map = env_map.read();
			r_env_map.contains_key(&full_path)
		};
		if !has_env {
			let env = unsafe {
				let mut options = EnvOpenOptions::new().read_txn_without_tls();
				let mut env_options = options.map_size(alloc_chunk_size).max_dbs(8);
				if let Some(max_readers) = max_readers {
					env_options = env_options.max_readers(max_readers);
				}
				env_options.open(&full_path)?
			};
			let (resize, new_size) = needs_resize(&env, alloc_chunk_size);
			if resize {
				unsafe {
					env.resize(new_size)?;
				};
			}
			debug!("DB Mapsize for {} is {}", db_name, env.info().map_size);
			let mut w_env_map = env_map.write();
			w_env_map.insert(full_path.clone(), env);
		}

		// Database setup.
		let r_env_map = env_map.read();
		let env = r_env_map.get(&full_path).unwrap();
		let mut write = env.write_txn()?;
		let db = env.create_database(&mut write, Some(db_name))?;
		write.commit()?;

		let s = Store {
			env: env.clone(),
			env_path: full_path.clone(),
			db: Arc::new(db),
			name: db_name.to_string(),
			version: DEFAULT_DB_VERSION,
			alloc_chunk_size,
		};

		// Migrate to default environment if needed.
		if let Some(env_name) = env_name {
			if env_name != DEFAULT_ENV_NAME {
				let migrate_from = Path::new(root_path).join(env_name);
				if s.migrate_to_default_env(&migrate_from).is_ok() {
					let _ = fs::remove_dir_all(&migrate_from);
				} else {
					error!("Migrating DB {} failed", env_name);
				}
			}
		}

		Ok(s)
	}

	/// Migrate db from provided path to store environment.
	fn migrate_to_default_env(&self, from_path: &Path) -> Result<(), Error> {
		if !from_path.exists() {
			return Ok(());
		};
		debug!("Migrating DB {} to {}", self.name, DEFAULT_ENV_NAME);
		let from_env = unsafe {
			let mut options = EnvOpenOptions::new().read_txn_without_tls();
			let env_options = options.map_size(self.alloc_chunk_size).max_dbs(1);
			env_options.open(from_path)?
		};
		let db_from = {
			let mut write = from_env.write_txn()?;
			let db_name = self.name.as_str();
			let db: Database<Bytes, Bytes> = from_env.create_database(&mut write, Some(db_name))?;
			write.commit()?;
			db
		};
		let mut write_to = self.env.write_txn()?;
		let read_from = from_env.read_txn()?;
		let mut count = 0;
		for kv in db_from.iter(&read_from)? {
			count += 1;
			if let Ok((k, v)) = kv {
				self.db.put(&mut write_to, &k, &v)?;
			}
		}
		write_to.commit()?;
		debug!("Migrated {} records from DB {}", count, self.name);
		Ok(())
	}

	/// Construct a new store using a specific protocol version.
	/// Permits access to the db with legacy protocol versions for db migrations.
	pub fn with_version(&self, version: ProtocolVersion) -> Store {
		Store {
			env: self.env.clone(),
			env_path: self.env_path.clone(),
			db: self.db.clone(),
			name: self.name.clone(),
			version,
			alloc_chunk_size: self.alloc_chunk_size,
		}
	}

	/// Protocol version for the store.
	pub fn protocol_version(&self) -> ProtocolVersion {
		self.version
	}

	/// Gets a value from the db, provided its key.
	/// Deserializes the retrieved data using the provided function.
	fn get_with<F, T>(&self, key: &[u8], read: &RoTxn, deserialize: F) -> Result<Option<T>, Error>
	where
		F: Fn(&[u8], &[u8]) -> Result<T, Error>,
	{
		let res: Option<&[u8]> = self.db.get(read, key)?;
		match res {
			None => Ok(None),
			Some(res) => deserialize(key, res).map(Some),
		}
	}

	/// Gets a `Readable` value from the db, provided its key.
	/// Note: Creates a new read transaction so will *not* see any uncommitted data.
	pub fn get_ser<T: ser::Readable>(
		&self,
		key: &[u8],
		deser_mode: Option<DeserializationMode>,
	) -> Result<Option<T>, Error> {
		let d = match deser_mode {
			Some(d) => d,
			_ => DeserializationMode::default(),
		};
		self.wait_for_resize();
		let read = self.env.read_txn()?;
		self.get_with(key, &read, |_, mut data| {
			ser::deserialize(&mut data, self.protocol_version(), d).map_err(From::from)
		})
	}

	/// Whether the provided key exists.
	pub fn exists(&self, key: &[u8]) -> Result<bool, Error> {
		self.wait_for_resize();
		let read = self.env.read_txn()?;
		let res = self.db.get(&read, key)?;
		Ok(res.is_some())
	}

	/// Produces an iterator from the provided key prefix.
	pub fn iter<F, T>(&self, prefix: &[u8], deserialize: F) -> Result<PrefixIterator<F, T>, Error>
	where
		F: Fn(&[u8], &[u8]) -> Result<T, Error>,
	{
		self.wait_for_resize();
		let read = self.env.clone().static_read_txn()?;
		Ok(PrefixIterator::new(
			self.db.clone(),
			read,
			prefix,
			deserialize,
		))
	}

	/// Wait while DB is resizing.
	fn wait_for_resize(&self) {
		loop {
			let resizing = {
				let res_map = ENV_RESIZING.get_or_init(|| Arc::new(RwLock::new(HashMap::new())));
				let r_res_map = res_map.read();
				r_res_map.get(&self.env_path).map(|r| *r).unwrap_or(false)
			};
			if !resizing {
				break;
			}
			debug!("Wait on {}, resizing DB", self.name);
			thread::sleep(Duration::from_millis(100));
		}
	}

	/// Resize database environment if needed.
	fn maybe_resize(&self) -> Result<(), Error> {
		self.wait_for_resize();
		let (resize, new_size) = needs_resize(&self.env, self.alloc_chunk_size);
		if resize {
			let res_map = ENV_RESIZING.get().unwrap();
			{
				let mut w_res_map = res_map.write();
				w_res_map.insert(self.env_path.clone(), true);
			}
			debug!("Start resizing {} DB", self.name);
			unsafe {
				loop {
					let batches_count =
						ENV_BATCHES_COUNT.get_or_init(|| Arc::new(RwLock::new(HashMap::new())));
					let batches = batches_count.read();
					let cur = batches.get(&self.env_path).unwrap_or(&0);
					if cur == &0 {
						break;
					}
					debug!("Wait {} batches to complete", cur);
					thread::sleep(Duration::from_millis(100));
				}
				self.env.resize(new_size)?;
			}
			{
				let mut w_res_map = res_map.write();
				w_res_map.insert(self.env_path.clone(), false);
			}
			debug!("End resizing {} DB", self.name);
		}
		Ok(())
	}

	/// Builds a new batch to be used with this store.
	pub fn batch(&self) -> Result<Batch<'_>, Error> {
		self.maybe_resize()?;
		on_change_batches_count(&self.env_path, true);
		Ok(Batch::new(self)?)
	}
}

/// Batches counter to decrement value on drop.
struct BatchesCounter<'a> {
	env_path: &'a String,
}

impl Drop for BatchesCounter<'_> {
	fn drop(&mut self) {
		on_change_batches_count(&self.env_path, false);
	}
}

/// Batch to write multiple Writeables to db in an atomic manner.
pub struct Batch<'a> {
	store: &'a Store,
	write: RwTxn<'a>,
	#[allow(dead_code)]
	counter: BatchesCounter<'a>,
}

impl<'a> Batch<'a> {
	/// Creates a new batch for provided db.
	pub fn new(store: &'a Store) -> Result<Batch<'a>, Error> {
		let write = store.env.write_txn()?;
		Ok(Batch {
			store,
			write,
			counter: BatchesCounter {
				env_path: &store.env_path,
			},
		})
	}

	/// Writes a single key/value pair to the db.
	pub fn put(&mut self, key: &[u8], value: &[u8]) -> Result<(), Error> {
		self.store.db.put(&mut self.write, key, value)?;
		Ok(())
	}

	/// Writes a single key and its `Writeable` value to the db.
	/// Encapsulates serialization using the (default) version configured on the store instance.
	pub fn put_ser<W: ser::Writeable>(&mut self, key: &[u8], value: &W) -> Result<(), Error> {
		self.put_ser_with_version(key, value, self.store.protocol_version())
	}

	/// Protocol version used by this batch.
	pub fn protocol_version(&self) -> ProtocolVersion {
		self.store.protocol_version()
	}

	/// Writes a single key and its `Writeable` value to the db.
	/// Encapsulates serialization using the specified protocol version.
	pub fn put_ser_with_version<W: ser::Writeable>(
		&mut self,
		key: &[u8],
		value: &W,
		version: ProtocolVersion,
	) -> Result<(), Error> {
		let ser_value = ser::ser_vec(value, version);
		match ser_value {
			Ok(data) => self.put(key, &data),
			Err(err) => Err(err.into()),
		}
	}

	/// Low-level access for retrieving data by key.
	/// Takes a function for flexible deserialization.
	fn get_with<F, T>(&self, key: &[u8], deserialize: F) -> Result<Option<T>, Error>
	where
		F: Fn(&[u8], &[u8]) -> Result<T, Error>,
	{
		let read = self.write.nested_read_txn()?;
		self.store.get_with(key, &read, deserialize)
	}

	/// Whether the provided key exists.
	/// This is in the context of the current write transaction.
	pub fn exists(&self, key: &[u8]) -> Result<bool, Error> {
		let read = self.write.nested_read_txn()?;
		let res = self.store.db.get(&read, key)?;
		Ok(res.is_some())
	}

	/// Produces an iterator from the provided key prefix.
	pub fn iter<F, T>(&self, prefix: &[u8], deserialize: F) -> Result<PrefixIterator<F, T>, Error>
	where
		F: Fn(&[u8], &[u8]) -> Result<T, Error>,
	{
		self.store.iter(prefix, deserialize)
	}

	/// Gets a `Readable` value from the db by provided key and provided deserialization strategy.
	pub fn get_ser<T: ser::Readable>(
		&self,
		key: &[u8],
		deser_mode: Option<DeserializationMode>,
	) -> Result<Option<T>, Error> {
		let d = match deser_mode {
			Some(d) => d,
			_ => DeserializationMode::default(),
		};
		self.get_with(key, |_, mut data| {
			match ser::deserialize(&mut data, self.protocol_version(), d) {
				Ok(res) => Ok(res),
				Err(e) => Err(From::from(e)),
			}
		})
	}

	/// Deletes a key/value pair from the db.
	pub fn delete(&mut self, key: &[u8]) -> Result<(), Error> {
		self.store.db.delete(&mut self.write, key)?;
		Ok(())
	}

	/// Writes the batch to db.
	pub fn commit(self) -> Result<(), Error> {
		self.write.commit()?;
		Ok(())
	}

	/// Creates a child of this batch. It will be merged with its parent on
	/// commit, abandoned otherwise.
	pub fn child(&mut self) -> Result<Batch<'_>, Error> {
		self.store.maybe_resize()?;
		on_change_batches_count(&self.store.env_path, true);
		let write = self.store.env.nested_write_txn(&mut self.write)?;
		Ok(Batch {
			store: self.store,
			write,
			counter: BatchesCounter {
				env_path: &self.store.env_path,
			},
		})
	}
}

/// An iterator based on key prefix.
/// Caller is responsible for deserialization of the data.
pub struct PrefixIterator<F, T>
where
	F: Fn(&[u8], &[u8]) -> Result<T, Error>,
{
	db: Arc<Database<Bytes, Bytes>>,
	read: Arc<RoTxn<'static, WithoutTls>>,
	keys: Vec<Vec<u8>>,
	skip: usize,
	deserialize: F,
}

impl<F, T> Iterator for PrefixIterator<F, T>
where
	F: Fn(&[u8], &[u8]) -> Result<T, Error>,
{
	type Item = T;

	fn next(&mut self) -> Option<Self::Item> {
		if let Some(k) = self.keys.iter().skip(self.skip).next() {
			let v = self.db.get(&self.read, k).unwrap_or(None);
			if let Some(v) = v {
				return match (self.deserialize)(k, v) {
					Ok(v) => {
						self.skip += 1;
						Some(v)
					}
					Err(_) => None,
				};
			}
		}
		None
	}
}

impl<F, T> PrefixIterator<F, T>
where
	F: Fn(&[u8], &[u8]) -> Result<T, Error>,
{
	/// Initialize a new prefix iterator.
	pub fn new(
		db: Arc<Database<Bytes, Bytes>>,
		read: RoTxn<'static, WithoutTls>,
		prefix: &[u8],
		deserialize: F,
	) -> PrefixIterator<F, T> {
		let keys = if let Ok(iter) = db.prefix_iter(&read, &prefix) {
			iter.move_between_keys()
				.filter(|kv| kv.is_ok())
				.map(|kv| kv.unwrap().0.to_vec())
				.collect::<Vec<Vec<u8>>>()
		} else {
			vec![]
		};
		PrefixIterator {
			db,
			read: Arc::new(read),
			keys,
			skip: 0,
			deserialize,
		}
	}
}

/// Determines whether the environment needs a resize based on a simple percentage threshold.
pub fn needs_resize(env: &Env<WithoutTls>, alloc_chunk_size: usize) -> (bool, usize) {
	let env_info = env.info();
	let stat = env.stat();
	let size_used = stat.page_size as usize * env_info.last_page_number;
	trace!("DB map size: {}", env_info.map_size);
	trace!("Space used: {}", size_used);
	trace!("Space remaining: {}", env_info.map_size - size_used);
	let resize_percent = RESIZE_PERCENT;
	trace!(
		"Percent used: {:.*}  Percent threshold: {:.*}",
		4,
		size_used as f64 / env_info.map_size as f64,
		4,
		resize_percent
	);

	let resize = if size_used as f32 / env_info.map_size as f32 > resize_percent
		|| env_info.map_size < alloc_chunk_size
	{
		trace!("Resize threshold met (percent-based)");
		true
	} else {
		trace!("Resize threshold not met (percent-based)");
		false
	};

	let new_size = if resize {
		if env_info.map_size < alloc_chunk_size {
			alloc_chunk_size
		} else {
			let mut tot = env_info.map_size - (env_info.map_size % alloc_chunk_size);
			while size_used as f32 / tot as f32 > RESIZE_MIN_TARGET_PERCENT {
				tot += alloc_chunk_size;
			}
			tot
		}
	} else {
		env_info.map_size
	};

	if resize {
		debug!("Resizing DB to {} from {}", new_size, env_info.map_size);
	}

	(resize, new_size)
}

/// Increment or decrement active batches count for current environment.
fn on_change_batches_count(env_path: &String, inc: bool) {
	let batches_count = ENV_BATCHES_COUNT.get_or_init(|| Arc::new(RwLock::new(HashMap::new())));
	let mut w_batches = batches_count.write();
	let batches = w_batches.clone();
	let count = {
		let cur = batches.get(env_path).unwrap_or(&0);
		if inc {
			cur + 1
		} else {
			cur - 1
		}
	};
	w_batches.insert(env_path.clone(), count);
}
