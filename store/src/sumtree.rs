// Copyright 2017 The Grin Developers
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

//! Implementation of the persistent Backend for the prunable MMR sum-tree.

use memmap;

use std::fs::{File, OpenOptions, metadata};
use std::io::{self, Write, BufReader, BufRead};
use std::path::Path;

use core::core::pmmr::{Summable, Backend, HashSum, VecBackend};
use core::ser;

const PMMR_DATA_FILE: &'static str = "pmmr_dat.bin";
const PMMR_RM_LOG_FILE: &'static str = "pmmr_rm_log.bin";

/// Wrapper for a file that can be read at any position (random read) but for
/// which writes are append only. Reads are backed by a memory map (mmap(2)),
/// relying on the operating system for fast access and caching. The memory
/// map is reallocated to expand it when new writes are flushed.
struct AppendOnlyFile {
	path: String,
	file: File,
	mmap: Option<memmap::Mmap>,
}

impl AppendOnlyFile {
	/// Open a file (existing or not) as append-only, backed by a mmap.
	fn open(path: String) -> io::Result<AppendOnlyFile> {
		let file = OpenOptions::new()
			.read(true)
			.append(true)
			.create(true)
			.open(path.clone())?;
		Ok(AppendOnlyFile {
			path: path,
			file: file,
			mmap: None,
		})
	}

	/// Append data to the file.
	fn append(&mut self, buf: &[u8]) -> io::Result<()> {
		self.file.write_all(buf)
	}

	/// Syncs all writes (fsync), reallocating the memory map to make the newly
	/// written data accessible.
	fn sync(&mut self) -> io::Result<()> {
		self.file.sync_data()?;
		self.mmap = Some(unsafe {
			memmap::file(&self.file)
				.protection(memmap::Protection::Read)
				.map()?
		});
		Ok(())
	}

	/// Read length bytes of data at offset from the file. Leverages the memory
	/// map.
	fn read(&self, offset: usize, length: usize) -> Vec<u8> {
		if let None = self.mmap {
			return vec![];
		}
		let mmap = self.mmap.as_ref().unwrap();
		(&mmap[offset..(offset + length)]).to_vec()
	}

	/// Current size of the file in bytes.
	fn size(&self) -> io::Result<u64> {
		metadata(&self.path).map(|md| md.len())
	}
}

/// Log file fully cached in memory containing all positions that should be
/// eventually removed from the MMR append-only data file. Allows quick
/// checking of whether a piece of data has been marked for deletion. When the
/// log becomes too long, the MMR backend will actually remove chunks from the
/// MMR data file and truncate the remove log.
struct RemoveLog {
	file: File,
	// Ordered vector of MMR positions that should get eventually removed.
	removed: Vec<u64>,
}

impl RemoveLog {
	/// Open the remove log file. The content of the file will be read in memory
	/// for fast checking.
	fn open(path: String) -> io::Result<RemoveLog> {
		let log_path = Path::new(&path);
		let mut removed = Vec::with_capacity(1000);
		if log_path.exists() {
			let mut file = BufReader::with_capacity(8 * 1000, File::open(path.clone())?);
			loop {
				// need a block to end mutable borrow before consume
				let buf_len = {
					let buf = file.fill_buf()?;
					if buf.len() == 0 {
						break;
					}
					let elmts_res: Result<Vec<u64>, ser::Error> = ser::deserialize(&mut &buf[..]);
					match elmts_res {
						Ok(elmts) => {
							for elmt in elmts {
								if let Err(idx) = removed.binary_search(&elmt) {
									removed.insert(idx, elmt);
								}
							}
						}
						Err(_) => {
							return Err(io::Error::new(
								io::ErrorKind::InvalidData,
								"Corrupted storage, could not read remove log.",
							));
						}
					}
					buf.len()
				};
				file.consume(buf_len);
			}
		}

		let file = OpenOptions::new().append(true).create(true).open(
			path.clone(),
		)?;
		Ok(RemoveLog {
			file: file,
			removed: removed,
		})
	}

	/// Append a set of new positions to the remove log. Both adds those
	/// positions
	/// to the ordered in-memory set and to the file.
	fn append(&mut self, elmts: Vec<u64>) -> io::Result<()> {
		for elmt in elmts {
			match self.removed.binary_search(&elmt) {
				Ok(_) => continue,
				Err(idx) => {
					self.file.write_all(&ser::ser_vec(&elmt).unwrap()[..])?;
					self.removed.insert(idx, elmt);
				}
			}
		}
		self.file.sync_data()
	}

	/// Whether the remove log currently includes the provided position.
	fn includes(&self, elmt: u64) -> bool {
		self.removed.binary_search(&elmt).is_ok()
	}

	/// Number of positions stored in the remove log.
	fn len(&self) -> usize {
		self.removed.len()
	}
}

/// PMMR persistent backend implementation. Relies on multiple facilities to
/// handle writing, reading and pruning.
///
/// * A main storage file appends HashSum instances as they come. This
/// AppendOnlyFile is also backed by a mmap for reads.
/// * An in-memory backend buffers the latest batch of writes to ensure the
/// PMMR can always read recent values even if they haven't been flushed to
/// disk yet.
/// * A remove log tracks the positions that need to be pruned from the
/// main storage file.
pub struct PMMRBackend<T>
where
	T: Summable + Clone,
{
	hashsum_file: AppendOnlyFile,
	remove_log: RemoveLog,
	// buffers addition of new elements until they're fully written to disk
	buffer: VecBackend<T>,
	buffer_index: usize,
}

impl<T> Backend<T> for PMMRBackend<T>
where
	T: Summable + Clone,
{
	/// Append the provided HashSums to the backend storage.
	#[allow(unused_variables)]
	fn append(&mut self, position: u64, data: Vec<HashSum<T>>) -> Result<(), String> {
		self.buffer.append(
			position - (self.buffer_index as u64),
			data.clone(),
		)?;
		for hs in data {
			if let Err(e) = self.hashsum_file.append(&ser::ser_vec(&hs).unwrap()[..]) {
				return Err(format!(
					"Could not write to log storage, disk full? {:?}",
					e
				));
			}
		}
		Ok(())
	}

	/// Get a HashSum by insertion position
	fn get(&self, position: u64) -> Option<HashSum<T>> {
		// First, check if it's in our temporary write buffer
		let pos_sz = position as usize;
		if pos_sz - 1 >= self.buffer_index && pos_sz - 1 < self.buffer_index + self.buffer.len() {
			return self.buffer.get((pos_sz - self.buffer_index) as u64);
		}

		// The MMR starts at 1, our backend starts at 0
		let pos = position - 1;

		// Second, check if this position has been pruned in the remove log
		if self.remove_log.includes(pos) {
			return None;
		}

		// TODO check skip list

		// Must be on disk, doing a read at the correct position
		let record_len = 32 + T::sum_len();
		let data = self.hashsum_file.read(
			(pos as usize) * record_len,
			record_len,
		);
		match ser::deserialize(&mut &data[..]) {
			Ok(hashsum) => Some(hashsum),
			Err(e) => {
				error!(
					"Corrupted storage, could not read an entry from sum tree store: {:?}",
					e
				);
				None
			}
		}
	}

	/// Remove HashSums by insertion position
	fn remove(&mut self, positions: Vec<u64>) -> Result<(), String> {
		self.buffer.remove(positions.clone());
		self.remove_log.append(positions).map_err(|e| {
			format!("Could not write to log storage, disk full? {:?}", e)
		})
	}
}

impl<T> PMMRBackend<T>
where
	T: Summable + Clone,
{
	/// Instantiates a new PMMR backend that will use the provided directly to
	/// store its files.
	pub fn new(data_dir: String) -> io::Result<PMMRBackend<T>> {
		let hs_file = AppendOnlyFile::open(format!("{}/{}", data_dir, PMMR_DATA_FILE))?;
		let sz = hs_file.size()?;
		let record_len = 32 + T::sum_len();
		let rm_log = RemoveLog::open(format!("{}/{}", data_dir, PMMR_RM_LOG_FILE))?;

		Ok(PMMRBackend {
			hashsum_file: hs_file,
			remove_log: rm_log,
			buffer: VecBackend::new(),
			buffer_index: (sz as usize) / record_len,
		})
	}

	/// Syncs all files to disk. A call to sync is required to ensure all the
	/// data
	/// has been successfully written to disk.
	pub fn sync(&mut self) -> io::Result<()> {
		self.buffer_index = self.buffer_index + self.buffer.len();
		self.buffer.clear();

		self.hashsum_file.sync()
	}
}
