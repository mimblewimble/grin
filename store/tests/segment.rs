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

use crate::core::core::hash::DefaultHashable;
use crate::core::core::pmmr;
use crate::core::core::pmmr::segment::{Segment, SegmentIdentifier};
use crate::core::core::pmmr::{Backend, ReadablePMMR, ReadonlyPMMR, PMMR};
use crate::core::ser::{
	BinReader, BinWriter, DeserializationMode, Error, PMMRable, ProtocolVersion, Readable, Reader,
	Writeable, Writer,
};
use crate::store::pmmr::PMMRBackend;
use chrono::Utc;
use croaring::Bitmap;
use grin_core as core;
use grin_store as store;
use std::fs;
use std::io::Cursor;

#[test]
fn prunable_mmr() {
	let t = Utc::now();
	let data_dir = format!(
		"./target/tmp/{}.{}-prunable_mmr",
		t.timestamp(),
		t.timestamp_subsec_nanos()
	);
	fs::create_dir_all(&data_dir).unwrap();

	let n_leaves = 64 + 8 + 4 + 2 + 1;
	let mut ba = PMMRBackend::new(&data_dir, true, ProtocolVersion(1), None).unwrap();
	let mut mmr = PMMR::new(&mut ba);
	for i in 0..n_leaves {
		mmr.push(&TestElem([i / 7, i / 5, i / 3, i])).unwrap();
	}
	let last_pos = mmr.unpruned_size();
	let root = mmr.root().unwrap();

	let mut bitmap = Bitmap::create();
	bitmap.add_range(0..n_leaves as u64);

	let id = SegmentIdentifier { height: 3, idx: 1 };

	// Validate a segment before any pruning
	let mmr = ReadonlyPMMR::at(&mut ba, last_pos);
	let segment = Segment::from_pmmr(id, &mmr, true).unwrap();
	assert_eq!(
		segment.root(last_pos, Some(&bitmap)).unwrap().unwrap(),
		mmr.get_hash(29).unwrap()
	);
	segment.validate(last_pos, Some(&bitmap), root).unwrap();

	// Prune a few leaves
	let mut mmr = PMMR::at(&mut ba, last_pos);
	prune(&mut mmr, &mut bitmap, &[8, 9, 13]);
	ba.sync().unwrap();
	ba.check_compact(last_pos, &Bitmap::create()).unwrap();
	ba.sync().unwrap();

	// Validate
	let mmr = ReadonlyPMMR::at(&mut ba, last_pos);
	let segment = Segment::from_pmmr(id, &mmr, true).unwrap();
	assert_eq!(
		segment.root(last_pos, Some(&bitmap)).unwrap().unwrap(),
		mmr.get_hash(29).unwrap()
	);
	segment.validate(last_pos, Some(&bitmap), root).unwrap();

	// Prune more
	let mut mmr = PMMR::at(&mut ba, last_pos);
	prune(&mut mmr, &mut bitmap, &[10, 11]);
	ba.sync().unwrap();
	ba.check_compact(last_pos, &Bitmap::create()).unwrap();
	ba.sync().unwrap();

	// Validate
	let mmr = ReadonlyPMMR::at(&mut ba, last_pos);
	let segment = Segment::from_pmmr(id, &mmr, true).unwrap();
	assert_eq!(
		segment.root(last_pos, Some(&bitmap)).unwrap().unwrap(),
		mmr.get_hash(29).unwrap()
	);
	segment.validate(last_pos, Some(&bitmap), root).unwrap();

	// Prune all but 1
	let mut mmr = PMMR::at(&mut ba, last_pos);
	prune(&mut mmr, &mut bitmap, &[14, 15]);
	ba.sync().unwrap();
	ba.check_compact(last_pos, &Bitmap::create()).unwrap();
	ba.sync().unwrap();

	// Validate
	let mmr = ReadonlyPMMR::at(&mut ba, last_pos);
	let segment = Segment::from_pmmr(id, &mmr, true).unwrap();
	assert_eq!(
		segment.root(last_pos, Some(&bitmap)).unwrap().unwrap(),
		mmr.get_hash(29).unwrap()
	);
	segment.validate(last_pos, Some(&bitmap), root).unwrap();

	// Prune all
	let mut mmr = PMMR::at(&mut ba, last_pos);
	prune(&mut mmr, &mut bitmap, &[12]);
	ba.sync().unwrap();
	ba.check_compact(last_pos, &Bitmap::create()).unwrap();
	ba.sync().unwrap();

	let mmr = ReadonlyPMMR::at(&mut ba, last_pos);
	assert!(Segment::from_pmmr(id, &mmr, true).is_ok());

	// Final segment is not full, test it before pruning
	let id = SegmentIdentifier { height: 3, idx: 9 };

	let mmr = ReadonlyPMMR::at(&mut ba, last_pos);
	let segment = Segment::from_pmmr(id, &mmr, true).unwrap();
	segment.validate(last_pos, Some(&bitmap), root).unwrap();

	// Prune second and third to last leaves (a full peak in the MMR)
	let mut mmr = PMMR::at(&mut ba, last_pos);
	prune(&mut mmr, &mut bitmap, &[76, 77]);
	ba.sync().unwrap();
	ba.check_compact(last_pos, &Bitmap::create()).unwrap();
	ba.sync().unwrap();

	// Validate
	let mmr = ReadonlyPMMR::at(&mut ba, last_pos);
	let segment = Segment::from_pmmr(id, &mmr, true).unwrap();
	segment.validate(last_pos, Some(&bitmap), root).unwrap();

	// Prune final element
	let mut mmr = PMMR::at(&mut ba, last_pos);
	prune(&mut mmr, &mut bitmap, &[78]);
	ba.sync().unwrap();
	ba.check_compact(last_pos, &Bitmap::create()).unwrap();
	ba.sync().unwrap();

	// Validate
	let mmr = ReadonlyPMMR::at(&mut ba, last_pos);
	let segment = Segment::from_pmmr(id, &mmr, true).unwrap();
	segment.validate(last_pos, Some(&bitmap), root).unwrap();

	std::mem::drop(ba);
	fs::remove_dir_all(&data_dir).unwrap();
}

#[test]
fn pruned_segment() {
	let t = Utc::now();
	let data_dir = format!(
		"./target/tmp/{}.{}-pruned_segment",
		t.timestamp(),
		t.timestamp_subsec_nanos()
	);
	fs::create_dir_all(&data_dir).unwrap();

	let n_leaves = 16;
	let mut ba = PMMRBackend::new(&data_dir, true, ProtocolVersion(1), None).unwrap();
	let mut mmr = PMMR::new(&mut ba);
	for i in 0..n_leaves {
		mmr.push(&TestElem([i / 7, i / 5, i / 3, i])).unwrap();
	}
	let last_pos = mmr.unpruned_size();
	let root = mmr.root().unwrap();

	let mut bitmap = Bitmap::create();
	bitmap.add_range(0..n_leaves as u64);

	// Prune all leaves of segment 1
	prune(&mut mmr, &mut bitmap, &[4, 5, 6, 7]);
	ba.sync().unwrap();
	ba.check_compact(last_pos, &Bitmap::create()).unwrap();
	ba.sync().unwrap();

	// Validate the empty segment 1
	let id = SegmentIdentifier { height: 2, idx: 1 };
	let mmr = ReadonlyPMMR::at(&mut ba, last_pos);
	let segment = Segment::from_pmmr(id, &mmr, true).unwrap();
	assert_eq!(segment.leaf_iter().count(), 0);
	assert_eq!(segment.hash_iter().count(), 1);
	assert_eq!(
		segment
			.first_unpruned_parent(last_pos, Some(&bitmap))
			.unwrap(),
		(ba.get_hash(13).unwrap(), 14)
	);
	assert!(segment.root(last_pos, Some(&bitmap)).unwrap().is_none());
	segment.validate(last_pos, Some(&bitmap), root).unwrap();

	// Prune all leaves of segment 0
	let mut mmr = PMMR::at(&mut ba, last_pos);
	prune(&mut mmr, &mut bitmap, &[0, 1, 2, 3]);
	ba.sync().unwrap();
	ba.check_compact(last_pos, &Bitmap::create()).unwrap();
	ba.sync().unwrap();

	// Validate the empty segment 1 again
	let mmr = ReadonlyPMMR::at(&mut ba, last_pos);
	let segment = Segment::from_pmmr(id, &mmr, true).unwrap();
	assert_eq!(segment.leaf_iter().count(), 0);
	assert_eq!(segment.hash_iter().count(), 1);
	// Since both 7 and 14 are now pruned, the first unpruned hash will be at 15
	assert_eq!(
		segment
			.first_unpruned_parent(last_pos, Some(&bitmap))
			.unwrap(),
		(ba.get_hash(14).unwrap(), 15)
	);
	assert!(segment.root(last_pos, Some(&bitmap)).unwrap().is_none());
	segment.validate(last_pos, Some(&bitmap), root).unwrap();

	// Prune all leaves of segment 2 & 3
	let mut mmr = PMMR::at(&mut ba, last_pos);
	prune(&mut mmr, &mut bitmap, &[8, 9, 10, 11, 12, 13, 14, 15]);
	ba.sync().unwrap();
	ba.check_compact(last_pos, &Bitmap::create()).unwrap();
	ba.sync().unwrap();

	// Validate the empty segment 1 again
	let mmr = ReadonlyPMMR::at(&mut ba, last_pos);
	let segment = Segment::from_pmmr(id, &mmr, true).unwrap();
	assert_eq!(segment.leaf_iter().count(), 0);
	assert_eq!(segment.hash_iter().count(), 1);
	// Since both 15 and 30 are now pruned, the first unpruned hash will be at 31: the mmr root
	assert_eq!(
		segment
			.first_unpruned_parent(last_pos, Some(&bitmap))
			.unwrap(),
		(root, 31)
	);
	assert!(segment.root(last_pos, Some(&bitmap)).unwrap().is_none());
	segment.validate(last_pos, Some(&bitmap), root).unwrap();

	let n_leaves = n_leaves + 4 + 2 + 1;
	let mut mmr = PMMR::at(&mut ba, last_pos);
	for i in 16..n_leaves {
		mmr.push(&TestElem([i / 7, i / 5, i / 3, i])).unwrap();
	}
	bitmap.add_range(16..n_leaves as u64);
	let last_pos = mmr.unpruned_size();
	let root = mmr.root().unwrap();

	// Prune all leaves of segment 4
	// The root of this segment is a direct peak of the full MMR
	prune(&mut mmr, &mut bitmap, &[16, 17, 18, 19]);
	ba.sync().unwrap();
	ba.check_compact(last_pos, &Bitmap::create()).unwrap();
	ba.sync().unwrap();

	// Validate segment 4
	let id = SegmentIdentifier { height: 2, idx: 4 };
	let mmr = ReadonlyPMMR::at(&mut ba, last_pos);
	let segment = Segment::from_pmmr(id, &mmr, true).unwrap();
	assert_eq!(segment.leaf_iter().count(), 0);
	assert_eq!(segment.hash_iter().count(), 1);
	assert_eq!(
		segment
			.first_unpruned_parent(last_pos, Some(&bitmap))
			.unwrap(),
		(ba.get_hash(37).unwrap(), 38)
	);
	assert!(segment.root(last_pos, Some(&bitmap)).unwrap().is_none());
	segment.validate(last_pos, Some(&bitmap), root).unwrap();

	// Segment 5 has 2 peaks
	let id = SegmentIdentifier { height: 2, idx: 5 };
	let mmr = ReadonlyPMMR::at(&mut ba, last_pos);
	let segment = Segment::from_pmmr(id, &mmr, true).unwrap();
	assert_eq!(segment.leaf_iter().count(), 3);
	assert_eq!(
		segment
			.first_unpruned_parent(last_pos, Some(&bitmap))
			.unwrap()
			.1,
		1 + segment.segment_pos_range(last_pos).1
	);
	assert!(segment.root(last_pos, Some(&bitmap)).unwrap().is_some());
	segment.validate(last_pos, Some(&bitmap), root).unwrap();
	let prev_segment = segment;

	// Prune final leaf (a peak)
	let mut mmr = PMMR::at(&mut ba, last_pos);
	prune(&mut mmr, &mut bitmap, &[22]);
	ba.sync().unwrap();
	ba.check_compact(last_pos, &Bitmap::create()).unwrap();
	ba.sync().unwrap();

	// Segment 5 should be unchanged
	let mmr = ReadonlyPMMR::at(&mut ba, last_pos);
	let segment = Segment::from_pmmr(id, &mmr, true).unwrap();
	assert_eq!(segment, prev_segment);
	segment.validate(last_pos, Some(&bitmap), root).unwrap();

	// Prune other peak of segment 5
	let mut mmr = PMMR::at(&mut ba, last_pos);
	prune(&mut mmr, &mut bitmap, &[20, 21]);
	ba.sync().unwrap();
	ba.check_compact(last_pos, &Bitmap::create()).unwrap();
	ba.sync().unwrap();

	// Validate segment 5 again
	let mmr = ReadonlyPMMR::at(&mut ba, last_pos);
	let segment = Segment::from_pmmr(id, &mmr, true).unwrap();
	assert_eq!(segment.leaf_iter().count(), 1);
	assert_eq!(segment.hash_iter().count(), 1);
	assert_eq!(
		segment
			.first_unpruned_parent(last_pos, Some(&bitmap))
			.unwrap()
			.1,
		1 + segment.segment_pos_range(last_pos).1
	);
	assert!(segment.root(last_pos, Some(&bitmap)).unwrap().is_some());
	segment.validate(last_pos, Some(&bitmap), root).unwrap();

	std::mem::drop(ba);
	fs::remove_dir_all(&data_dir).unwrap();
}

#[test]
fn ser_round_trip() {
	let t = Utc::now();
	let data_dir = format!(
		"./target/tmp/{}.{}-segment_ser_round_trip",
		t.timestamp(),
		t.timestamp_subsec_nanos()
	);
	fs::create_dir_all(&data_dir).unwrap();

	let n_leaves = 32;
	let mut ba = PMMRBackend::new(&data_dir, true, ProtocolVersion(1), None).unwrap();
	let mut mmr = pmmr::PMMR::new(&mut ba);
	for i in 0..n_leaves {
		mmr.push(&TestElem([i / 7, i / 5, i / 3, i])).unwrap();
	}
	let mut bitmap = Bitmap::create();
	bitmap.add_range(0..n_leaves as u64);
	let last_pos = mmr.unpruned_size();

	prune(&mut mmr, &mut bitmap, &[0, 1]);
	ba.sync().unwrap();
	ba.check_compact(last_pos, &Bitmap::create()).unwrap();
	ba.sync().unwrap();

	let mmr = ReadonlyPMMR::at(&ba, last_pos);
	let id = SegmentIdentifier { height: 3, idx: 0 };
	let segment = Segment::from_pmmr(id, &mmr, true).unwrap();

	let mut cursor = Cursor::new(Vec::<u8>::new());
	let mut writer = BinWriter::new(&mut cursor, ProtocolVersion(1));
	Writeable::write(&segment, &mut writer).unwrap();
	assert_eq!(
		cursor.position(),
		(9) + (8 + 7 * (8 + 32)) + (8 + 6 * (8 + 16)) + (8 + 2 * 32)
	);
	cursor.set_position(0);

	let mut reader = BinReader::new(
		&mut cursor,
		ProtocolVersion(1),
		DeserializationMode::default(),
	);
	let segment2: Segment<TestElem> = Readable::read(&mut reader).unwrap();
	assert_eq!(segment, segment2);

	std::mem::drop(ba);
	fs::remove_dir_all(&data_dir).unwrap();
}

fn prune<T, B>(mmr: &mut PMMR<T, B>, bitmap: &mut Bitmap, leaf_idxs: &[u64])
where
	T: PMMRable,
	B: Backend<T>,
{
	for &leaf_idx in leaf_idxs {
		mmr.prune(pmmr::insertion_to_pmmr_index(leaf_idx)).unwrap();
		bitmap.remove(leaf_idx as u32);
	}
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct TestElem(pub [u32; 4]);

impl DefaultHashable for TestElem {}

impl PMMRable for TestElem {
	type E = Self;

	fn as_elmt(&self) -> Self::E {
		*self
	}

	fn elmt_size() -> Option<u16> {
		Some(16)
	}
}

impl Writeable for TestElem {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), Error> {
		writer.write_u32(self.0[0])?;
		writer.write_u32(self.0[1])?;
		writer.write_u32(self.0[2])?;
		writer.write_u32(self.0[3])
	}
}

impl Readable for TestElem {
	fn read<R: Reader>(reader: &mut R) -> Result<TestElem, Error> {
		Ok(TestElem([
			reader.read_u32()?,
			reader.read_u32()?,
			reader.read_u32()?,
			reader.read_u32()?,
		]))
	}
}
