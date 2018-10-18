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
use std::io::{self, BufRead, BufReader, BufWriter, ErrorKind, Read, Write};
use std::os::unix::io::AsRawFd;
use std::path::Path;

#[cfg(not(any(target_os = "linux", target_os = "android")))]
use libc::{ftruncate as ftruncate64, off_t as off64_t};
#[cfg(any(target_os = "linux"))]
use libc::{ftruncate64, off64_t};

use core::core::hash::Hash;
use core::ser;

/// A no-op function for doing nothing with some pruned data.
pub fn prune_noop(_pruned_data: &[u8]) {}

/// Hash file (MMR) wrapper around an append only file.
pub struct HashFile {
	file: AppendOnlyFile,
}

impl HashFile {
	/// Open (or create) a hash file at the provided path on disk.
	pub fn open(path: String) -> io::Result<HashFile> {
		let file = AppendOnlyFile::open(path)?;
		Ok(HashFile { file })
	}

	/// Append a hash to this hash file.
	/// Will not be written to disk until flush() is subsequently called.
	/// Alternatively discard() may be called to discard any pending changes.
	pub fn append(&mut self, hash: &Hash) -> io::Result<()> {
		let mut bytes = ser::ser_vec(hash).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
		self.file.append(&mut bytes);
		Ok(())
	}

	/// Read a hash from the hash file by position.
	pub fn read(&self, position: u64) -> Option<Hash> {
		// The MMR starts at 1, our binary backend starts at 0.
		let pos = position - 1;

		// Must be on disk, doing a read at the correct position
		let file_offset = (pos as usize) * Hash::SIZE;
		let data = self.file.read(file_offset, Hash::SIZE);
		match ser::deserialize(&mut &data[..]) {
			Ok(h) => Some(h),
			Err(e) => {
				error!(
					"Corrupted storage, could not read an entry from hash file: {:?}",
					e
				);
				return None;
			}
		}
	}

	/// Rewind the backend file to the specified position.
	pub fn rewind(&mut self, position: u64) -> io::Result<()> {
		self.file.rewind(position * Hash::SIZE as u64);
		Ok(())
	}

	/// Flush unsynced changes to the hash file to disk.
	pub fn flush(&mut self) -> io::Result<()> {
		self.file.flush()
	}

	/// Discard any unsynced changes to the hash file.
	pub fn discard(&mut self) {
		self.file.discard()
	}

	/// Size of the hash file in bytes.
	pub fn size(&self) -> io::Result<u64> {
		self.file.size()
	}
}

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
	/// Supports two scenarios currently -
	///   * rewind from a clean state (rewinding to handle a forked block)
	///   * rewind within the buffer itself (raw_tx fails to validate)
	/// Note: we do not currently support a rewind() that
	/// crosses the buffer boundary.
	pub fn rewind(&mut self, file_pos: u64) {
		if self.buffer.is_empty() {
			// rewinding from clean state, no buffer, not already rewound anything
			if self.buffer_start_bak == 0 {
				self.buffer_start_bak = self.buffer_start;
			}
			self.buffer_start = file_pos as usize;
		} else {
			// rewinding (within) the buffer
			if self.buffer_start as u64 > file_pos {
				panic!("cannot rewind buffer beyond buffer_start");
			} else {
				let buffer_len = file_pos - self.buffer_start as u64;
				self.buffer.truncate(buffer_len as usize);
			}
		}
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
			let buffer_offset = offset - self.buffer_start;
			return self.read_from_buffer(buffer_offset, length);
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

	// Read length bytes from the buffer, from offset.
	// Return empty vec if we do not have enough bytes in the buffer to read a full
	// vec.
	fn read_from_buffer(&self, offset: usize, length: usize) -> Vec<u8> {
		if self.buffer.len() < (offset + length) {
			vec![]
		} else {
			self.buffer[offset..(offset + length)].to_vec()
		}
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

	/// Current size of the (unsynced) file in bytes.
	pub fn size_unsync(&self) -> u64 {
		(self.buffer_start + self.buffer.len()) as u64
	}

	/// Path of the underlying file
	pub fn path(&self) -> String {
		self.path.clone()
	}
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
