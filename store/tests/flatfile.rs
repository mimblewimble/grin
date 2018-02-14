// Copyright 2017 The Grin Developers
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

extern crate env_logger;
extern crate grin_core as core;
extern crate grin_store as store;
extern crate time;

use std::fs;

use core::ser::*;
use core::core::pmmr::{PMMR, Backend};
use store::flatfile::FlatFileStore;

#[test]
fn flatfile_append() {
	let (data_dir, elems) = setup("flatfile_append");
	let mut flat_file_store:FlatFileStore<TestElem> = FlatFileStore::new(data_dir.to_string(), 16).unwrap();

	// adding test set of 9 elements and sync
	let result = flat_file_store.append(0, elems.clone());
	flat_file_store.sync().unwrap();

	// Read back
	let mut flat_file_store:FlatFileStore<TestElem> = FlatFileStore::new(data_dir.to_string(), 16).unwrap();
	let r = flat_file_store.get(0);
	assert_eq!(r.unwrap(), elems[0]);
	let r = flat_file_store.get(8);
	assert_eq!(r.unwrap(), elems[8]);

	// Remove some elements from the store
	let _ = flat_file_store.remove(vec![0]);
	flat_file_store.sync().unwrap();

	let r = flat_file_store.get(0);
	assert_eq!(r.unwrap(), elems[1]);

	let _ = flat_file_store.remove(vec![4]);
	let r = flat_file_store.get(0);
	assert_eq!(r.unwrap(), elems[1]);
	let r = flat_file_store.get(1);
	assert_eq!(r.unwrap(), elems[2]);
	let r = flat_file_store.get(2);
	assert_eq!(r.unwrap(), elems[3]);
	let r = flat_file_store.get(3);
	assert_eq!(r.unwrap(), elems[4]);
	let r = flat_file_store.get(4);
	assert_eq!(r.unwrap(), elems[6]);
	let r = flat_file_store.get(6);
	assert_eq!(r.unwrap(), elems[8]);
	let r = flat_file_store.get(7);
	assert_eq!(r, None);

	flat_file_store.sync().unwrap();
	let _ = flat_file_store.remove(vec![6]);
	let r = flat_file_store.get(6);
	assert_eq!(r, None);

	let r = flat_file_store.get(5);
	assert_eq!(r.unwrap(), elems[7]);

	flat_file_store.sync().unwrap();
	let _ = flat_file_store.check_compact(1);

	//All should be the same as before compaction
	let r = flat_file_store.get(0);
	assert_eq!(r.unwrap(), elems[1]);
	let r = flat_file_store.get(1);
	assert_eq!(r.unwrap(), elems[2]);
	let r = flat_file_store.get(2);
	assert_eq!(r.unwrap(), elems[3]);
	let r = flat_file_store.get(3);
	assert_eq!(r.unwrap(), elems[4]);
	let r = flat_file_store.get(4);
	assert_eq!(r.unwrap(), elems[6]);
	let r = flat_file_store.get(5);
	assert_eq!(r.unwrap(), elems[7]);
	let r = flat_file_store.get(6);
	assert_eq!(r, None);
	teardown(data_dir);
}

fn setup(tag: &str) -> (String, Vec<TestElem>) {
	let _ = env_logger::init();
	let t = time::get_time();
	let data_dir = format!("./target/{}.{}-{}", t.sec, t.nsec, tag);
	fs::create_dir_all(data_dir.clone()).unwrap();

	let elems = vec![
		TestElem([0, 0, 0, 0]),
		TestElem([0, 0, 0, 1]),
		TestElem([0, 0, 0, 2]),
		TestElem([0, 0, 0, 3]),
		TestElem([0, 0, 0, 4]),
		TestElem([0, 0, 0, 5]),
		TestElem([0, 0, 0, 6]),
		TestElem([0, 0, 0, 7]),
		TestElem([0, 0, 0, 8]),
	];
	(data_dir, elems)
}

fn teardown(data_dir: String) {
	fs::remove_dir_all(data_dir).unwrap();
}

fn load(pos: u64, elems: &[TestElem], backend: &mut store::pmmr::PMMRBackend<TestElem>) -> u64 {
	let mut pmmr = PMMR::at(backend, pos);
	for elem in elems {
		pmmr.push(elem.clone()).unwrap();
	}
	pmmr.unpruned_size()
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct TestElem([u32; 4]);
impl Writeable for TestElem {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), Error> {
		try!(writer.write_u32(self.0[0]));
		try!(writer.write_u32(self.0[1]));
		try!(writer.write_u32(self.0[2]));
		writer.write_u32(self.0[3])
	}
}
impl Readable for TestElem {
	fn read(reader: &mut Reader) -> Result<TestElem, Error> {
		Ok(TestElem (
			[
				reader.read_u32()?,
				reader.read_u32()?,
				reader.read_u32()?,
				reader.read_u32()?,
			]
		))
	}
}
