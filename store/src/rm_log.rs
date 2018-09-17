// Copyright 2018 The Grin Developers
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

//! The deprecated rm_log impl. Still used for migration
//! from rm_log -> leaf_set on startup and fast sync.

use std::fs::File;
use std::io::{self, BufWriter, Write};

use core::ser;
use types::read_ordered_vec;

/// Log file fully cached in memory containing all positions that should be
/// eventually removed from the MMR append-only data file. Allows quick
/// checking of whether a piece of data has been marked for deletion. When the
/// log becomes too long, the MMR backend will actually remove chunks from the
/// MMR data file and truncate the remove log.
pub struct RemoveLog {
	path: String,
	/// Ordered vector of MMR positions that should get eventually removed.
	pub removed: Vec<(u64, u32)>,
	// Holds positions temporarily until flush is called.
	removed_tmp: Vec<(u64, u32)>,
	// Holds truncated removed temporarily until discarded or committed
	removed_bak: Vec<(u64, u32)>,
}

impl RemoveLog {
	/// Open the remove log file.
	/// The content of the file will be read in memory for fast checking.
	pub fn open(path: String) -> io::Result<RemoveLog> {
		let removed = read_ordered_vec(path.clone(), 12)?;
		Ok(RemoveLog {
			path: path,
			removed: removed,
			removed_tmp: vec![],
			removed_bak: vec![],
		})
	}

	/// Rewinds the remove log back to the provided index.
	/// We keep everything in the rm_log from that index and earlier.
	/// In practice the index is a block height, so we rewind back to that block
	/// keeping everything in the rm_log up to and including that block.
	pub fn rewind(&mut self, idx: u32) -> io::Result<()> {
		// backing it up before truncating (unless we already have a backup)
		if self.removed_bak.is_empty() {
			self.removed_bak = self.removed.clone();
		}

		if idx == 0 {
			self.removed = vec![];
			self.removed_tmp = vec![];
		} else {
			// retain rm_log entries up to and including those at the provided index
			self.removed.retain(|&(_, x)| x <= idx);
			self.removed_tmp.retain(|&(_, x)| x <= idx);
		}
		Ok(())
	}

	/// Append a set of new positions to the remove log. Both adds those
	/// positions the ordered in-memory set and to the file.
	pub fn append(&mut self, elmts: Vec<u64>, index: u32) -> io::Result<()> {
		for elmt in elmts {
			match self.removed_tmp.binary_search(&(elmt, index)) {
				Ok(_) => continue,
				Err(idx) => {
					self.removed_tmp.insert(idx, (elmt, index));
				}
			}
		}
		Ok(())
	}

	/// Flush the positions to remove to file.
	pub fn flush(&mut self) -> io::Result<()> {
		for elmt in &self.removed_tmp {
			match self.removed.binary_search(&elmt) {
				Ok(_) => continue,
				Err(idx) => {
					self.removed.insert(idx, *elmt);
				}
			}
		}
		let mut file = BufWriter::new(File::create(self.path.clone())?);
		for elmt in &self.removed {
			file.write_all(&ser::ser_vec(&elmt).unwrap()[..])?;
		}
		self.removed_tmp = vec![];
		self.removed_bak = vec![];
		file.flush()
	}

	/// Discard pending changes
	pub fn discard(&mut self) {
		if self.removed_bak.len() > 0 {
			self.removed = self.removed_bak.clone();
			self.removed_bak = vec![];
		}
		self.removed_tmp = vec![];
	}

	/// Whether the remove log currently includes the provided position.
	pub fn includes(&self, elmt: u64) -> bool {
		include_tuple(&self.removed, elmt) || include_tuple(&self.removed_tmp, elmt)
	}

	/// Number of positions stored in the remove log.
	pub fn len(&self) -> usize {
		self.removed.len()
	}

	/// Return vec of pos for removed elements before the provided cutoff index.
	/// Useful for when we prune and compact an MMR.
	pub fn removed_pre_cutoff(&self, cutoff_idx: u32) -> Vec<u64> {
		self.removed
			.iter()
			.filter_map(
				|&(pos, idx)| {
					if idx < cutoff_idx {
						Some(pos)
					} else {
						None
					}
				},
			).collect()
	}
}

fn include_tuple(v: &Vec<(u64, u32)>, e: u64) -> bool {
	if let Err(pos) = v.binary_search(&(e, 0)) {
		if pos < v.len() && v[pos].0 == e {
			return true;
		}
	}
	false
}
