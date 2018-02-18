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

use std::cmp;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, ErrorKind, Write};
use std::marker::PhantomData;
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::io::Read;

#[cfg(any(target_os = "linux"))]
use libc::{ftruncate64, off64_t};
#[cfg(not(any(target_os = "linux", target_os = "android")))]
use libc::{ftruncate as ftruncate64, off_t as off64_t};

use core::core::pmmr::{self, Backend, HashSum, Summable};
use core::ser;
use util::LOGGER;

const PMMR_DATA_FILE: &'static str = "pmmr_dat.bin";
const PMMR_RM_LOG_FILE: &'static str = "pmmr_rm_log.bin";
const PMMR_PRUNED_FILE: &'static str = "pmmr_pruned.bin";

/// Maximum number of nodes in the remove log before it gets flushed
pub const RM_LOG_MAX_NODES: usize = 10000;

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
		self.file.sync_data()?;
		self.buffer = vec![];
		self.mmap = Some(unsafe { memmap::Mmap::map(&self.file)? });
		Ok(())
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

	/// Read length bytes of data at offset from the file. Leverages the memory
	/// map.
	fn read(&self, offset: usize, length: usize) -> Vec<u8> {
		if offset >= self.buffer_start {
			let offset = offset - self.buffer_start;
			return self.buffer[offset..(offset+length)].to_vec();
		}
		if let None = self.mmap {
			return vec![];
		}
		let mmap = self.mmap.as_ref().unwrap();
		(&mmap[offset..(offset + length)]).to_vec()
	}

	/// Truncates the underlying file to the provided offset
	fn truncate(&self, offs: usize) -> io::Result<()> {
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
	fn save_prune(&self, target: String, prune_offs: Vec<u64>, prune_len: u64) -> io::Result<()> {
		let mut reader = File::open(self.path.clone())?;
		let mut writer = File::create(target)?;

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
				let prune_at = prune_offs[prune_pos] as usize;
				if prune_at != buf_start {
					writer.write_all(&buf[buf_start..prune_at])?;
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

	/// Current size of the file in bytes.
	fn size(&self) -> io::Result<u64> {
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
struct RemoveLog {
	path: String,
	// Ordered vector of MMR positions that should get eventually removed.
	removed: Vec<(u64, u32)>,
	// Holds positions temporarily until flush is called.
	removed_tmp: Vec<(u64, u32)>,
	// Holds truncated removed temporarily until discarded or committed
	removed_bak: Vec<(u64, u32)>,
}

impl RemoveLog {
	/// Open the remove log file. The content of the file will be read in memory
	/// for fast checking.
	fn open(path: String) -> io::Result<RemoveLog> {
		let removed = read_ordered_vec(path.clone(), 12)?;
		Ok(RemoveLog {
			path: path,
			removed: removed,
			removed_tmp: vec![],
			removed_bak: vec![],
		})
	}

	/// Truncate and empties the remove log.
	fn rewind(&mut self, last_offs: u32) -> io::Result<()> {
		// simplifying assumption: we always remove older than what's in tmp
		self.removed_tmp = vec![];

		// backing it up before truncating
		self.removed_bak = self.removed.clone();

		// backing it up before truncating
		self.removed_bak = self.removed.clone();

		if last_offs == 0 {
			self.removed = vec![];
		} else {
			self.removed = self.removed
				.iter()
				.filter(|&&(_, idx)| idx < last_offs)
				.map(|x| *x)
				.collect();
		}
		Ok(())
	}

	/// Append a set of new positions to the remove log. Both adds those
	/// positions the ordered in-memory set and to the file.
	fn append(&mut self, elmts: Vec<u64>, index: u32) -> io::Result<()> {
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
	fn flush(&mut self) -> io::Result<()> {
		let mut file = File::create(self.path.clone())?;
		for elmt in &self.removed_tmp {
			match self.removed.binary_search(&elmt) {
				Ok(_) => continue,
				Err(idx) => {
					self.removed.insert(idx, *elmt);
				}
			}
		}
		for elmt in &self.removed {
			file.write_all(&ser::ser_vec(&elmt).unwrap()[..])?;
		}
		self.removed_tmp = vec![];
		self.removed_bak = vec![];
		file.sync_data()
	}

	/// Discard pending changes
	fn discard(&mut self) {
		if self.removed_bak.len() > 0 {
			self.removed = self.removed_bak.clone();
			self.removed_bak = vec![];
		}
		self.removed_tmp = vec![];
	}

	/// Whether the remove log currently includes the provided position.
	fn includes(&self, elmt: u64) -> bool {
		include_tuple(&self.removed, elmt) || include_tuple(&self.removed_tmp, elmt)
	}

	/// Number of positions stored in the remove log.
	fn len(&self) -> usize {
		self.removed.len()
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
	data_dir: String,
	hashsum_file: AppendOnlyFile,
	remove_log: RemoveLog,
	pruned_nodes: pmmr::PruneList,
	phantom: PhantomData<T>,
}

impl<T> Backend<T> for PMMRBackend<T>
where
	T: Summable + Clone,
{
	/// Append the provided HashSums to the backend storage.
	#[allow(unused_variables)]
	fn append(&mut self, position: u64, data: Vec<HashSum<T>>) -> Result<(), String> {
		for d in data {
			self.hashsum_file.append(&mut ser::ser_vec(&d).unwrap());
		}
		Ok(())
	}

	/// Get a HashSum by insertion position
	fn get(&self, position: u64) -> Option<HashSum<T>> {
		// Check if this position has been pruned in the remove log or the
		// pruned list
		if self.remove_log.includes(position) {
			return None;
		}
		let shift = self.pruned_nodes.get_shift(position);
		if let None = shift {
			return None;
		}

		// The MMR starts at 1, our binary backend starts at 0
		let pos = position - 1;

		// Must be on disk, doing a read at the correct position
		let record_len = 32 + T::sum_len();
		let file_offset = ((pos - shift.unwrap()) as usize) * record_len;
		let data = self.hashsum_file.read(file_offset, record_len);
		match ser::deserialize(&mut &data[..]) {
			Ok(hashsum) => Some(hashsum),
			Err(e) => {
				error!(
					LOGGER,
					"Corrupted storage, could not read an entry from sum tree store: {:?}",
					e
				);
				None
			}
		}
	}

	fn rewind(&mut self, position: u64, index: u32) -> Result<(), String> {
		self.remove_log
			.rewind(index)
			.map_err(|e| format!("Could not truncate remove log: {}", e))?;

		let shift = self.pruned_nodes.get_shift(position).unwrap_or(0);
		let record_len = 32 + T::sum_len();
		let file_pos = (position - shift) * (record_len as u64);
		self.hashsum_file.rewind(file_pos);
		Ok(())
	}

	/// Remove HashSums by insertion position
	fn remove(&mut self, positions: Vec<u64>, index: u32) -> Result<(), String> {
		self.remove_log.append(positions, index).map_err(|e| {
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
		let rm_log = RemoveLog::open(format!("{}/{}", data_dir, PMMR_RM_LOG_FILE))?;
		let prune_list = read_ordered_vec(format!("{}/{}", data_dir, PMMR_PRUNED_FILE), 8)?;

		Ok(PMMRBackend {
			data_dir: data_dir,
			hashsum_file: hs_file,
			remove_log: rm_log,
			pruned_nodes: pmmr::PruneList {
				pruned_nodes: prune_list,
			},
			phantom: PhantomData,
		})
	}

	/// Total size of the PMMR stored by this backend. Only produces the fully
	/// sync'd size.
	pub fn unpruned_size(&self) -> io::Result<u64> {
		let total_shift = self.pruned_nodes.get_shift(::std::u64::MAX).unwrap();
		let record_len = 32 + T::sum_len() as u64;
		let sz = self.hashsum_file.size()?;
		Ok(sz / record_len + total_shift)
	}

	/// Syncs all files to disk. A call to sync is required to ensure all the
	/// data has been successfully written to disk.
	pub fn sync(&mut self) -> io::Result<()> {
		if let Err(e) = self.hashsum_file.flush() {
			return Err(io::Error::new(
					io::ErrorKind::Interrupted,
					format!("Could not write to log storage, disk full? {:?}", e),
					));
		}

		self.remove_log.flush()?;
		Ok(())
	}

	/// Discard the current, non synced state of the backend.
	pub fn discard(&mut self) {
		self.hashsum_file.discard();
		self.remove_log.discard();
	}

	/// Checks the length of the remove log to see if it should get compacted.
	/// If so, the remove log is flushed into the pruned list, which itself gets
	/// saved, and the main hashsum data file is rewritten, cutting the removed
	/// data.
	///
	/// If a max_len strictly greater than 0 is provided, the value will be used
	/// to decide whether the remove log has reached its maximum length,
	/// otherwise the RM_LOG_MAX_NODES default value is used.
	///
	/// TODO whatever is calling this should also clean up the commit to
	/// position index in db
	pub fn check_compact(&mut self, max_len: usize) -> io::Result<()> {
		if !(max_len > 0 && self.remove_log.len() > max_len
			|| max_len == 0 && self.remove_log.len() > RM_LOG_MAX_NODES)
		{
			return Ok(());
		}

		// 0. validate none of the nodes in the rm log are in the prune list (to
		// avoid accidental double compaction)
		for pos in &self.remove_log.removed[..] {
			if let None = self.pruned_nodes.pruned_pos(pos.0) {
				// TODO we likely can recover from this by directly jumping to 3
				error!(
					LOGGER,
					"The remove log contains nodes that are already in the pruned \
					 list, a previous compaction likely failed."
				);
				return Ok(());
			}
		}

		// 1. save hashsum file to a compact copy, skipping data that's in the
		// remove list
		let tmp_prune_file = format!("{}/{}.prune", self.data_dir, PMMR_DATA_FILE);
		let record_len = (32 + T::sum_len()) as u64;
		let to_rm = self.remove_log
			.removed
			.iter()
			.map(|&(pos, _)| {
				let shift = self.pruned_nodes.get_shift(pos);
				(pos - 1 - shift.unwrap()) * record_len
			})
			.collect();
		self.hashsum_file
			.save_prune(tmp_prune_file.clone(), to_rm, record_len)?;

		// 2. update the prune list and save it in place
		for &(rm_pos, _) in &self.remove_log.removed[..] {
			self.pruned_nodes.add(rm_pos);
		}
		write_vec(
			format!("{}/{}", self.data_dir, PMMR_PRUNED_FILE),
			&self.pruned_nodes.pruned_nodes,
		)?;

		// 3. move the compact copy to the hashsum file and re-open it
		fs::rename(
			tmp_prune_file.clone(),
			format!("{}/{}", self.data_dir, PMMR_DATA_FILE),
		)?;
		self.hashsum_file = AppendOnlyFile::open(format!("{}/{}", self.data_dir, PMMR_DATA_FILE))?;

		// 4. truncate the rm log
		self.remove_log.rewind(0)?;
		self.remove_log.flush()?;

		Ok(())
	}
}

// Read an ordered vector of scalars from a file.
fn read_ordered_vec<T>(path: String, elmt_len: usize) -> io::Result<Vec<T>>
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

fn write_vec<T>(path: String, v: &Vec<T>) -> io::Result<()>
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
