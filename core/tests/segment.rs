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

mod common;

use self::core::core::pmmr;
use self::core::core::{Segment, SegmentIdentifier};
use common::TestElem;
use grin_core as core;
use grin_core::core::pmmr::ReadablePMMR;

fn test_unprunable_size(height: u8, n_leaves: u32) {
	let size = 1u64 << height;
	let n_segments = (n_leaves as u64 + size - 1) / size;

	// Build an MMR with n_leaves leaves
	let mut ba = pmmr::VecBackend::new();
	let mut mmr = pmmr::PMMR::new(&mut ba);
	for i in 0..n_leaves {
		mmr.push(&TestElem([i / 7, i / 5, i / 3, i])).unwrap();
	}
	let mmr = mmr.readonly_pmmr();
	let last_pos = mmr.unpruned_size();
	let root = mmr.root().unwrap();

	for idx in 0..n_segments {
		let id = SegmentIdentifier { height, idx };
		let segment = Segment::from_pmmr(id, &mmr, false).unwrap();
		println!(
			"\n\n>>>>>>> N_LEAVES = {}, LAST_POS = {}, SEGMENT = {}:\n{:#?}",
			n_leaves, last_pos, idx, segment
		);
		if idx < n_segments - 1 || (n_leaves as u64) % size == 0 {
			// Check if the reconstructed subtree root matches with the hash stored in the mmr
			let subtree_root = segment.root(last_pos, None).unwrap().unwrap();
			let last = pmmr::insertion_to_pmmr_index((idx + 1) * size - 1) + (height as u64);
			assert_eq!(subtree_root, mmr.get_hash(last).unwrap());
			println!(" ROOT OK");
		}
		segment.validate(last_pos, None, root).unwrap();
		println!(" PROOF OK");
	}
}

#[test]
fn unprunable_mmr() {
	for i in 1..=64 {
		test_unprunable_size(3, i);
	}
}
