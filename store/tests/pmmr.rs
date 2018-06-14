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

extern crate env_logger;
extern crate grin_core as core;
extern crate grin_store as store;
extern crate time;

use std::fs;

use core::core::pmmr::{Backend, PMMR};
use core::ser::{Error, PMMRIndexHashable, PMMRable, Readable, Reader, Writeable, Writer};
use store::types::prune_noop;

#[test]
fn pmmr_append() {
	let (data_dir, elems) = setup("append");
	let mut backend = store::pmmr::PMMRBackend::new(data_dir.to_string()).unwrap();

	// adding first set of 4 elements and sync
	let mut mmr_size = load(0, &elems[0..4], &mut backend);
	backend.sync().unwrap();

	// adding the rest and sync again
	mmr_size = load(mmr_size, &elems[4..9], &mut backend);
	backend.sync().unwrap();

	// check the resulting backend store and the computation of the root
	let node_hash = elems[0].hash_with_index(0);
	assert_eq!(backend.get_hash(1).unwrap(), node_hash);

	// 0010012001001230

	let pos_0 = elems[0].hash_with_index(0);
	let pos_1 = elems[1].hash_with_index(1);
	let pos_2 = (pos_0, pos_1).hash_with_index(2);

	let pos_3 = elems[2].hash_with_index(3);
	let pos_4 = elems[3].hash_with_index(4);
	let pos_5 = (pos_3, pos_4).hash_with_index(5);
	let pos_6 = (pos_2, pos_5).hash_with_index(6);

	let pos_7 = elems[4].hash_with_index(7);
	let pos_8 = elems[5].hash_with_index(8);
	let pos_9 = (pos_7, pos_8).hash_with_index(9);

	let pos_10 = elems[6].hash_with_index(10);
	let pos_11 = elems[7].hash_with_index(11);
	let pos_12 = (pos_10, pos_11).hash_with_index(12);
	let pos_13 = (pos_9, pos_12).hash_with_index(13);
	let pos_14 = (pos_6, pos_13).hash_with_index(14);

	let pos_15 = elems[8].hash_with_index(15);

	{
		let pmmr: PMMR<TestElem, _> = PMMR::at(&mut backend, mmr_size);
		assert_eq!(pmmr.root(), (pos_14, pos_15).hash_with_index(16));
	}

	teardown(data_dir);
}

#[test]
fn pmmr_compact_leaf_sibling() {
	let (data_dir, elems) = setup("compact_leaf_sibling");

	// setup the mmr store with all elements
	let mut backend = store::pmmr::PMMRBackend::new(data_dir.to_string()).unwrap();
	let mmr_size = load(0, &elems[..], &mut backend);
	backend.sync().unwrap();

	// On far left of the MMR -
	// pos 1 and 2 are leaves (and siblings)
	// the parent is pos 3

	let (pos_1_hash, pos_2_hash, pos_3_hash) = {
		let mut pmmr = PMMR::at(&mut backend, mmr_size);
		(
			pmmr.get_hash(1).unwrap(),
			pmmr.get_hash(2).unwrap(),
			pmmr.get_hash(3).unwrap(),
		)
	};

	// prune pos 1
	{
		let mut pmmr = PMMR::at(&mut backend, mmr_size);
		pmmr.prune(1, 1).unwrap();

		// prune pos 8 as well to push the remove list past the cutoff
		pmmr.prune(8, 1).unwrap();
	}
	backend.sync().unwrap();

	// // check pos 1, 2, 3 are in the state we expect after pruning
	{
		let pmmr = PMMR::at(&mut backend, mmr_size);

		// check that pos 1 is "removed"
		assert_eq!(pmmr.get_hash(1), None);

		// check that pos 2 and 3 are unchanged
		assert_eq!(pmmr.get_hash(2).unwrap(), pos_2_hash);
		assert_eq!(pmmr.get_hash(3).unwrap(), pos_3_hash);
	}

	// check we can still retrieve the "removed" element at pos 1
	// from the backend hash file.
	assert_eq!(backend.get_from_file(1).unwrap(), pos_1_hash);

	// aggressively compact the PMMR files
	backend.check_compact(1, 2, &prune_noop).unwrap();

	// check pos 1, 2, 3 are in the state we expect after compacting
	{
		let pmmr = PMMR::at(&mut backend, mmr_size);

		// check that pos 1 is "removed"
		assert_eq!(pmmr.get_hash(1), None);

		// check that pos 2 and 3 are unchanged
		assert_eq!(pmmr.get_hash(2).unwrap(), pos_2_hash);
		assert_eq!(pmmr.get_hash(3).unwrap(), pos_3_hash);
	}

	// Check we can still retrieve the "removed" hash at pos 1 from the hash file.
	// It should still be available even after pruning and compacting.
	assert_eq!(backend.get_from_file(1).unwrap(), pos_1_hash);

	teardown(data_dir);
}

#[test]
fn pmmr_prune_compact() {
	let (data_dir, elems) = setup("prune_compact");

	// setup the mmr store with all elements
	let mut backend = store::pmmr::PMMRBackend::new(data_dir.to_string()).unwrap();
	let mmr_size = load(0, &elems[..], &mut backend);
	backend.sync().unwrap();

	// save the root
	let root = {
		let pmmr: PMMR<TestElem, _> = PMMR::at(&mut backend, mmr_size);
		pmmr.root()
	};

	// pruning some choice nodes
	{
		let mut pmmr: PMMR<TestElem, _> = PMMR::at(&mut backend, mmr_size);
		pmmr.prune(1, 1).unwrap();
		pmmr.prune(4, 1).unwrap();
		pmmr.prune(5, 1).unwrap();
	}
	backend.sync().unwrap();

	// check the root and stored data
	{
		let pmmr: PMMR<TestElem, _> = PMMR::at(&mut backend, mmr_size);
		assert_eq!(root, pmmr.root());
		// check we can still retrieve same element from leaf index 2
		assert_eq!(pmmr.get_data(2).unwrap(), TestElem(2));
		// and the same for leaf index 7
		assert_eq!(pmmr.get_data(11).unwrap(), TestElem(7));
	}

	// compact
	backend.check_compact(2, 2, &prune_noop).unwrap();

	// recheck the root and stored data
	{
		let pmmr: PMMR<TestElem, _> = PMMR::at(&mut backend, mmr_size);
		assert_eq!(root, pmmr.root());
		assert_eq!(pmmr.get_data(2).unwrap(), TestElem(2));
		assert_eq!(pmmr.get_data(11).unwrap(), TestElem(7));
	}

	teardown(data_dir);
}

#[test]
fn pmmr_reload() {
	let (data_dir, elems) = setup("reload");

	// set everything up with an initial backend
	let mut backend = store::pmmr::PMMRBackend::new(data_dir.to_string()).unwrap();

	let mmr_size = load(0, &elems[..], &mut backend);

	// retrieve entries from the hash file for comparison later
	let pos_3_hash = backend.get_hash(3).unwrap();
	let pos_4_hash = backend.get_hash(4).unwrap();
	let pos_5_hash = backend.get_hash(5).unwrap();

	// save the root
	let root = {
		let pmmr: PMMR<TestElem, _> = PMMR::at(&mut backend, mmr_size);
		pmmr.root()
	};

	{
		backend.sync().unwrap();

		// prune a node so we have prune data
		{
			let mut pmmr: PMMR<TestElem, _> = PMMR::at(&mut backend, mmr_size);
			pmmr.prune(1, 1).unwrap();
		}
		backend.sync().unwrap();

		// now check and compact the backend
		backend.check_compact(1, 2, &prune_noop).unwrap();
		backend.sync().unwrap();

		// prune another node to force compact to actually do something
		{
			let mut pmmr: PMMR<TestElem, _> = PMMR::at(&mut backend, mmr_size);
			pmmr.prune(4, 1).unwrap();
			pmmr.prune(2, 1).unwrap();
		}
		backend.sync().unwrap();

		backend.check_compact(1, 2, &prune_noop).unwrap();
		backend.sync().unwrap();

		assert_eq!(backend.unpruned_size().unwrap(), mmr_size);

		// prune some more to get rm log data
		{
			let mut pmmr: PMMR<TestElem, _> = PMMR::at(&mut backend, mmr_size);
			pmmr.prune(5, 1).unwrap();
		}
		backend.sync().unwrap();
		assert_eq!(backend.unpruned_size().unwrap(), mmr_size);
	}

	// create a new backend referencing the data files
	// and check everything still works as expected
	{
		let mut backend = store::pmmr::PMMRBackend::new(data_dir.to_string()).unwrap();
		assert_eq!(backend.unpruned_size().unwrap(), mmr_size);
		{
			let pmmr: PMMR<TestElem, _> = PMMR::at(&mut backend, mmr_size);
			assert_eq!(root, pmmr.root());
		}

		// pos 1 and pos 2 are both removed (via parent pos 3 in prune list)
		assert_eq!(backend.get_hash(1), None);
		assert_eq!(backend.get_hash(2), None);

		// pos 3 is removed (via prune list)
		assert_eq!(backend.get_hash(3), None);

		// pos 4 is removed (via prune list)
		assert_eq!(backend.get_hash(4), None);
		// pos 5 is removed (via rm_log)
		assert_eq!(backend.get_hash(5), None);

		// now check contents of the hash file
		// pos 1 and pos 2 are no longer in the hash file
		assert_eq!(backend.get_from_file(1), None);
		assert_eq!(backend.get_from_file(2), None);

		// pos 3 is still in there
		assert_eq!(backend.get_from_file(3), Some(pos_3_hash));

		// pos 4 and pos 5 are also still in there
		assert_eq!(backend.get_from_file(4), Some(pos_4_hash));
		assert_eq!(backend.get_from_file(5), Some(pos_5_hash));
	}

	teardown(data_dir);
}

#[test]
fn pmmr_rewind() {
	let (data_dir, elems) = setup("rewind");
	let mut backend = store::pmmr::PMMRBackend::new(data_dir.clone()).unwrap();

	// adding elements and keeping the corresponding root
	let mut mmr_size = load(0, &elems[0..4], &mut backend);
	backend.sync().unwrap();
	let root1 = {
		let pmmr: PMMR<TestElem, _> = PMMR::at(&mut backend, mmr_size);
		pmmr.root()
	};

	mmr_size = load(mmr_size, &elems[4..6], &mut backend);
	backend.sync().unwrap();
	let root2 = {
		let pmmr: PMMR<TestElem, _> = PMMR::at(&mut backend, mmr_size);
		pmmr.root()
	};

	mmr_size = load(mmr_size, &elems[6..9], &mut backend);
	backend.sync().unwrap();

	// prune the first 4 elements (leaves at pos 1, 2, 4, 5)
	{
		let mut pmmr: PMMR<TestElem, _> = PMMR::at(&mut backend, mmr_size);
		pmmr.prune(1, 1).unwrap();
		pmmr.prune(2, 1).unwrap();
		pmmr.prune(4, 1).unwrap();
		pmmr.prune(5, 1).unwrap();
	}
	backend.sync().unwrap();

	// and compact the MMR to remove the 2 pruned elements
	backend.check_compact(2, 2, &prune_noop).unwrap();
	backend.sync().unwrap();

	// rewind and check the roots still match
	{
		let mut pmmr: PMMR<TestElem, _> = PMMR::at(&mut backend, mmr_size);
		pmmr.rewind(9, 3).unwrap();
		assert_eq!(pmmr.root(), root2);
	}
	backend.sync().unwrap();
	{
		let pmmr: PMMR<TestElem, _> = PMMR::at(&mut backend, 10);
		assert_eq!(pmmr.root(), root2);
	}

	// also check the data file looks correct
	// everything up to and including pos 7 should be pruned from the data file
	// except the data at pos 8 and 9 (corresponding to elements 5 and 6)
	for pos in 1..8 {
		assert_eq!(backend.get_data(pos), None);
	}
	assert_eq!(backend.get_data(8), Some(elems[4]));
	assert_eq!(backend.get_data(9), Some(elems[5]));

	assert_eq!(backend.data_size().unwrap(), 2);

	{
		let mut pmmr: PMMR<TestElem, _> = PMMR::at(&mut backend, 10);
		pmmr.rewind(5, 3).unwrap();
		assert_eq!(pmmr.root(), root1);
	}
	backend.sync().unwrap();
	{
		let pmmr: PMMR<TestElem, _> = PMMR::at(&mut backend, 7);
		assert_eq!(pmmr.root(), root1);
	}

	// also check the data file looks correct
	// everything up to and including pos 7 should be pruned from the data file
	// but we have rewound to pos 5 so everything after that should be None
	for pos in 1..10 {
		assert_eq!(backend.get_data(pos), None);
	}

	// check we have no data in the backend after
	// pruning, compacting and rewinding
	assert_eq!(backend.data_size().unwrap(), 0);

	teardown(data_dir);
}

#[test]
fn pmmr_compact_single_leaves() {
	let (data_dir, elems) = setup("compact_single_leaves");
	let mut backend = store::pmmr::PMMRBackend::new(data_dir.clone()).unwrap();
	let mmr_size = load(0, &elems[0..5], &mut backend);
	backend.sync().unwrap();

	{
		let mut pmmr: PMMR<TestElem, _> = PMMR::at(&mut backend, mmr_size);
		pmmr.prune(1, 1).unwrap();
		pmmr.prune(4, 1).unwrap();
	}

	backend.sync().unwrap();

	// compact
	backend.check_compact(2, 2, &prune_noop).unwrap();

	{
		let mut pmmr: PMMR<TestElem, _> = PMMR::at(&mut backend, mmr_size);
		pmmr.prune(2, 2).unwrap();
		pmmr.prune(5, 2).unwrap();
	}

	backend.sync().unwrap();

	// compact
	backend.check_compact(2, 3, &prune_noop).unwrap();

	teardown(data_dir);
}

#[test]
fn pmmr_compact_entire_peak() {
	let (data_dir, elems) = setup("compact_entire_peak");
	let mut backend = store::pmmr::PMMRBackend::new(data_dir.clone()).unwrap();
	let mmr_size = load(0, &elems[0..5], &mut backend);
	backend.sync().unwrap();

	let pos_7_hash = backend.get_hash(7).unwrap();

	let pos_8 = backend.get_data(8).unwrap();
	let pos_8_hash = backend.get_hash(8).unwrap();

	// prune all leaves under the peak at pos 7
	{
		let mut pmmr: PMMR<TestElem, _> = PMMR::at(&mut backend, mmr_size);
		pmmr.prune(1, 1).unwrap();
		pmmr.prune(2, 1).unwrap();
		pmmr.prune(4, 1).unwrap();
		pmmr.prune(5, 1).unwrap();
	}

	backend.sync().unwrap();

	// compact
	backend.check_compact(2, 2, &prune_noop).unwrap();

	// now check we have pruned up to and including the peak at pos 7
	// hash still available in underlying hash file
	assert_eq!(backend.get_hash(7), None);
	assert_eq!(backend.get_from_file(7), Some(pos_7_hash));

	// now check we still have subsequent hash and data where we expect
	assert_eq!(backend.get_hash(8), Some(pos_8_hash));
	assert_eq!(backend.get_data(8), Some(pos_8));
	assert_eq!(backend.get_from_file(8), Some(pos_8_hash));

	teardown(data_dir);
}

#[test]
fn pmmr_compact_horizon() {
	let (data_dir, elems) = setup("compact_horizon");
	let mut backend = store::pmmr::PMMRBackend::new(data_dir.clone()).unwrap();
	let mmr_size = load(0, &elems[..], &mut backend);
	backend.sync().unwrap();

	// 0010012001001230
	// 9 leaves
	assert_eq!(backend.data_size().unwrap(), 19);
	assert_eq!(backend.hash_size().unwrap(), 35);

	let pos_3_hash = backend.get_hash(3).unwrap();

	let pos_6_hash = backend.get_hash(6).unwrap();

	let pos_7_hash = backend.get_hash(7).unwrap();

	let pos_8 = backend.get_data(8).unwrap();
	let pos_8_hash = backend.get_hash(8).unwrap();

	let pos_11 = backend.get_data(11).unwrap();
	let pos_11_hash = backend.get_hash(11).unwrap();

	{
		// pruning some choice nodes with an increasing block height
		{
			let mut pmmr: PMMR<TestElem, _> = PMMR::at(&mut backend, mmr_size);
			pmmr.prune(4, 1).unwrap();
			pmmr.prune(5, 2).unwrap();
			pmmr.prune(1, 3).unwrap();
			pmmr.prune(2, 4).unwrap();
		}
		backend.sync().unwrap();

		// check we can read hashes and data correctly after pruning
		{
			assert_eq!(backend.get_hash(3), None);
			assert_eq!(backend.get_from_file(3), Some(pos_3_hash));

			assert_eq!(backend.get_hash(6), None);
			assert_eq!(backend.get_from_file(6), Some(pos_6_hash));

			assert_eq!(backend.get_hash(7), None);
			assert_eq!(backend.get_from_file(7), Some(pos_7_hash));

			assert_eq!(backend.get_hash(8), Some(pos_8_hash));
			assert_eq!(backend.get_data(8), Some(pos_8));
			assert_eq!(backend.get_from_file(8), Some(pos_8_hash));

			assert_eq!(backend.get_hash(11), Some(pos_11_hash));
			assert_eq!(backend.get_data(11), Some(pos_11));
			assert_eq!(backend.get_from_file(11), Some(pos_11_hash));
		}

		// compact
		backend.check_compact(2, 3, &prune_noop).unwrap();
		backend.sync().unwrap();

		// check we can read a hash by pos correctly after compaction
		{
			assert_eq!(backend.get_hash(3), None);
			assert_eq!(backend.get_from_file(3), Some(pos_3_hash));

			assert_eq!(backend.get_hash(6), None);
			assert_eq!(backend.get_from_file(6), Some(pos_6_hash));

			assert_eq!(backend.get_hash(7), None);
			assert_eq!(backend.get_from_file(7), Some(pos_7_hash));

			assert_eq!(backend.get_hash(8), Some(pos_8_hash));
			assert_eq!(backend.get_from_file(8), Some(pos_8_hash));
		}
	}

	// recheck stored data
	{
		// recreate backend
		let backend = store::pmmr::PMMRBackend::<TestElem>::new(data_dir.to_string()).unwrap();

		assert_eq!(backend.data_size().unwrap(), 17);
		assert_eq!(backend.hash_size().unwrap(), 33);

		// check we can read a hash by pos correctly from recreated backend
		assert_eq!(backend.get_hash(7), None);
		assert_eq!(backend.get_from_file(7), Some(pos_7_hash));

		assert_eq!(backend.get_hash(8), Some(pos_8_hash));
		assert_eq!(backend.get_from_file(8), Some(pos_8_hash));
	}

	{
		let mut backend = store::pmmr::PMMRBackend::<TestElem>::new(data_dir.to_string()).unwrap();

		{
			let mut pmmr: PMMR<TestElem, _> = PMMR::at(&mut backend, mmr_size);

			pmmr.prune(8, 5).unwrap();
			pmmr.prune(9, 5).unwrap();
		}

		// compact some more
		backend.check_compact(1, 6, &prune_noop).unwrap();
	}

	// recheck stored data
	{
		// recreate backend
		let backend = store::pmmr::PMMRBackend::<TestElem>::new(data_dir.to_string()).unwrap();

		// 0010012001001230

		assert_eq!(backend.data_size().unwrap(), 15);
		assert_eq!(backend.hash_size().unwrap(), 29);

		// check we can read a hash by pos correctly from recreated backend
		assert_eq!(backend.get_hash(7), None);
		assert_eq!(backend.get_from_file(7), Some(pos_7_hash));

		assert_eq!(backend.get_hash(11), Some(pos_11_hash));
		assert_eq!(backend.get_data(11), Some(pos_11));
		assert_eq!(backend.get_from_file(11), Some(pos_11_hash));
	}

	teardown(data_dir);
}

#[test]
fn compact_twice() {
	let (data_dir, elems) = setup("compact_twice");

	// setup the mmr store with all elements
	let mut backend = store::pmmr::PMMRBackend::new(data_dir.to_string()).unwrap();
	let mmr_size = load(0, &elems[..], &mut backend);
	backend.sync().unwrap();

	// save the root
	let root = {
		let pmmr: PMMR<TestElem, _> = PMMR::at(&mut backend, mmr_size);
		pmmr.root()
	};

	// pruning some choice nodes
	{
		let mut pmmr: PMMR<TestElem, _> = PMMR::at(&mut backend, mmr_size);
		pmmr.prune(1, 1).unwrap();
		pmmr.prune(2, 1).unwrap();
		pmmr.prune(4, 1).unwrap();
	}
	backend.sync().unwrap();

	// check the root and stored data
	{
		let pmmr: PMMR<TestElem, _> = PMMR::at(&mut backend, mmr_size);
		assert_eq!(root, pmmr.root());
		assert_eq!(pmmr.get_data(5).unwrap(), TestElem(4));
		assert_eq!(pmmr.get_data(11).unwrap(), TestElem(7));
	}

	// compact
	backend.check_compact(2, 2, &prune_noop).unwrap();

	// recheck the root and stored data
	{
		let pmmr: PMMR<TestElem, _> = PMMR::at(&mut backend, mmr_size);
		assert_eq!(root, pmmr.root());
		assert_eq!(pmmr.get_data(5).unwrap(), TestElem(4));
		assert_eq!(pmmr.get_data(11).unwrap(), TestElem(7));
	}

	// now prune some more nodes
	{
		let mut pmmr: PMMR<TestElem, _> = PMMR::at(&mut backend, mmr_size);
		pmmr.prune(5, 2).unwrap();
		pmmr.prune(8, 2).unwrap();
		pmmr.prune(9, 2).unwrap();
	}
	backend.sync().unwrap();

	// recheck the root and stored data
	{
		let pmmr: PMMR<TestElem, _> = PMMR::at(&mut backend, mmr_size);
		assert_eq!(root, pmmr.root());
		assert_eq!(pmmr.get_data(11).unwrap(), TestElem(7));
	}

	// compact
	backend.check_compact(2, 3, &prune_noop).unwrap();

	// recheck the root and stored data
	{
		let pmmr: PMMR<TestElem, _> = PMMR::at(&mut backend, mmr_size);
		assert_eq!(root, pmmr.root());
		assert_eq!(pmmr.get_data(11).unwrap(), TestElem(7));
	}

	teardown(data_dir);
}

fn setup(tag: &str) -> (String, Vec<TestElem>) {
	match env_logger::try_init() {
		Ok(_) => println!("Initializing env logger"),
		Err(e) => println!("env logger already initialized: {:?}", e),
	};
	let t = time::get_time();
	let data_dir = format!("./target/tmp/{}.{}-{}", t.sec, t.nsec, tag);
	fs::create_dir_all(data_dir.clone()).unwrap();

	let mut elems = vec![];
	for x in 1..20 {
		elems.push(TestElem(x));
	}
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
struct TestElem(u32);

impl PMMRable for TestElem {
	fn len() -> usize {
		4
	}
}

impl Writeable for TestElem {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), Error> {
		writer.write_u32(self.0)
	}
}
impl Readable for TestElem {
	fn read(reader: &mut Reader) -> Result<TestElem, Error> {
		Ok(TestElem(reader.read_u32()?))
	}
}
