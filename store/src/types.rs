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

//! Common storage-related types
use memmap;

use std::cmp;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, BufWriter, ErrorKind, Write};
use std::os::unix::io::AsRawFd;
use std::io::Read;
use std::path::Path;

#[cfg(any(target_os = "linux"))]
use libc::{ftruncate64, off64_t};
#[cfg(not(any(target_os = "linux", target_os = "android")))]
use libc::{ftruncate as ftruncate64, off_t as off64_t};

use core::ser;

/// A no-op function for doing nothing with some pruned data.
pub fn prune_noop(_pruned_data: &[u8]) {}

/// Wrapper for a file that can be read at any position (random read) but for
/// which writes are append only. Reads are backed by a memory map (mmap(2)),
/// relying on the operating system for fast access and caching. The memory
/// map is reallocated to expand it when new writes are flushed.
///
/// Despite being append-only, the file can still be pruned and truncated. The
/// former simply happens by rewriting it, ignoring some of the data. The
/// latter by truncating the underlying file and re-creating the mmap.
pub struct AppendOnlyFile {
	path: String,
	file: File,
	mmap: Option<memmap::Mmap>,
	buffer_start: usize,
	buffer: Vec<u8>,
	buffer_start_bak: usize,
}

impl AppendOnlyFile {
	/// Open a file (existing or not) as append-only, backed by a mmap.
	pub fn open(path: String) -> io::Result<AppendOnlyFile> {
		let file = OpenOptions::new()
			.read(true)
			.append(true)
			.create(true)
			.open(path.clone())?;
		let mut aof = AppendOnlyFile {
			path: path.clone(),
			file: file,
			mmap: None,
			buffer_start: 0,
			buffer: vec![],
			buffer_start_bak: 0,
		};
		// if we have a non-empty file then mmap it.
		if let Ok(sz) = aof.size() {
			if sz > 0 {
				aof.buffer_start = sz as usize;
				aof.mmap = Some(unsafe { memmap::Mmap::map(&aof.file)? });
			}
		}
		Ok(aof)
	}

	/// Append data to the file. Until the append-only file is synced, data is
	/// only written to memory.
	pub fn append(&mut self, buf: &mut Vec<u8>) {
		self.buffer.append(buf);
	}

	/// Rewinds the data file back to a lower position. The new position needs
	/// to be the one of the first byte the next time data is appended.
	pub fn rewind(&mut self, pos: u64) {
		if self.buffer_start_bak > 0 || self.buffer.len() > 0 {
			panic!("Can't rewind on a dirty state.");
		}
		self.buffer_start_bak = self.buffer_start;
		self.buffer_start = pos as usize;
	}

	/// Syncs all writes (fsync), reallocating the memory map to make the newly
	/// written data accessible.
	pub fn flush(&mut self) -> io::Result<()> {
		if self.buffer_start_bak > 0 {
			// flushing a rewound state, we need to truncate before applying
			self.truncate(self.buffer_start)?;
			self.buffer_start_bak = 0;
		}

		self.buffer_start += self.buffer.len();
		self.file.write(&self.buffer[..])?;
		self.file.sync_all()?;

		self.buffer = vec![];

		// Note: file must be non-empty to memory map it
		if self.file.metadata()?.len() == 0 {
			self.mmap = None;
		} else {
			self.mmap = Some(unsafe { memmap::Mmap::map(&self.file)? });
		}

		Ok(())
	}

	/// Returns the last position (in bytes), taking into account whether data
	/// has been rewound
	pub fn last_buffer_pos(&self) -> usize {
		self.buffer_start
	}

	/// Discard the current non-flushed data.
	pub fn discard(&mut self) {
		if self.buffer_start_bak > 0 {
			// discarding a rewound state, restore the buffer start
			self.buffer_start = self.buffer_start_bak;
			self.buffer_start_bak = 0;
		}
		self.buffer = vec![];
	}

	/// Read length bytes of data at offset from the file.
	/// Leverages the memory map.
	pub fn read(&self, offset: usize, length: usize) -> Vec<u8> {
		if offset >= self.buffer_start {
			if self.buffer.is_empty() {
				return vec![];
			}
			let offset = offset - self.buffer_start;
			return self.buffer[offset..(offset + length)].to_vec();
		}
		if let None = self.mmap {
			return vec![];
		}
		let mmap = self.mmap.as_ref().unwrap();

		if mmap.len() < (offset + length) {
			return vec![];
		}

		(&mmap[offset..(offset + length)]).to_vec()
	}

	/// Truncates the underlying file to the provided offset
	pub fn truncate(&self, offs: usize) -> io::Result<()> {
		let fd = self.file.as_raw_fd();
		let res = unsafe { ftruncate64(fd, offs as off64_t) };
		if res == -1 {
			Err(io::Error::last_os_error())
		} else {
			Ok(())
		}
	}

	/// Saves a copy of the current file content, skipping data at the provided
	/// prune indices. The prune Vec must be ordered.
	pub fn save_prune<T>(
		&self,
		target: String,
		prune_offs: Vec<u64>,
		prune_len: u64,
		prune_cb: T,
	) -> io::Result<()>
	where
		T: Fn(&[u8]),
	{
		if prune_offs.is_empty() {
			fs::copy(self.path.clone(), target.clone())?;
			Ok(())
		} else {
			let mut reader = File::open(self.path.clone())?;
			let mut writer = BufWriter::new(File::create(target.clone())?);

			// align the buffer on prune_len to avoid misalignments
			let mut buf = vec![0; (prune_len * 256) as usize];
			let mut read = 0;
			let mut prune_pos = 0;
			loop {
				// fill our buffer
				let len = match reader.read(&mut buf) {
					Ok(0) => return Ok(()),
					Ok(len) => len,
					Err(ref e) if e.kind() == ErrorKind::Interrupted => continue,
					Err(e) => return Err(e),
				} as u64;

				// write the buffer, except if we prune offsets in the current span,
				// in which case we skip
				let mut buf_start = 0;
				while prune_offs[prune_pos] >= read && prune_offs[prune_pos] < read + len {
					let prune_at = (prune_offs[prune_pos] - read) as usize;
					if prune_at != buf_start {
						writer.write_all(&buf[buf_start..prune_at])?;
					} else {
						prune_cb(&buf[buf_start..prune_at]);
					}
					buf_start = prune_at + (prune_len as usize);
					if prune_offs.len() > prune_pos + 1 {
						prune_pos += 1;
					} else {
						break;
					}
				}
				writer.write_all(&mut buf[buf_start..(len as usize)])?;
				read += len;
			}
		}
	}

	/// Current size of the file in bytes.
	pub fn size(&self) -> io::Result<u64> {
		fs::metadata(&self.path).map(|md| md.len())
	}

	/// Path of the underlying file
	pub fn path(&self) -> String {
		self.path.clone()
	}
}

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
		// simplifying assumption: we always remove older than what's in tmp
		self.removed_tmp = vec![];
		// backing it up before truncating
		self.removed_bak = self.removed.clone();

		if idx == 0 {
			self.removed = vec![];
		} else {
			// retain rm_log entries up to and including those at the provided index
			self.removed.retain(|&(_, x)| x <= idx);
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
			)
			.collect()
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

/// Read an ordered vector of scalars from a file.
pub fn read_ordered_vec<T>(path: String, elmt_len: usize) -> io::Result<Vec<T>>
where
	T: ser::Readable + cmp::Ord,
{
	let file_path = Path::new(&path);
	let mut ovec = Vec::with_capacity(1000);
	if file_path.exists() {
		let mut file = BufReader::with_capacity(elmt_len * 1000, File::open(path.clone())?);
		loop {
			// need a block to end mutable borrow before consume
			let buf_len = {
				let buf = file.fill_buf()?;
				if buf.len() == 0 {
					break;
				}
				let elmts_res: Result<Vec<T>, ser::Error> = ser::deserialize(&mut &buf[..]);
				match elmts_res {
					Ok(elmts) => for elmt in elmts {
						if let Err(idx) = ovec.binary_search(&elmt) {
							ovec.insert(idx, elmt);
						}
					},
					Err(_) => {
						return Err(io::Error::new(
							io::ErrorKind::InvalidData,
							format!("Corrupted storage, could not read file at {}", path),
						));
					}
				}
				buf.len()
			};
			file.consume(buf_len);
		}
	}
	Ok(ovec)
}

/// Writes an ordered vector to a file
pub fn write_vec<T>(path: String, v: &Vec<T>) -> io::Result<()>
where
	T: ser::Writeable,
{
	let mut file_path = File::create(&path)?;
	ser::serialize(&mut file_path, v).map_err(|_| {
		io::Error::new(
			io::ErrorKind::InvalidInput,
			format!("Failed to serialize data when writing to {}", path),
		)
	})?;
	Ok(())
}
