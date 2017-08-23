// Copyright 2016 The Grin Developers
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

use memmap;

use std::fs::{File, OpenOptions};
use std::io::{self, Write, BufReader, BufRead};
use std::sync::RwLock;

use core::core::pmmr::{PMMR, Summable, Backend, HashSum};
use core::ser::{self, Readable, Reader, Writeable, Writer};

/// Wrapper for a file that can be read at any position (random read) but for
/// which writes are append only. Reads are backed by a memory map (mmap(2)),
/// relying on the operating system for fast access and caching. The memory
/// map is reallocated to expand it when new writes are flushed.
struct AppendOnlyFile {
	file: File,
	mmap: memmap::Mmap,
}

impl AppendOnlyFile {
	/// Open a file (existing or not) as append-only, backed by a mmap.
	fn open(path: String) -> io::Result<AppendOnlyFile> {
		let file = OpenOptions::new().append(true).create(true).open(path.clone())?;
		let mmap = unsafe {
			memmap::file(&file).protection(memmap::Protection::Read).map()?
		};
		Ok(AppendOnlyFile {
			file: file,
			mmap: mmap,
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
		self.mmap = unsafe {
			memmap::file(&self.file).protection(memmap::Protection::Read).map()?
		};
		Ok(())
	}

	/// Read length bytes of data at offset from the file. Leverages the memory
	/// map.
	fn read(&self, offset: usize, length: usize) -> Vec<u8> {
		(&self.mmap[offset..(offset+length)]).to_vec()
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
		let mut file = BufReader::with_capacity(8*1000, File::open(path.clone())?);
		let mut removed = Vec::with_capacity(1000);
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
					},
					Err(_) => {
						return Err(io::Error::new(io::ErrorKind::InvalidData,
																			"Corrupted storage, could not read remove log."));
					}
				}
				buf.len()
			};
			file.consume(buf_len);
		}
	
		let file = OpenOptions::new().append(true).create(true).open(path.clone())?;
		Ok(RemoveLog {
			file: file,
			removed: removed,
		})
	}

	/// Append a set of new positions to the remove log. Both adds those positions
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

pub struct MMRBackend {
	hashsum_file: RwLock<AppendOnlyFile>,
	remove_log: RwLock<RemoveLog>,
}

impl<T> Backend<T> for MMRBackend where T: Summable {
	/// Append the provided HashSums to the backend storage.
	#[allow(unused_variables)]
	fn append(&self, position: u64, data: Vec<HashSum<T>>) -> Result<(), String> {
		let mut f = self.hashsum_file.write().unwrap();
		for hs in data {
			if let Err(e) = f.append(&ser::ser_vec(&hs).unwrap()[..]) {
				return Err(format!("Could not write to log storage, disk full? {:?}", e));
			}
		}
		Ok(())
	}

	/// Get a HashSum by insertion position
	fn get(&self, position: u64) -> Option<HashSum<T>> {
		let log = self.remove_log.read().unwrap();
		if log.includes(position) {
			return None
		}
		// TODO check skip list
		let record_len = 32 + T::sum_len();
		let f = self.hashsum_file.read().unwrap();
		let data = f.read((position as usize)*record_len, record_len);
		match ser::deserialize(&mut &data[..]) {
			Ok(hashsum) => Some(hashsum),
			Err(e) => {
				error!("Corrupted storage, could not read an entry from sum tree store: {:?}", e);
				None
			}
		}
	}

	/// Remove HashSums by insertion position
	fn remove(&self, positions: Vec<u64>) -> Result<(), String> {
		let mut log = self.remove_log.write().unwrap();
		log.append(positions).map_err(|e| {
			format!("Could not write to log storage, disk full? {:?}", e)
		})
	}
}
 
// impl MMRStore<T> where T: Summable {
// 	pub fn root(&self) -> HashSum<T> {
// 	}
// 
// 	pub fn view(&self) -> View<T> {
// 	}
// }
// 
// pub struct View<T: Summable + Writeable> {
//   view_tree: MMR<T>
// }
// 
// impl <T> View<T> where T: Summable {
// 
// 	pub fn root(&self) -> Option<(Hash, T::Sum)> {
//     None
//   }
// 
// 	pub fn push(&mut self, elmt: T) -> u64 {
// 	}
// 
// 	pub fn prune(&self, position: u64) {
// 	}
// 
//   pub fn commit(self) {}
// }
