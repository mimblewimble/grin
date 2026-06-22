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
use std::cell::RefCell;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::path::Path;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{mpsc, Arc, OnceLock};
use std::time::Duration;
use std::{fs, thread};

use crate::grin_core::global;
use crate::grin_core::ser::{self, DeserializationMode, ProtocolVersion};
use crate::util::RwLock;

/// number of bytes to grow the database by when needed
pub const ALLOC_CHUNK_SIZE_DEFAULT: usize = 134_217_728; //128 MB
/// And for test mode, to avoid too much disk allocation on windows
pub const ALLOC_CHUNK_SIZE_DEFAULT_TEST: usize = 1_048_576; //1 MB
/// Minimal percent of used space when resizing must be performed.
const RESIZE_PERCENT: f32 = 0.9;
/// Want to ensure that each resize gives us at least this %
/// of total space free
const RESIZE_MIN_TARGET_PERCENT: u128 = 65;

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

/// Default environment.
pub const DEFAULT_ENV_NAME: &'static str = "lmdb";
/// Default multi-database environment without prefixes.
const DEFAULT_MULTI_DB_ENV_NAME: &'static str = "multi_lmdb";
/// Migration completion marker in the default database.
const MIGRATION_COMPLETE_KEY: &[u8] = b"__grin_migration_complete";
/// Prefix key separator.
pub const PREFIX_KEY_SEPARATOR: u8 = b':';

/// Mapping of database path to environment state.
static ENV_MAP: OnceLock<RwLock<HashMap<String, EnvState>>> = OnceLock::new();

thread_local! {
	static THREAD_TX_COUNTS: RefCell<HashMap<String, u32>> = RefCell::new(HashMap::new());
}

/// State of active database environment.
struct EnvState {
	env: Env<WithoutTls>,
	open_txs_count: AtomicU32,
	resizing: AtomicBool,
	resize_checking: AtomicBool,
	stores_count: AtomicU32,
}

/// LMDB-backed store facilitating data access and serialization. All writes
/// are done through a Batch abstraction providing atomicity.
pub struct Store {
	env: Env<WithoutTls>,
	env_path: String,
	pre_dbs: Arc<HashMap<u8, Database<Bytes, Bytes>>>,
	def_db: Database<Bytes, Bytes>,
	version: ProtocolVersion,
	alloc_chunk_size: usize,
}

impl Drop for Store {
	fn drop(&mut self) {
		{
			let mut w_map = ENV_MAP.get().unwrap().write();
			let stores_count = w_map
				.get(&self.env_path)
				.unwrap()
				.stores_count
				.load(Ordering::Relaxed);
			w_map
				.get_mut(&self.env_path)
				.unwrap()
				.stores_count
				.store(stores_count - 1, Ordering::Relaxed);
		}
		let no_stores = {
			ENV_MAP
				.get()
				.unwrap()
				.read()
				.get(&self.env_path)
				.unwrap()
				.stores_count
				.load(Ordering::Relaxed)
				== 0
		};
		if no_stores {
			let mut w_map = ENV_MAP.get().unwrap().write();
			w_map.remove(&self.env_path);
		}
	}
}

impl Store {
	/// Create a new LMDB env under the provided directory.
	/// Creates default environment named "multi_lmdb".
	/// Be aware of transactional semantics in lmdb
	/// (transactions are per environment, not per database).
	/// Data from non-default `env_name` and prefixes will be
	/// migrated into default multi db env file if needed.
	pub fn new(
		root_path: &str,
		env_name: Option<&str>,
		db_name: Option<&str>,
		prefixes: Vec<u8>,
		max_readers: Option<u32>,
		db_migration_prog_tx: Option<mpsc::Sender<i8>>,
	) -> Result<Store, Error> {
		let full_path = Path::new(root_path)
			.join(DEFAULT_MULTI_DB_ENV_NAME)
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
		let env_map = ENV_MAP.get_or_init(|| RwLock::new(HashMap::new()));
		let has_env = {
			let r_env_map = env_map.read();
			r_env_map.contains_key(&full_path)
		};
		if !has_env {
			let env = unsafe {
				let mut options = EnvOpenOptions::new().read_txn_without_tls();
				let mut env_options = options.max_dbs(24);
				if let Some(max_readers) = max_readers {
					env_options = env_options.max_readers(max_readers);
				}
				env_options.open(&full_path)?
			};
			debug!("DB Mapsize is {}", env.info().map_size);
			let mut w_env_map = env_map.write();
			w_env_map.insert(
				full_path.clone(),
				EnvState {
					env,
					open_txs_count: AtomicU32::new(0),
					resizing: AtomicBool::new(false),
					resize_checking: AtomicBool::new(false),
					stores_count: AtomicU32::new(1),
				},
			);
		} else {
			let mut w_env_map = env_map.write();
			let stores_count = w_env_map
				.get(&full_path)
				.unwrap()
				.stores_count
				.load(Ordering::Relaxed);
			w_env_map
				.get_mut(&full_path)
				.unwrap()
				.stores_count
				.store(stores_count + 1, Ordering::Relaxed);
		}

		// Database setup.
		let s = {
			let r_env_map = env_map.read();
			let env = r_env_map.get(&full_path).unwrap().env.clone();
			let mut write = env.write_txn()?;
			let def_name = db_name.unwrap_or(DEFAULT_ENV_NAME);
			let def_db = env.create_database(&mut write, Some(def_name))?;
			let mut dbs_map = HashMap::<u8, Database<Bytes, Bytes>>::new();
			for p in prefixes {
				let db = env.create_database(&mut write, Some(p.to_string().as_str()))?;
				dbs_map.insert(p, db);
			}
			write.commit()?;

			let s = Store {
				env: env.clone(),
				env_path: full_path.clone(),
				pre_dbs: Arc::new(dbs_map),
				def_db,
				version: DEFAULT_DB_VERSION,
				alloc_chunk_size,
			};
			s
		};

		// Migrate to default environment if needed.
		let env_name = env_name.unwrap_or(DEFAULT_ENV_NAME);
		if env_name != DEFAULT_MULTI_DB_ENV_NAME {
			let migrate_from = Path::new(root_path).join(env_name);
			if migrate_from.exists() {
				let delete_old_db_file = || -> Result<(), Error> {
					match fs::remove_dir_all(&migrate_from) {
						Ok(_) => Ok(()),
						Err(e) => {
							return Err(Error::FileErr(format!(
								"Can not remove old DB file: {:?}",
								e
							)));
						}
					}
				};
				if s.migration_complete()? {
					if let Err(e) = delete_old_db_file() {
						return Err(e);
					}
				} else {
					let _ = s.clear();
					match s.migrate_to_default_env(db_name, &migrate_from, db_migration_prog_tx) {
						Ok(_) => {
							if let Err(e) = delete_old_db_file() {
								return Err(e);
							}
						}
						Err(e) => {
							error!("DB {} migration error: {:?}", env_name, e);
							match s.clear() {
								Ok(_) => {}
								Err(e) => {
									error!(
										"Can not clear new DB after unsuccessful migration: {:?}",
										e
									)
								}
							}
							return Err(e);
						}
					}
				}
			}
		}

		Ok(s)
	}

	/// Check if migration has already completed successfully.
	fn migration_complete(&self) -> Result<bool, Error> {
		let read = self.env.read_txn()?;
		Ok(self.def_db.get(&read, MIGRATION_COMPLETE_KEY)?.is_some())
	}

	/// Mark migration as successfully completed.
	fn set_migration_complete(&self, write: &mut RwTxn<'_>) -> Result<(), Error> {
		self.def_db.put(write, MIGRATION_COMPLETE_KEY, b"1")?;
		Ok(())
	}

	/// Migrate database from provided path to default environment.
	fn migrate_to_default_env(
		&self,
		from_name: Option<&str>,
		from_path: &Path,
		db_migration_prog_tx: Option<mpsc::Sender<i8>>,
	) -> Result<(), Error> {
		info!("Migrating DB {:?}, please wait...", from_path);

		if let Some(migration_prog_tx) = &db_migration_prog_tx {
			let _ = migration_prog_tx.send(0i8);
		}

		let from_env = unsafe {
			let mut options = EnvOpenOptions::new().read_txn_without_tls();
			let env_options = options.max_dbs(24);
			env_options.open(from_path)?
		};
		let from_used = env_size(&from_env);
		let to_used = env_size(&self.env);
		let to_map_size = self.env.info().map_size;

		// Leave headroom so the migrated env is not immediately above the resize threshold.
		let used = to_used.saturating_add(from_used) as u128;
		let required = ((used * 100 + RESIZE_MIN_TARGET_PERCENT - 1) / RESIZE_MIN_TARGET_PERCENT)
			.min(usize::MAX as u128) as usize;
		let required = round_size_to_chunk(required, self.alloc_chunk_size);

		if required > to_map_size {
			unsafe {
				self.env.resize(required)?;
			}
		}
		let db_from = {
			let mut write = from_env.write_txn()?;
			let db: Database<Bytes, Bytes> = from_env.create_database(&mut write, from_name)?;
			write.commit()?;
			db
		};
		let mut write_to = self.env.write_txn()?;
		let read_from = from_env.read_txn()?;
		let mut count = 0;
		let total = db_from.iter(&read_from)?.count();
		let mut prev_prog = 0;
		for (index, kv) in db_from.iter(&read_from)?.enumerate() {
			if let Some(migration_prog_tx) = &db_migration_prog_tx {
				let prog = 100 * index / total;
				if prev_prog != prog && prog != 100 {
					prev_prog = prog;
					let _ = migration_prog_tx.send(prog as i8);
				}
			}
			let (k, v) = kv?;
			if k.len() > 1 && k[1] == PREFIX_KEY_SEPARATOR {
				let db_name = k.split_at(1).0;
				if let Some(db) = self.pre_dbs.get(&db_name[0]) {
					let key = k.split_at(2).1;
					db.put(&mut write_to, key, &v)?;
					count += 1;
				} else {
					warn!("Migration: unknown DB key: {}", db_name[0]);
				}
			} else {
				self.def_db.put(&mut write_to, k, &v)?;
				count += 1;
			}
		}
		self.set_migration_complete(&mut write_to)?;
		write_to.commit()?;

		if let Some(migration_prog_tx) = &db_migration_prog_tx {
			let _ = migration_prog_tx.send(100i8);
		}

		info!("Migrated {} records from {:?}", count, from_path);
		Ok(())
	}

	/// Get number of active environment transactions.
	fn open_txs_count(&self) -> u32 {
		ENV_MAP
			.get()
			.unwrap()
			.read()
			.get(&self.env_path)
			.unwrap()
			.open_txs_count
			.load(Ordering::Relaxed)
	}

	/// Try to acquire the resize check guard.
	fn start_resize_checking(&self) -> bool {
		ENV_MAP
			.get()
			.unwrap()
			.read()
			.get(&self.env_path)
			.unwrap()
			.resize_checking
			.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
			.is_ok()
	}

	/// Release the resize check guard.
	fn finish_resize_checking(&self) {
		ENV_MAP
			.get()
			.unwrap()
			.read()
			.get(&self.env_path)
			.unwrap()
			.resize_checking
			.store(false, Ordering::Release);
	}

	/// Set flag if environment is waiting for resize.
	fn set_resizing(&self, resizing: bool) {
		ENV_MAP
			.get()
			.unwrap()
			.read()
			.get(&self.env_path)
			.unwrap()
			.resizing
			.store(resizing, Ordering::Release);
	}

	/// Resize database environment if needed.
	fn maybe_resize(&self) {
		if !self.start_resize_checking() {
			return;
		}

		let (resize, new_size) = needs_resize(&self.env, self.alloc_chunk_size);
		if !resize {
			self.finish_resize_checking();
			return;
		}

		let env_path = self.env_path.clone();
		let env = self.env.clone();

		self.set_resizing(true);

		// Resize immediately or at another thread to not interrupt current
		// transaction waiting all open transactions to be closed.
		if self.open_txs_count() != 0 {
			debug!("Waiting txs to be closed before DB {} resize", env_path);
			thread::spawn(move || {
				loop {
					let txs_count = ENV_MAP
						.get()
						.unwrap()
						.read()
						.get(&env_path)
						.unwrap()
						.open_txs_count
						.load(Ordering::Relaxed);
					if txs_count == 0 {
						debug!("Start resizing DB {}", env_path);
						break;
					}
					thread::sleep(Duration::from_millis(100));
				}

				unsafe {
					match env.resize(new_size) {
						Ok(_) => debug!("End resizing DB {}", env_path),
						Err(e) => error!("Resize DB {} error: {:?}", env_path, e),
					}
				}

				let mut w_env_map = ENV_MAP.get().unwrap().write();
				let env_state = w_env_map.get_mut(&env_path).unwrap();
				env_state.resizing.store(false, Ordering::Release);
				env_state.resize_checking.store(false, Ordering::Release);
			});
		} else {
			debug!("Start immediate resizing DB {}", env_path);
			unsafe {
				match env.resize(new_size) {
					Ok(_) => debug!("End resizing DB {}", env_path),
					Err(e) => error!("Resize DB {} error: {:?}", env_path, e),
				}
			}
			self.set_resizing(false);
			self.finish_resize_checking();
		}
	}

	/// Clear all data from database environment.
	fn clear(&self) -> Result<(), Error> {
		let mut w = self.env.write_txn()?;
		self.def_db.clear(&mut w)?;
		for db in self.pre_dbs.values() {
			db.clear(&mut w)?;
		}
		w.commit()?;
		Ok(())
	}

	/// Protocol version for the store.
	pub fn protocol_version(&self) -> ProtocolVersion {
		self.version
	}

	/// Get database from provided key or return default.
	fn get_db(&self, db_key: Option<u8>) -> Result<&Database<Bytes, Bytes>, Error> {
		match db_key {
			Some(db) => {
				if let Some(db) = self.pre_dbs.get(&db) {
					Ok(db)
				} else {
					Err(Error::OtherErr("db for provided key not found".to_string()))
				}
			}
			None => Ok(&self.def_db),
		}
	}

	/// Gets a value from the database, provided its key.
	/// Deserializes the retrieved data using the provided function.
	fn get_with<F, T>(
		&self,
		db_key: Option<u8>,
		key: &[u8],
		read: &RoTxn,
		deserialize: F,
	) -> Result<Option<T>, Error>
	where
		F: Fn(&[u8], &[u8]) -> Result<T, Error>,
	{
		let db = self.get_db(db_key)?;
		let res: Option<&[u8]> = db.get(read, key)?;
		match res {
			None => Ok(None),
			Some(res) => deserialize(key, res).map(Some),
		}
	}

	/// Gets a `Readable` value from the database, provided its key.
	/// Note: Creates a new read transaction so will *not* see any uncommitted data.
	pub fn get_ser<T: ser::Readable>(
		&self,
		db_key: Option<u8>,
		key: &[u8],
		deser_mode: Option<DeserializationMode>,
	) -> Result<Option<T>, Error> {
		let _tx_counter = self.enter_tx();

		let res = {
			let d = match deser_mode {
				Some(d) => d,
				_ => DeserializationMode::default(),
			};
			match self.env.read_txn() {
				Ok(read) => self.get_with(db_key, key, &read, |_, mut data| {
					ser::deserialize(&mut data, self.protocol_version(), d).map_err(From::from)
				}),
				Err(e) => Err(Error::from(e)),
			}
		};
		res
	}

	/// Whether the key exists at the provided database key.
	pub fn exists(&self, db_key: Option<u8>, key: &[u8]) -> Result<bool, Error> {
		let _tx_counter = self.enter_tx();

		let res = {
			match self.env.read_txn() {
				Ok(read) => {
					let db_res = self.get_db(db_key);
					match db_res {
						Ok(db) => {
							let res = db.get(&read, key);
							match res {
								Ok(r) => Ok(r.is_some()),
								Err(e) => Err(Error::from(e)),
							}
						}
						Err(e) => Err(Error::from(e)),
					}
				}
				Err(e) => Err(Error::from(e)),
			}
		};
		res
	}

	/// Produces an iterator from the provided database key.
	pub fn iter<'a, F, T>(
		&self,
		db_key: Option<u8>,
		deserialize: F,
	) -> Result<DatabaseIterator<'a, F, T>, Error>
	where
		F: Fn(&[u8], &[u8]) -> Result<T, Error>,
	{
		let tx_counter = self.enter_tx();

		let res = {
			match self.env.clone().static_read_txn() {
				Ok(read) => {
					let db_res = self.get_db(db_key);
					match db_res {
						Ok(db) => DatabaseIterator::new(
							Arc::new(db.clone()),
							Some(tx_counter),
							read,
							deserialize,
						),
						Err(e) => Err(Error::from(e)),
					}
				}
				Err(e) => Err(Error::from(e)),
			}
		};
		res
	}

	/// Builds a new batch to be used with this store.
	pub fn batch(&self) -> Result<Batch<'_>, Error> {
		self.maybe_resize();
		Batch::new(self)
	}

	/// Increment the open-tx counter, blocking during resize unless this thread already holds a tx.
	fn enter_tx(&self) -> TxCounter {
		loop {
			let mut map = ENV_MAP.get().unwrap().write();
			let state = map.get_mut(&self.env_path).unwrap();
			let nested_tx = THREAD_TX_COUNTS.with(|txs| {
				txs.borrow()
					.get(&self.env_path)
					.is_some_and(|count| *count > 0)
			});
			if !state.resizing.load(Ordering::Acquire) || nested_tx {
				state.open_txs_count.fetch_add(1, Ordering::Relaxed);
				THREAD_TX_COUNTS.with(|txs| {
					let mut txs = txs.borrow_mut();
					*txs.entry(self.env_path.clone()).or_insert(0) += 1;
				});
				return TxCounter {
					env_path: self.env_path.clone(),
					_not_send: PhantomData,
				};
			}
			drop(map);
			thread::sleep(Duration::from_millis(10));
		}
	}
}

/// Environment transactions counter, allows to decrement value on drop.
pub struct TxCounter {
	env_path: String,
	_not_send: PhantomData<Rc<()>>,
}

impl Drop for TxCounter {
	fn drop(&mut self) {
		THREAD_TX_COUNTS.with(|txs| {
			let mut txs = txs.borrow_mut();
			if let Some(count) = txs.get_mut(&self.env_path) {
				*count -= 1;
				if *count == 0 {
					txs.remove(&self.env_path);
				}
			}
		});
		let mut w_env_map = ENV_MAP.get().unwrap().write();
		let env_state = w_env_map.get_mut(&self.env_path).unwrap();
		let open_txs_count = env_state.open_txs_count.load(Ordering::Relaxed);
		env_state
			.open_txs_count
			.store(open_txs_count - 1, Ordering::Relaxed);
	}
}

/// Batch to write multiple Writeables to the database in an atomic manner.
pub struct Batch<'a> {
	store: &'a Store,
	write: RwTxn<'a>,
	#[allow(dead_code)]
	tx_counter: Option<TxCounter>,
}

impl<'a> Batch<'a> {
	/// Creates a new batch for provided store.
	pub fn new(store: &'a Store) -> Result<Batch<'a>, Error> {
		let tx_counter = store.enter_tx();
		let write = store.env.write_txn()?;
		Ok(Batch {
			store,
			write,
			tx_counter: Some(tx_counter),
		})
	}

	/// Writes a single key/value pair to the provided database key.
	pub fn put(&mut self, db_key: Option<u8>, key: &[u8], value: &[u8]) -> Result<(), Error> {
		let db = self.store.get_db(db_key)?;
		let w = &mut self.write;
		db.put(w, key, value)?;
		Ok(())
	}

	/// Writes a single key and its `Writeable` value to the provided database key.
	/// Encapsulates serialization using the (default) version configured on the store instance.
	pub fn put_ser<W: ser::Writeable>(
		&mut self,
		db_key: Option<u8>,
		key: &[u8],
		value: &W,
	) -> Result<(), Error> {
		self.put_ser_with_version(db_key, key, value, self.store.protocol_version())
	}

	/// Protocol version used by this batch.
	pub fn protocol_version(&self) -> ProtocolVersion {
		self.store.protocol_version()
	}

	/// Writes a single key and its `Writeable` value to the provided database key.
	/// Encapsulates serialization using the specified protocol version.
	pub fn put_ser_with_version<W: ser::Writeable>(
		&mut self,
		db_key: Option<u8>,
		key: &[u8],
		value: &W,
		version: ProtocolVersion,
	) -> Result<(), Error> {
		let ser_value = ser::ser_vec(value, version);
		match ser_value {
			Ok(data) => self.put(db_key, key, &data),
			Err(err) => Err(err.into()),
		}
	}

	/// Low-level access for retrieving data by key.
	/// Takes a function for flexible deserialization.
	fn get_with<F, T>(
		&self,
		db_key: Option<u8>,
		key: &[u8],
		deserialize: F,
	) -> Result<Option<T>, Error>
	where
		F: Fn(&[u8], &[u8]) -> Result<T, Error>,
	{
		let read = self.write.nested_read_txn()?;
		self.store.get_with(db_key, key, &read, deserialize)
	}

	/// Whether the provided key exists.
	/// This is in the context of the current write transaction.
	pub fn exists(&self, db_key: Option<u8>, key: &[u8]) -> Result<bool, Error> {
		let read = self.write.nested_read_txn()?;
		let db = self.store.get_db(db_key)?;
		let res = db.get(&read, key)?;
		Ok(res.is_some())
	}

	/// Produces an iterator from the provided database key.
	pub fn iter<F, T>(
		&'a self,
		db_key: Option<u8>,
		deserialize: F,
	) -> Result<DatabaseIterator<'a, F, T>, Error>
	where
		F: Fn(&[u8], &[u8]) -> Result<T, Error>,
	{
		let res = {
			match self.write.nested_read_txn() {
				Ok(read) => {
					let db_res = self.store.get_db(db_key);
					match db_res {
						Ok(db) => {
							DatabaseIterator::new(Arc::new(db.clone()), None, read, deserialize)
						}
						Err(e) => Err(Error::from(e)),
					}
				}
				Err(e) => Err(Error::from(e)),
			}
		};
		res
	}

	/// Gets a `Readable` value from the database by provided key and deserialization strategy.
	pub fn get_ser<T: ser::Readable>(
		&self,
		db_key: Option<u8>,
		key: &[u8],
		deser_mode: Option<DeserializationMode>,
	) -> Result<Option<T>, Error> {
		let d = match deser_mode {
			Some(d) => d,
			_ => DeserializationMode::default(),
		};
		self.get_with(db_key, key, |_, mut data| {
			match ser::deserialize(&mut data, self.protocol_version(), d) {
				Ok(res) => Ok(res),
				Err(e) => Err(From::from(e)),
			}
		})
	}

	/// Deletes a key/value pair from the database.
	pub fn delete(&mut self, db_key: Option<u8>, key: &[u8]) -> Result<(), Error> {
		let db = self.store.get_db(db_key)?;
		db.delete(&mut self.write, key)?;
		Ok(())
	}

	/// Writes the batch to database.
	pub fn commit(self) -> Result<(), Error> {
		self.write.commit()?;
		Ok(())
	}

	/// Creates a child of this batch. It will be merged with its parent on
	/// commit, abandoned otherwise.
	pub fn child(&mut self) -> Result<Batch<'_>, Error> {
		let res = {
			match self.store.env.nested_write_txn(&mut self.write) {
				Ok(write) => Ok(Batch {
					store: self.store,
					write,
					tx_counter: None,
				}),
				Err(e) => Err(Error::from(e)),
			}
		};
		res
	}
}

/// An iterator based on database key.
/// Caller is responsible for deserialization of the data.
pub struct DatabaseIterator<'a, F, T>
where
	F: Fn(&[u8], &[u8]) -> Result<T, Error>,
{
	db: Arc<Database<Bytes, Bytes>>,
	read: Arc<RoTxn<'a, WithoutTls>>,
	keys: Vec<Vec<u8>>,
	skip_cur: usize,
	skip_total: usize,
	done: bool,
	deserialize: F,
	#[allow(dead_code)]
	tx_counter: Option<TxCounter>,
}

impl<F, T> Iterator for DatabaseIterator<'_, F, T>
where
	F: Fn(&[u8], &[u8]) -> Result<T, Error>,
{
	type Item = Result<T, Error>;

	fn next(&mut self) -> Option<Self::Item> {
		loop {
			if self.done {
				return None;
			} else if let Some(k) = self.keys.iter().skip(self.skip_cur).next() {
				self.skip_total += 1;
				self.skip_cur += 1;
				match self.db.get(&self.read, k) {
					Ok(v) => {
						if let Some(v) = v {
							return match (self.deserialize)(k, v) {
								Ok(v) => Some(Ok(v)),
								Err(e) => {
									error!("db iter: error deserializing: {}", e);
									Some(Err(Error::from(e)))
								}
							};
						}
					}
					Err(e) => {
						return {
							error!("db iter: error read value: {}", e);
							Some(Err(Error::from(e)))
						}
					}
				}
			} else if let Err(e) = self.load_next_keys() {
				error!("db iter: error read keys: {}", e);
				self.done = true;
				return Some(Err(e));
			}
		}
	}
}

impl<'a, F, T> DatabaseIterator<'a, F, T>
where
	F: Fn(&[u8], &[u8]) -> Result<T, Error>,
{
	/// Initialize a new prefix iterator.
	pub fn new(
		db: Arc<Database<Bytes, Bytes>>,
		tx_counter: Option<TxCounter>,
		read: RoTxn<'a, WithoutTls>,
		deserialize: F,
	) -> Result<DatabaseIterator<'a, F, T>, Error> {
		// load keys before constructing tx_counter to avoid double-decrementing open_txs_count on error
		let keys = Self::read_key_page(&db, &read, 0)?;
		let done = keys.is_empty();
		Ok(DatabaseIterator {
			db,
			read: Arc::new(read),
			keys,
			skip_cur: 0,
			skip_total: 0,
			done,
			deserialize,
			tx_counter,
		})
	}

	fn load_next_keys(&mut self) -> Result<(), Error> {
		self.keys = Self::read_key_page(&self.db, &self.read, self.skip_total)?;
		self.skip_cur = 0;
		self.done = self.keys.is_empty();
		Ok(())
	}

	fn read_key_page(
		db: &Database<Bytes, Bytes>,
		read: &RoTxn<'a, WithoutTls>,
		skip: usize,
	) -> Result<Vec<Vec<u8>>, Error> {
		let iter = db.iter(read)?;
		iter.move_between_keys()
			.skip(skip)
			.take(10000)
			.map(|kv| kv.map(|(k, _)| k.to_vec()).map_err(Error::from))
			.collect::<Result<Vec<Vec<u8>>, Error>>()
	}
}

/// Get environment size.
fn env_size(env: &Env<WithoutTls>) -> usize {
	let info = env.info();
	let stat = env.stat();
	stat.page_size as usize * info.last_page_number
}

/// Round size proportionally to chunk size.
fn round_size_to_chunk(size: usize, chunk_size: usize) -> usize {
	let rem = size % chunk_size;
	if rem == 0 {
		size
	} else {
		size + (chunk_size - rem)
	}
}

/// Determines whether the environment needs a resize based on a simple percentage threshold.
pub fn needs_resize(env: &Env<WithoutTls>, alloc_chunk_size: usize) -> (bool, usize) {
	let env_info = env.info();
	let size_used = env_size(env);
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
			while size_used as f32 / tot as f32 > RESIZE_MIN_TARGET_PERCENT as f32 / 100.0 {
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
