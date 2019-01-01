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

use env_logger;
use grin_core as core;
use grin_store as store;

use std::fs;

use chrono::prelude::Utc;
use croaring::Bitmap;

use crate::core::core::pmmr::{Backend, PMMR};
use crate::core::ser::{
	Error, FixedLength, PMMRIndexHashable, PMMRable, Readable, Reader, Writeable, Writer,
};
use crate::store::types::prune_noop;

#[test]
fn pmmr_append() {
	let (data_dir, elems) = setup("append");
	let mut backend = store::pmmr::PMMRBackend::new(data_dir.to_string(), true, None).unwrap();

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
		let pmmr: PMMR<'_, TestElem, _> = PMMR::at(&mut backend, mmr_size);
		assert_eq!(pmmr.root(), (pos_14, pos_15).hash_with_index(16));
	}

	teardown(data_dir);
}

#[test]
fn pmmr_compact_leaf_sibling() {
	let (data_dir, elems) = setup("compact_leaf_sibling");

	// setup the mmr store with all elements
	let mut backend = store::pmmr::PMMRBackend::new(data_dir.to_string(), true, None).unwrap();
	let mmr_size = load(0, &elems[..], &mut backend);
	backend.sync().unwrap();

	// On far left of the MMR -
	// pos 1 and 2 are leaves (and siblings)
	// the parent is pos 3

	let (pos_1_hash, pos_2_hash, pos_3_hash) = {
		let pmmr = PMMR::at(&mut backend, mmr_size);
		(
			pmmr.get_hash(1).unwrap(),
			pmmr.get_hash(2).unwrap(),
			pmmr.get_hash(3).unwrap(),
		)
	};

	// prune pos 1
	{
		let mut pmmr = PMMR::at(&mut backend, mmr_size);
		pmmr.prune(1).unwrap();

		// prune pos 8 as well to push the remove list past the cutoff
		pmmr.prune(8).unwrap();
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
	backend
		.check_compact(1, &Bitmap::create(), &prune_noop)
		.unwrap();

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
	let mut backend = store::pmmr::PMMRBackend::new(data_dir.to_string(), true, None).unwrap();
	let mmr_size = load(0, &elems[..], &mut backend);
	backend.sync().unwrap();

	// save the root
	let root = {
		let pmmr: PMMR<'_, TestElem, _> = PMMR::at(&mut backend, mmr_size);
		pmmr.root()
	};

	// pruning some choice nodes
	{
		let mut pmmr: PMMR<'_, TestElem, _> = PMMR::at(&mut backend, mmr_size);
		pmmr.prune(1).unwrap();
		pmmr.prune(4).unwrap();
		pmmr.prune(5).unwrap();
	}
	backend.sync().unwrap();

	// check the root and stored data
	{
		let pmmr: PMMR<'_, TestElem, _> = PMMR::at(&mut backend, mmr_size);
		assert_eq!(root, pmmr.root());
		// check we can still retrieve same element from leaf index 2
		assert_eq!(pmmr.get_data(2).unwrap(), TestElem(2));
		// and the same for leaf index 7
		assert_eq!(pmmr.get_data(11).unwrap(), TestElem(7));
	}

	// compact
	backend
		.check_compact(2, &Bitmap::create(), &prune_noop)
		.unwrap();

	// recheck the root and stored data
	{
		let pmmr: PMMR<'_, TestElem, _> = PMMR::at(&mut backend, mmr_size);
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
	let mut backend = store::pmmr::PMMRBackend::new(data_dir.to_string(), true, None).unwrap();

	let mmr_size = load(0, &elems[..], &mut backend);

	// retrieve entries from the hash file for comparison later
	let pos_3_hash = backend.get_hash(3).unwrap();
	let pos_4_hash = backend.get_hash(4).unwrap();
	let pos_5_hash = backend.get_hash(5).unwrap();

	// save the root
	let root = {
		let pmmr: PMMR<'_, TestElem, _> = PMMR::at(&mut backend, mmr_size);
		pmmr.root()
	};

	{
		backend.sync().unwrap();

		// prune a node so we have prune data
		{
			let mut pmmr: PMMR<'_, TestElem, _> = PMMR::at(&mut backend, mmr_size);
			pmmr.prune(1).unwrap();
		}
		backend.sync().unwrap();

		// now check and compact the backend
		backend
			.check_compact(1, &Bitmap::create(), &prune_noop)
			.unwrap();
		backend.sync().unwrap();

		// prune another node to force compact to actually do something
		{
			let mut pmmr: PMMR<'_, TestElem, _> = PMMR::at(&mut backend, mmr_size);
			pmmr.prune(4).unwrap();
			pmmr.prune(2).unwrap();
		}
		backend.sync().unwrap();

		backend
			.check_compact(4, &Bitmap::create(), &prune_noop)
			.unwrap();
		backend.sync().unwrap();

		assert_eq!(backend.unpruned_size(), mmr_size);

		// prune some more to get rm log data
		{
			let mut pmmr: PMMR<'_, TestElem, _> = PMMR::at(&mut backend, mmr_size);
			pmmr.prune(5).unwrap();
		}
		backend.sync().unwrap();
		assert_eq!(backend.unpruned_size(), mmr_size);
	}

	// create a new backend referencing the data files
	// and check everything still works as expected
	{
		let mut backend = store::pmmr::PMMRBackend::new(data_dir.to_string(), true, None).unwrap();
		assert_eq!(backend.unpruned_size(), mmr_size);
		{
			let pmmr: PMMR<'_, TestElem, _> = PMMR::at(&mut backend, mmr_size);
			assert_eq!(root, pmmr.root());
		}

		// pos 1 and pos 2 are both removed (via parent pos 3 in prune list)
		assert_eq!(backend.get_hash(1), None);
		assert_eq!(backend.get_hash(2), None);

		// pos 3 is "removed" but we keep the hash around for root of pruned subtree
		assert_eq!(backend.get_hash(3), Some(pos_3_hash));

		// pos 4 is removed (via prune list)
		assert_eq!(backend.get_hash(4), None);
		// pos 5 is removed (via leaf_set)
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
	let mut backend = store::pmmr::PMMRBackend::new(data_dir.clone(), true, None).unwrap();

	// adding elements and keeping the corresponding root
	let mut mmr_size = load(0, &elems[0..4], &mut backend);
	backend.sync().unwrap();
	let root1 = {
		let pmmr: PMMR<'_, TestElem, _> = PMMR::at(&mut backend, mmr_size);
		pmmr.root()
	};

	mmr_size = load(mmr_size, &elems[4..6], &mut backend);
	backend.sync().unwrap();
	let root2 = {
		let pmmr: PMMR<'_, TestElem, _> = PMMR::at(&mut backend, mmr_size);
		assert_eq!(pmmr.unpruned_size(), 10);
		pmmr.root()
	};

	mmr_size = load(mmr_size, &elems[6..9], &mut backend);
	backend.sync().unwrap();
	let root3 = {
		let pmmr: PMMR<'_, TestElem, _> = PMMR::at(&mut backend, mmr_size);
		assert_eq!(pmmr.unpruned_size(), 16);
		pmmr.root()
	};

	// prune the first 4 elements (leaves at pos 1, 2, 4, 5)
	{
		let mut pmmr: PMMR<'_, TestElem, _> = PMMR::at(&mut backend, mmr_size);
		pmmr.prune(1).unwrap();
		pmmr.prune(2).unwrap();
		pmmr.prune(4).unwrap();
		pmmr.prune(5).unwrap();
	}
	backend.sync().unwrap();

	println!("before compacting - ");
	for x in 1..17 {
		println!("pos {}, {:?}", x, backend.get_from_file(x));
	}

	// and compact the MMR to remove the pruned elements
	backend
		.check_compact(6, &Bitmap::create(), &prune_noop)
		.unwrap();
	backend.sync().unwrap();

	println!("after compacting - ");
	for x in 1..17 {
		println!("pos {}, {:?}", x, backend.get_from_file(x));
	}

	println!("root1 {:?}, root2 {:?}, root3 {:?}", root1, root2, root3);

	// rewind and check the roots still match
	{
		let mut pmmr: PMMR<'_, TestElem, _> = PMMR::at(&mut backend, mmr_size);
		pmmr.rewind(9, &Bitmap::of(&vec![11, 12, 16])).unwrap();
		assert_eq!(pmmr.unpruned_size(), 10);

		// assert_eq!(pmmr.root(), root2);
	}
	println!("after rewinding - ");
	for x in 1..17 {
		println!("pos {}, {:?}", x, backend.get_from_file(x));
	}

	println!("doing a sync after rewinding");
	backend.sync().unwrap();

	{
		let pmmr: PMMR<'_, TestElem, _> = PMMR::at(&mut backend, 10);
		assert_eq!(pmmr.root(), root2);
	}

	// Also check the data file looks correct.
	// pos 1, 2, 4, 5 are all leaves but these have been pruned.
	for pos in vec![1, 2, 4, 5] {
		assert_eq!(backend.get_data(pos), None);
	}
	// pos 3, 6, 7 are non-leaves so we have no data for these
	for pos in vec![3, 6, 7] {
		assert_eq!(backend.get_data(pos), None);
	}

	// pos 8 and 9 are both leaves and should be unaffected by prior pruning

	for x in 1..16 {
		println!("data at {}, {:?}", x, backend.get_data(x));
	}

	assert_eq!(backend.get_data(8), Some(elems[4]));
	assert_eq!(backend.get_hash(8), Some(elems[4].hash_with_index(7)));

	assert_eq!(backend.get_data(9), Some(elems[5]));
	assert_eq!(backend.get_hash(9), Some(elems[5].hash_with_index(8)));

	assert_eq!(backend.data_size(), 2);

	{
		let mut pmmr: PMMR<'_, TestElem, _> = PMMR::at(&mut backend, 10);
		pmmr.rewind(5, &Bitmap::create()).unwrap();
		assert_eq!(pmmr.root(), root1);
	}
	backend.sync().unwrap();
	{
		let pmmr: PMMR<'_, TestElem, _> = PMMR::at(&mut backend, 7);
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
	assert_eq!(backend.data_size(), 0);

	teardown(data_dir);
}

#[test]
fn pmmr_compact_single_leaves() {
	let (data_dir, elems) = setup("compact_single_leaves");
	let mut backend = store::pmmr::PMMRBackend::new(data_dir.clone(), true, None).unwrap();
	let mmr_size = load(0, &elems[0..5], &mut backend);
	backend.sync().unwrap();

	{
		let mut pmmr: PMMR<'_, TestElem, _> = PMMR::at(&mut backend, mmr_size);
		pmmr.prune(1).unwrap();
		pmmr.prune(4).unwrap();
	}

	backend.sync().unwrap();

	// compact
	backend
		.check_compact(2, &Bitmap::create(), &prune_noop)
		.unwrap();

	{
		let mut pmmr: PMMR<'_, TestElem, _> = PMMR::at(&mut backend, mmr_size);
		pmmr.prune(2).unwrap();
		pmmr.prune(5).unwrap();
	}

	backend.sync().unwrap();

	// compact
	backend
		.check_compact(2, &Bitmap::create(), &prune_noop)
		.unwrap();

	teardown(data_dir);
}

#[test]
fn pmmr_compact_entire_peak() {
	let (data_dir, elems) = setup("compact_entire_peak");
	let mut backend = store::pmmr::PMMRBackend::new(data_dir.clone(), true, None).unwrap();
	let mmr_size = load(0, &elems[0..5], &mut backend);
	backend.sync().unwrap();

	let pos_7_hash = backend.get_hash(7).unwrap();

	let pos_8 = backend.get_data(8).unwrap();
	let pos_8_hash = backend.get_hash(8).unwrap();

	// prune all leaves under the peak at pos 7
	{
		let mut pmmr: PMMR<'_, TestElem, _> = PMMR::at(&mut backend, mmr_size);
		pmmr.prune(1).unwrap();
		pmmr.prune(2).unwrap();
		pmmr.prune(4).unwrap();
		pmmr.prune(5).unwrap();
	}

	backend.sync().unwrap();

	// compact
	backend
		.check_compact(2, &Bitmap::create(), &prune_noop)
		.unwrap();

	// now check we have pruned up to and including the peak at pos 7
	// hash still available in underlying hash file
	assert_eq!(backend.get_hash(7), Some(pos_7_hash));
	assert_eq!(backend.get_from_file(7), Some(pos_7_hash));

	// now check we still have subsequent hash and data where we expect
	assert_eq!(backend.get_data(8), Some(pos_8));
	assert_eq!(backend.get_hash(8), Some(pos_8_hash));
	assert_eq!(backend.get_from_file(8), Some(pos_8_hash));

	teardown(data_dir);
}

#[test]
fn pmmr_compact_horizon() {
	let (data_dir, elems) = setup("compact_horizon");
	let mut backend = store::pmmr::PMMRBackend::new(data_dir.clone(), true, None).unwrap();
	let mmr_size = load(0, &elems[..], &mut backend);
	backend.sync().unwrap();

	// 0010012001001230
	// 9 leaves
	assert_eq!(backend.data_size(), 19);
	assert_eq!(backend.hash_size(), 35);

	let pos_1_hash = backend.get_hash(1).unwrap();
	let pos_2_hash = backend.get_hash(2).unwrap();
	let pos_3_hash = backend.get_hash(3).unwrap();
	let pos_6_hash = backend.get_hash(6).unwrap();
	let pos_7_hash = backend.get_hash(7).unwrap();

	let pos_8 = backend.get_data(8).unwrap();
	let pos_8_hash = backend.get_hash(8).unwrap();

	let pos_11 = backend.get_data(11).unwrap();
	let pos_11_hash = backend.get_hash(11).unwrap();

	{
		// pruning some choice nodes
		{
			let mut pmmr: PMMR<'_, TestElem, _> = PMMR::at(&mut backend, mmr_size);
			pmmr.prune(4).unwrap();
			pmmr.prune(5).unwrap();
			pmmr.prune(1).unwrap();
			pmmr.prune(2).unwrap();
		}
		backend.sync().unwrap();

		// check we can read hashes and data correctly after pruning
		{
			// assert_eq!(backend.get_hash(3), None);
			assert_eq!(backend.get_from_file(3), Some(pos_3_hash));

			// assert_eq!(backend.get_hash(6), None);
			assert_eq!(backend.get_from_file(6), Some(pos_6_hash));

			// assert_eq!(backend.get_hash(7), None);
			assert_eq!(backend.get_from_file(7), Some(pos_7_hash));

			assert_eq!(backend.get_hash(8), Some(pos_8_hash));
			assert_eq!(backend.get_data(8), Some(pos_8));
			assert_eq!(backend.get_from_file(8), Some(pos_8_hash));

			assert_eq!(backend.get_hash(11), Some(pos_11_hash));
			assert_eq!(backend.get_data(11), Some(pos_11));
			assert_eq!(backend.get_from_file(11), Some(pos_11_hash));
		}

		// compact
		backend
			.check_compact(4, &Bitmap::of(&vec![1, 2]), &prune_noop)
			.unwrap();
		backend.sync().unwrap();

		// check we can read a hash by pos correctly after compaction
		{
			assert_eq!(backend.get_hash(1), None);
			assert_eq!(backend.get_from_file(1), Some(pos_1_hash));

			assert_eq!(backend.get_hash(2), None);
			assert_eq!(backend.get_from_file(2), Some(pos_2_hash));

			assert_eq!(backend.get_hash(3), Some(pos_3_hash));

			assert_eq!(backend.get_hash(4), None);
			assert_eq!(backend.get_hash(5), None);
			assert_eq!(backend.get_hash(6), Some(pos_6_hash));

			assert_eq!(backend.get_from_file(7), Some(pos_7_hash));

			assert_eq!(backend.get_hash(8), Some(pos_8_hash));
			assert_eq!(backend.get_from_file(8), Some(pos_8_hash));
		}
	}

	// recheck stored data
	{
		// recreate backend
		let backend =
			store::pmmr::PMMRBackend::<TestElem>::new(data_dir.to_string(), true, None).unwrap();

		assert_eq!(backend.data_size(), 19);
		assert_eq!(backend.hash_size(), 35);

		// check we can read a hash by pos correctly from recreated backend
		assert_eq!(backend.get_hash(7), Some(pos_7_hash));
		assert_eq!(backend.get_from_file(7), Some(pos_7_hash));

		assert_eq!(backend.get_hash(8), Some(pos_8_hash));
		assert_eq!(backend.get_from_file(8), Some(pos_8_hash));
	}

	{
		let mut backend =
			store::pmmr::PMMRBackend::<TestElem>::new(data_dir.to_string(), true, None).unwrap();

		{
			let mut pmmr: PMMR<'_, TestElem, _> = PMMR::at(&mut backend, mmr_size);

			pmmr.prune(8).unwrap();
			pmmr.prune(9).unwrap();
		}

		// compact some more
		backend
			.check_compact(9, &Bitmap::create(), &prune_noop)
			.unwrap();
	}

	// recheck stored data
	{
		// recreate backend
		let backend =
			store::pmmr::PMMRBackend::<TestElem>::new(data_dir.to_string(), true, None).unwrap();

		// 0010012001001230

		assert_eq!(backend.data_size(), 13);
		assert_eq!(backend.hash_size(), 27);

		// check we can read a hash by pos correctly from recreated backend
		// get_hash() and get_from_file() should return the same value
		// and we only store leaves in the leaf_set so pos 7 still has a hash in there
		assert_eq!(backend.get_hash(7), Some(pos_7_hash));
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
	let mut backend = store::pmmr::PMMRBackend::new(data_dir.to_string(), true, None).unwrap();
	let mmr_size = load(0, &elems[..], &mut backend);
	backend.sync().unwrap();

	// save the root
	let root = {
		let pmmr: PMMR<'_, TestElem, _> = PMMR::at(&mut backend, mmr_size);
		pmmr.root()
	};

	// pruning some choice nodes
	{
		let mut pmmr: PMMR<'_, TestElem, _> = PMMR::at(&mut backend, mmr_size);
		pmmr.prune(1).unwrap();
		pmmr.prune(2).unwrap();
		pmmr.prune(4).unwrap();
	}
	backend.sync().unwrap();

	// check the root and stored data
	{
		let pmmr: PMMR<'_, TestElem, _> = PMMR::at(&mut backend, mmr_size);
		assert_eq!(root, pmmr.root());
		assert_eq!(pmmr.get_data(5).unwrap(), TestElem(4));
		assert_eq!(pmmr.get_data(11).unwrap(), TestElem(7));
	}

	// compact
	backend
		.check_compact(2, &Bitmap::create(), &prune_noop)
		.unwrap();

	// recheck the root and stored data
	{
		let pmmr: PMMR<'_, TestElem, _> = PMMR::at(&mut backend, mmr_size);
		assert_eq!(root, pmmr.root());
		assert_eq!(pmmr.get_data(5).unwrap(), TestElem(4));
		assert_eq!(pmmr.get_data(11).unwrap(), TestElem(7));
	}

	// now prune some more nodes
	{
		let mut pmmr: PMMR<'_, TestElem, _> = PMMR::at(&mut backend, mmr_size);
		pmmr.prune(5).unwrap();
		pmmr.prune(8).unwrap();
		pmmr.prune(9).unwrap();
	}
	backend.sync().unwrap();

	// recheck the root and stored data
	{
		let pmmr: PMMR<'_, TestElem, _> = PMMR::at(&mut backend, mmr_size);
		assert_eq!(root, pmmr.root());
		assert_eq!(pmmr.get_data(11).unwrap(), TestElem(7));
	}

	// compact
	backend
		.check_compact(2, &Bitmap::create(), &prune_noop)
		.unwrap();

	// recheck the root and stored data
	{
		let pmmr: PMMR<'_, TestElem, _> = PMMR::at(&mut backend, mmr_size);
		assert_eq!(root, pmmr.root());
		assert_eq!(pmmr.get_data(11).unwrap(), TestElem(7));
	}

	teardown(data_dir);
}

#[test]
fn cleanup_rewind_files_test() {
	let expected = 10;
	let prefix_to_delete = "foo";
	let prefix_to_save = "bar";
	let seconds_to_delete_after = 100;

	// create the scenario
	let (data_dir, _) = setup("cleanup_rewind_files_test");
	// create some files with the delete prefix that aren't yet old enough to delete
	create_numbered_files(&data_dir, expected, prefix_to_delete, 0, 0);
	// create some files with the delete prefix that are old enough to delete
	create_numbered_files(
		&data_dir,
		expected,
		prefix_to_delete,
		seconds_to_delete_after,
		expected,
	);
	// create some files with the save prefix that are old enough to delete, but will be saved because they don't start
	// with the right prefix
	create_numbered_files(
		&data_dir,
		expected,
		prefix_to_save,
		seconds_to_delete_after,
		0,
	);

	// run the cleaner
	let actual =
		store::pmmr::clean_files_by_prefix(&data_dir, prefix_to_delete, seconds_to_delete_after)
			.unwrap();
	assert_eq!(
		actual, expected,
		"the clean files by prefix function did not report the correct number of files deleted"
	);

	// check that the reported number is actually correct, the block is to borrow data_dir for the closure
	{
		// this function simply counts the number of files in the directory based on the prefix
		let count_fn = |prefix| {
			let mut remaining_count = 0;
			for entry in fs::read_dir(&data_dir).unwrap() {
				if entry
					.unwrap()
					.file_name()
					.into_string()
					.unwrap()
					.starts_with(prefix)
				{
					remaining_count += 1;
				}
			}
			remaining_count
		};

		assert_eq!(
			count_fn(prefix_to_delete),
			expected, // we expect this many to be left because they weren't old enough to be deleted yet
			"it should delete all of the files it is supposed to delete"
		);
		assert_eq!(
			count_fn(prefix_to_save),
			expected,
			"it should delete none of the files it is not supposed to"
		);
	}

	teardown(data_dir);
}

/// Create some files for testing with, for example
///
/// ```text
/// create_numbered_files(".", 3, "hello.txt.", 100, 2)
/// ```
///
/// will create files
///
/// ```text
/// hello.txt.2
/// hello.txt.3
/// hello.txt.4
/// ```
///
/// in the current working directory that are all 100 seconds old (modified and accessed time)
///
fn create_numbered_files(
	data_dir: &str,
	num_files: u32,
	prefix: &str,
	last_accessed_delay_seconds: u64,
	start_index: u32,
) {
	let now = std::time::SystemTime::now();
	let time_to_set = now - std::time::Duration::from_secs(last_accessed_delay_seconds);
	let time_to_set_ft = filetime::FileTime::from_system_time(time_to_set);

	for rewind_file_num in 0..num_files {
		let path = std::path::Path::new(&data_dir).join(format!(
			"{}.{}",
			prefix,
			start_index + rewind_file_num
		));
		let file = fs::File::create(path.clone()).unwrap();
		let metadata = file.metadata().unwrap();
		filetime::set_file_times(path, time_to_set_ft, time_to_set_ft).unwrap();
	}
}

fn setup(tag: &str) -> (String, Vec<TestElem>) {
	match env_logger::try_init() {
		Ok(_) => println!("Initializing env logger"),
		Err(e) => println!("env logger already initialized: {:?}", e),
	};
	let t = Utc::now();
	let data_dir = format!(
		"./target/tmp/{}.{}-{}",
		t.timestamp(),
		t.timestamp_subsec_nanos(),
		tag
	);
	fs::create_dir_all(data_dir.clone()).unwrap();

	let mut elems = vec![];
	for x in 1..20 {
		elems.push(TestElem(x));
	}
	(data_dir, elems)
}

/// note that taking ownership of the data_dir is a feature
/// because it will not be able to be used after teardown as intended
fn teardown(data_dir: String) {
	fs::remove_dir_all(data_dir).unwrap();
}

fn load(pos: u64, elems: &[TestElem], backend: &mut store::pmmr::PMMRBackend<TestElem>) -> u64 {
	let mut pmmr = PMMR::at(backend, pos);
	for elem in elems {
		pmmr.push(elem).unwrap();
	}
	pmmr.unpruned_size()
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
struct TestElem(u32);

impl FixedLength for TestElem {
	const LEN: usize = 4;
}

impl PMMRable for TestElem {
	type E = Self;

	fn as_elmt(&self) -> Self::E {
		self.clone()
	}
}

impl Writeable for TestElem {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), Error> {
		writer.write_u32(self.0)
	}
}
impl Readable for TestElem {
	fn read(reader: &mut dyn Reader) -> Result<TestElem, Error> {
		Ok(TestElem(reader.read_u32()?))
	}
}
