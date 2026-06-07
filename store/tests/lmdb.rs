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

use grin_core as core;
use grin_store as store;
use grin_util as util;

use core::global;
use core::ser::{self, Readable, Reader, Writeable, Writer};
use store::{
	needs_resize, to_key, to_key_u64, Store, ALLOC_CHUNK_SIZE_DEFAULT_TEST, DEFAULT_ENV_NAME,
};

use byteorder::WriteBytesExt;
use heed::types::Bytes;
use heed::{Database, Env, EnvOpenOptions, WithoutTls};
use std::fs;
use std::path::Path;

const WRITE_CHUNK_SIZE: usize = 20;
const TEST_ALLOC_SIZE: usize = store::lmdb::ALLOC_CHUNK_SIZE_DEFAULT / 8 / WRITE_CHUNK_SIZE;

#[derive(Clone)]
struct PhatChunkStruct {
	phatness: u64,
}

impl PhatChunkStruct {
	/// create
	pub fn new() -> PhatChunkStruct {
		PhatChunkStruct { phatness: 0 }
	}
}

impl Readable for PhatChunkStruct {
	fn read<R: Reader>(reader: &mut R) -> Result<PhatChunkStruct, ser::Error> {
		let mut retval = PhatChunkStruct::new();
		for _ in 0..TEST_ALLOC_SIZE {
			retval.phatness = reader.read_u64()?;
		}
		Ok(retval)
	}
}

impl Writeable for PhatChunkStruct {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		// write many times
		for _ in 0..TEST_ALLOC_SIZE {
			writer.write_u64(self.phatness)?;
		}
		Ok(())
	}
}

fn clean_output_dir(test_dir: &str) {
	let _ = fs::remove_dir_all(test_dir);
}

fn setup(test_dir: &str) {
	global::set_local_chain_type(global::ChainTypes::Mainnet);
	util::init_test_logger();
	clean_output_dir(test_dir);
}

#[test]
fn test_exists() -> Result<(), store::Error> {
	let test_dir = "target/test_exists";
	setup(test_dir);

	let prefix = b'P';
	let store = store::Store::new(test_dir, Some("test1"), None, vec![prefix], None, None)?;

	let key = [0, 0, 0, 1];
	let value = [1, 1, 1, 1];

	// Start new batch and insert a new key/value entry.
	let mut batch = store.batch()?;
	batch.put(Some(prefix), &key, &value)?;

	// Check we can see the new entry in uncommitted batch.
	assert!(batch.exists(Some(prefix), &key)?);

	// Check we cannot see the new entry yet outside of the uncommitted batch.
	assert!(!store.exists(Some(prefix), &key)?);

	batch.commit()?;

	// Check we can see the new entry after committing the batch.
	assert!(store.exists(Some(prefix), &key)?);

	clean_output_dir(test_dir);
	Ok(())
}

#[test]
fn test_iter() -> Result<(), store::Error> {
	let test_dir = "target/test_iter";
	setup(test_dir);

	let prefix = b'P';
	let store = store::Store::new(test_dir, Some("test1"), None, vec![prefix], None, None)?;

	let key = [0, 0, 0, 1];
	let value = [1, 1, 1, 1];

	// Start new batch and insert a new key/value entry.
	let mut batch = store.batch()?;
	batch.put(Some(prefix), &key, &value)?;

	// Check we can see the new entry via an iterator using the uncommitted batch.
	{
		let mut iter = batch.iter(Some(prefix), |_, v| Ok(v.to_vec()))?;
		assert_eq!(iter.next(), Some(Ok(value.to_vec())));
		assert_eq!(iter.next(), None);
	}

	// Check we can not yet see the new entry via an iterator outside the uncommitted batch.
	let mut iter = store.iter(Some(prefix), |_, v| Ok(v.to_vec()))?;
	assert_eq!(iter.next(), None);

	batch.commit()?;

	// Check we can see the new entry via an iterator after committing the batch.
	let mut iter = store.iter(Some(prefix), |_, v| Ok(v.to_vec()))?;
	assert_eq!(iter.next(), Some(Ok(value.to_vec())));
	assert_eq!(iter.next(), None);

	clean_output_dir(test_dir);
	Ok(())
}

#[test]
fn test_iter_pages() -> Result<(), store::Error> {
	let test_dir = "target/test_iter_pages";
	setup(test_dir);

	let prefix = b'P';
	let store = store::Store::new(test_dir, Some("test1"), None, vec![prefix], None, None)?;

	{
		let mut batch = store.batch()?;
		for i in 0..10_001u32 {
			batch.put(Some(prefix), &i.to_be_bytes(), &[1])?;
		}
		batch.commit()?;
	}

	let count = store
		.iter(Some(prefix), |_, v| Ok(v.to_vec()))?
		.collect::<Result<Vec<_>, _>>()?
		.len();
	assert_eq!(count, 10_001);

	clean_output_dir(test_dir);
	Ok(())
}

#[test]
fn lmdb_allocate() -> Result<(), store::Error> {
	let test_dir = "target/lmdb_allocate";
	setup(test_dir);
	let prefix = b'P';
	// Allocate more than the initial chunk, ensuring
	// the DB resizes underneath
	{
		let store = store::Store::new(test_dir, Some("test1"), None, vec![prefix], None, None)?;

		for i in 0..WRITE_CHUNK_SIZE * 2 {
			println!("Allocating chunk: {}", i);
			let chunk = PhatChunkStruct::new();
			let key_val = format!("phat_chunk_set_1_{}", i);
			let mut batch = store.batch()?;
			batch.put_ser(Some(prefix), key_val.as_bytes(), &chunk)?;
			batch.commit()?;
		}
	}
	println!("***********************************");
	println!("***************NEXT*****************");
	println!("***********************************");
	// Open env again and keep adding
	{
		let store = store::Store::new(test_dir, Some("test1"), None, vec![prefix], None, None)?;
		for i in 0..WRITE_CHUNK_SIZE * 2 {
			println!("Allocating chunk: {}", i);
			let chunk = PhatChunkStruct::new();
			let key_val = format!("phat_chunk_set_2_{}", i);
			let mut batch = store.batch()?;
			batch.put_ser(Some(prefix), key_val.as_bytes(), &chunk)?;
			batch.commit()?;
		}
	}

	clean_output_dir(test_dir);
	Ok(())
}

fn create_old_db(
	test_dir: &str,
) -> Result<(Database<Bytes, Bytes>, Env<WithoutTls>), store::Error> {
	let env_name = DEFAULT_ENV_NAME;
	let alloc_chunk_size = ALLOC_CHUNK_SIZE_DEFAULT_TEST;
	let full_path = Path::new(test_dir)
		.join(env_name)
		.to_str()
		.unwrap()
		.to_string();
	let _ = fs::create_dir_all(&full_path);

	let env = unsafe {
		let mut options = EnvOpenOptions::new().read_txn_without_tls();
		let env_options = options.map_size(alloc_chunk_size).max_dbs(1);
		env_options.open(&full_path)?
	};
	let (resize, new_size) = needs_resize(&env, alloc_chunk_size);
	if resize {
		unsafe {
			env.resize(new_size)?;
		};
	}

	let mut write = env.write_txn()?;
	let db: Database<Bytes, Bytes> = env.create_database(&mut write, Some(env_name))?;
	write.commit()?;

	Ok((db, env))
}

#[test]
fn test_migration() -> Result<(), store::Error> {
	let test_dir = "target/test_migration";
	setup(test_dir);

	let test_prefix_1 = b'H';
	let test_key_1 = [0, 1, 2, 4];
	let test_data_1 = [1, 2, 3, 4];

	let test_prefix_2 = b'G';
	let test_key_2 = [3, 4, 5, 6];
	let test_key_64_2 = 65480464;
	let test_data_2 = [4, 5, 6, 7];

	let test_key_3 = [6, 7, 8, 9];
	let test_data_3 = [7, 8, 9, 10];

	// Create old db and fill the data.
	{
		let (old_db, old_env) = create_old_db(test_dir)?;
		let mut w = old_env.write_txn()?;

		// Create old format key value.
		let key_1 = to_key(test_prefix_1, test_key_1);
		old_db.put(&mut w, key_1.as_slice(), test_data_1.as_slice())?;

		// Create old format 64 key value.
		let key_2 = to_key_u64(test_prefix_2, test_key_2, test_key_64_2);
		old_db.put(&mut w, key_2.as_slice(), test_data_2.as_slice())?;

		// Create key value without prefix.
		old_db.put(&mut w, test_key_3.as_slice(), test_data_3.as_slice())?;

		w.commit()?;
	}

	// Create new store to migrate data.
	let store = Store::new(
		test_dir,
		None,
		Some(DEFAULT_ENV_NAME),
		vec![test_prefix_1, test_prefix_2],
		None,
		None,
	)?;

	// Check we can see key value.
	{
		assert!(store.exists(Some(test_prefix_1), &test_key_1)?);
		let data = store.get_ser::<Vec<u8>>(Some(test_prefix_1), &test_key_1, None)?;
		assert_eq!(data, Some(test_data_1.to_vec()));
	}

	// Check we can see key 64 value.
	{
		let mut key = test_key_2.to_vec();
		key.write_u64::<byteorder::BigEndian>(test_key_64_2)
			.unwrap();
		assert!(store.exists(Some(test_prefix_2), &key)?);
		let data = store.get_ser::<Vec<u8>>(Some(test_prefix_2), &key, None)?;
		assert_eq!(data, Some(test_data_2.to_vec()));
	}

	// Check we can see key value without prefix.
	{
		assert!(store.exists(None, &test_key_3)?);
		let data = store.get_ser::<Vec<u8>>(None, &test_key_3, None)?;
		assert_eq!(data, Some(test_data_3.to_vec()));
	}

	clean_output_dir(test_dir);
	Ok(())
}
