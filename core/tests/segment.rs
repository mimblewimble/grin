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

mod common;

use self::core::core::pmmr;
use self::core::core::{Segment, SegmentIdentifier};
use common::TestElem;
use grin_core as core;
use grin_core::core::pmmr::ReadablePMMR;

fn test_unprunable_size(n_leaves: u64) {
	let log_size = 2;
	let size = 1u64 << log_size;
	let n_segments = (n_leaves + size - 1) / size;

	// Build an MMR with n_leaves leaves
	let mut ba = pmmr::VecBackend::new();
	let mut pmmr = pmmr::PMMR::new(&mut ba);
	for i in 0..(n_leaves as u32) {
		pmmr.push(&TestElem([i / 7, i / 5, i / 3, i])).unwrap();
	}
	let rpmmr = pmmr.readonly_pmmr();
	let root = rpmmr.root().unwrap();

	for idx in 0..n_segments {
		let id = SegmentIdentifier { log_size, idx };
		let segment = Segment::from_pmmr(id, &rpmmr).unwrap();
		println!(
			"\n\n>>>>>>> N_LEAVES = {}, LAST_POS = {}, SEGMENT = {}:\n{:#?}",
			n_leaves,
			rpmmr.unpruned_size(),
			idx,
			segment
		);
		segment.validate(pmmr.last_pos, None, root).unwrap();
		println!(" PROOF OK");
	}
}

#[test]
fn unprunable_pmmr() {
	for i in 1..=64 {
		test_unprunable_size(i);
	}
}
