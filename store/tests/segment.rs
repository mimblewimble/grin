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

use crate::core::core::hash::DefaultHashable;
use crate::core::core::pmmr;
use crate::core::core::pmmr::segment::{Segment, SegmentIdentifier};
use crate::core::core::pmmr::{ReadablePMMR, ReadonlyPMMR, PMMR};
use crate::core::ser::{Error, PMMRable, ProtocolVersion, Readable, Reader, Writeable, Writer};
use crate::store::pmmr::PMMRBackend;
use chrono::Utc;
use croaring::Bitmap;
use grin_core as core;
use grin_store as store;
use std::fs;

#[test]
fn prunable_mmr() {
	let t = Utc::now();
	let data_dir = format!(
		"./target/tmp/{}.{}-prunable_mmr",
		t.timestamp(),
		t.timestamp_subsec_nanos()
	);
	fs::create_dir_all(&data_dir).unwrap();

	let n_leaves = 64 + 8 + 4 + 1;
	let mut ba = PMMRBackend::new(&data_dir, true, ProtocolVersion(1), None).unwrap();
	let mut mmr = PMMR::new(&mut ba);
	for i in 0..n_leaves {
		mmr.push(&TestElem([i / 7, i / 5, i / 3, i])).unwrap();
	}
	let last_pos = mmr.unpruned_size();
	let root = mmr.root().unwrap();

	let mut bitmap = Bitmap::create();
	bitmap.add_range_closed(1..n_leaves);

	let id = SegmentIdentifier {
		log_size: 3,
		idx: 1,
	};

	// Validate a segment before any pruning
	let mmr = ReadonlyPMMR::at(&mut ba, last_pos);
	let segment = Segment::from_pmmr(id, &mmr, true).unwrap();
	assert_eq!(
		segment.root(last_pos, Some(&bitmap)).unwrap(),
		mmr.get_hash(30).unwrap()
	);
	segment.validate(last_pos, Some(&bitmap), root).unwrap();

	// Prune a few leaves
	let mut mmr = PMMR::at(&mut ba, last_pos);
	mmr.prune(pmmr::insertion_to_pmmr_index(9)).unwrap();
	bitmap.remove(9);
	mmr.prune(pmmr::insertion_to_pmmr_index(10)).unwrap();
	bitmap.remove(10);
	mmr.prune(pmmr::insertion_to_pmmr_index(13)).unwrap();
	bitmap.remove(13);
	ba.sync().unwrap();
	ba.check_compact(last_pos, &Bitmap::create()).unwrap();
	ba.sync().unwrap();

	// Validate a full segment with some pruned leaves
	let mmr = ReadonlyPMMR::at(&mut ba, last_pos);
	let segment = Segment::from_pmmr(id, &mmr, true).unwrap();
	assert_eq!(
		segment.root(last_pos, Some(&bitmap)).unwrap(),
		mmr.get_hash(30).unwrap()
	);
	segment.validate(last_pos, Some(&bitmap), root).unwrap();

	// Prune more
	let mut mmr = PMMR::at(&mut ba, last_pos);
	mmr.prune(pmmr::insertion_to_pmmr_index(11)).unwrap();
	bitmap.remove(11);
	mmr.prune(pmmr::insertion_to_pmmr_index(12)).unwrap();
	bitmap.remove(12);
	mmr.prune(pmmr::insertion_to_pmmr_index(14)).unwrap();
	bitmap.remove(14);
	ba.sync().unwrap();
	ba.check_compact(last_pos, &Bitmap::create()).unwrap();
	ba.sync().unwrap();

	// Validate again
	let mmr = ReadonlyPMMR::at(&mut ba, last_pos);
	let segment = Segment::from_pmmr(id, &mmr, true).unwrap();
	assert_eq!(
		segment.root(last_pos, Some(&bitmap)).unwrap(),
		mmr.get_hash(30).unwrap()
	);
	segment.validate(last_pos, Some(&bitmap), root).unwrap();

	fs::remove_dir_all(&data_dir).unwrap();
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
