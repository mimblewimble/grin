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
use core::core::pmmr::{Backend, HashSum, Summable, PMMR};
use core::core::hash::Hashed;

#[test]
fn sumtree_append() {
	let (data_dir, elems) = setup();
	let mut backend = store::sumtree::PMMRBackend::new(data_dir).unwrap();

	// adding first set of 4 elements and sync
	let mut mmr_size = load(0, &elems[0..4], &mut backend);
	backend.sync().unwrap();

	// adding the rest and sync again
	mmr_size = load(mmr_size, &elems[4..9], &mut backend);
	backend.sync().unwrap();

	// check the resulting backend store and the computation of the root
	let hash = Hashed::hash(&elems[0].clone());
	let sum = elems[0].sum();
	let node_hash = (1 as u64, &sum, hash).hash();
	assert_eq!(
		backend.get(1),
		Some(HashSum {
			hash: node_hash,
			sum: sum,
		})
	);

	let sum2 = HashSum::from_summable(1, &elems[0], None::<TestElem>)
		+ HashSum::from_summable(2, &elems[1], None::<TestElem>);
	let sum4 = sum2
		+ (HashSum::from_summable(4, &elems[2], None::<TestElem>)
			+ HashSum::from_summable(5, &elems[3], None::<TestElem>));
	let sum8 = sum4
		+ ((HashSum::from_summable(8, &elems[4], None::<TestElem>)
			+ HashSum::from_summable(9, &elems[5], None::<TestElem>))
			+ (HashSum::from_summable(11, &elems[6], None::<TestElem>)
				+ HashSum::from_summable(12, &elems[7], None::<TestElem>)));
	let sum9 = sum8 + HashSum::from_summable(16, &elems[8], None::<TestElem>);

	{
		let pmmr = PMMR::at(&mut backend, mmr_size);
		assert_eq!(pmmr.root(), sum9);
	}
}

#[test]
fn sumtree_prune_compact() {
	let (data_dir, elems) = setup();

	// setup the mmr store with all elements
	let mut backend = store::sumtree::PMMRBackend::new(data_dir).unwrap();
	let mmr_size = load(0, &elems[..], &mut backend);
	backend.sync().unwrap();

	// save the root
	let root: HashSum<TestElem>;
	{
		let pmmr = PMMR::at(&mut backend, mmr_size);
		root = pmmr.root();
	}

	// pruning some choice nodes
	{
		let mut pmmr = PMMR::at(&mut backend, mmr_size);
		pmmr.prune(1, 1).unwrap();
		pmmr.prune(4, 1).unwrap();
		pmmr.prune(5, 1).unwrap();
	}
	backend.sync().unwrap();

	// check the root
	{
		let pmmr = PMMR::at(&mut backend, mmr_size);
		assert_eq!(root, pmmr.root());
	}

	// compact
	backend.check_compact(2).unwrap();

	// recheck the root
	{
		let pmmr = PMMR::at(&mut backend, mmr_size);
		assert_eq!(root, pmmr.root());
	}
}

#[test]
fn sumtree_reload() {
	let (data_dir, elems) = setup();

	// set everything up with a first backend
	let mmr_size: u64;
	let root: HashSum<TestElem>;
	{
		let mut backend = store::sumtree::PMMRBackend::new(data_dir.clone()).unwrap();
		mmr_size = load(0, &elems[..], &mut backend);
		backend.sync().unwrap();

		// save the root and prune some nodes so we have prune data
		{
			let mut pmmr = PMMR::at(&mut backend, mmr_size);
			root = pmmr.root();
			pmmr.prune(1, 1).unwrap();
			pmmr.prune(4, 1).unwrap();
		}
		backend.sync().unwrap();
		backend.check_compact(1).unwrap();
		backend.sync().unwrap();
    assert_eq!(backend.unpruned_size().unwrap(), mmr_size);

		// prune some more to get rm log data
		{
			let mut pmmr = PMMR::at(&mut backend, mmr_size);
			pmmr.prune(5, 1).unwrap();
		}
		backend.sync().unwrap();
    assert_eq!(backend.unpruned_size().unwrap(), mmr_size);
	}

	// create a new backend and check everything is kosher
	{
		let mut backend = store::sumtree::PMMRBackend::new(data_dir).unwrap();
    assert_eq!(backend.unpruned_size().unwrap(), mmr_size);
		{
			let pmmr = PMMR::at(&mut backend, mmr_size);
			assert_eq!(root, pmmr.root());
		}
		assert_eq!(backend.get(5), None);
	}
}

#[test]
fn sumtree_rewind() {
	let (data_dir, elems) = setup();
	let mut backend = store::sumtree::PMMRBackend::new(data_dir).unwrap();

	// adding elements and keeping the corresponding root
	let mut mmr_size = load(0, &elems[0..4], &mut backend);
	backend.sync().unwrap();
	let root1: HashSum<TestElem>;
	{
		let pmmr = PMMR::at(&mut backend, mmr_size);
		root1 = pmmr.root();
	}

	mmr_size = load(mmr_size, &elems[4..6], &mut backend);
	backend.sync().unwrap();
	let root2: HashSum<TestElem>;
	{
		let pmmr = PMMR::at(&mut backend, mmr_size);
		root2 = pmmr.root();
	}

	mmr_size = load(mmr_size, &elems[6..9], &mut backend);
	backend.sync().unwrap();

	// prune and compact the 2 first elements to spice things up
	{
		let mut pmmr = PMMR::at(&mut backend, mmr_size);
		pmmr.prune(1, 1).unwrap();
		pmmr.prune(2, 1).unwrap();
	}
	backend.check_compact(1).unwrap();
	backend.sync().unwrap();

	// rewind and check the roots still match
	{
		let mut pmmr = PMMR::at(&mut backend, mmr_size);
		pmmr.rewind(9, 3).unwrap();
		assert_eq!(pmmr.root(), root2);
	}
	backend.sync().unwrap();
	{
		let pmmr = PMMR::at(&mut backend, 10);
		assert_eq!(pmmr.root(), root2);
	}

	{
		let mut pmmr = PMMR::at(&mut backend, 10);
		pmmr.rewind(5, 3).unwrap();
		assert_eq!(pmmr.root(), root1);
	}
	backend.sync().unwrap();
	{
		let pmmr = PMMR::at(&mut backend, 7);
		assert_eq!(pmmr.root(), root1);
	}
}

fn setup() -> (String, Vec<TestElem>) {
	let _ = env_logger::init();
	let t = time::get_time();
	let data_dir = format!("./target/{}.{}", t.sec, t.nsec);
	fs::create_dir_all(data_dir.clone()).unwrap();

	let elems = vec![
		TestElem([0, 0, 0, 1]),
		TestElem([0, 0, 0, 2]),
		TestElem([0, 0, 0, 3]),
		TestElem([0, 0, 0, 4]),
		TestElem([0, 0, 0, 5]),
		TestElem([0, 0, 0, 6]),
		TestElem([0, 0, 0, 7]),
		TestElem([0, 0, 0, 8]),
		TestElem([1, 0, 0, 0]),
	];
	(data_dir, elems)
}

fn load(pos: u64, elems: &[TestElem], backend: &mut store::sumtree::PMMRBackend<TestElem>) -> u64 {
	let mut pmmr = PMMR::at(backend, pos);
	for elem in elems {
		pmmr.push(elem.clone(), None::<TestElem>).unwrap();
	}
	pmmr.unpruned_size()
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct TestElem([u32; 4]);
impl Summable for TestElem {
	type Sum = u64;
	fn sum(&self) -> u64 {
		// sums are not allowed to overflow, so we use this simple
  // non-injective "sum" function that will still be homomorphic
		self.0[0] as u64 * 0x1000 + self.0[1] as u64 * 0x100 + self.0[2] as u64 * 0x10
			+ self.0[3] as u64
	}
	fn sum_len() -> usize {
		8
	}
}

impl Writeable for TestElem {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), Error> {
		try!(writer.write_u32(self.0[0]));
		try!(writer.write_u32(self.0[1]));
		try!(writer.write_u32(self.0[2]));
		writer.write_u32(self.0[3])
	}
}
