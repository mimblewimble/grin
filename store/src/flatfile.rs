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

//! Implementation of an append-only flat file, basically storing a large
//! vector of N items

use core::ser;
use std::marker::PhantomData;
use std::fs::{self};
use std::io::{self};
use util::LOGGER;
use types::{AppendOnlyFile, RemoveLog};

const DATA_FILE: &'static str = "dat.bin";
const RM_LOG_FILE: &'static str = "rm_log.bin";

/// Maximum number of nodes in the remove log before it gets flushed
pub const RM_LOG_MAX_NODES: usize = 10000;

/// Flat file store implementation that stores a large vector of items
/// along with a skip list etc.. TODO: Fill this out
pub struct FlatFileStore<T>
where
	T: ser::Writeable + ser::Readable,
{
	data_dir :String,
	record_len: usize,
	data_file: AppendOnlyFile,
	data_file_path: String,
	remove_log: RemoveLog,
	phantom: PhantomData<T>,
}

impl <T> FlatFileStore<T>
where
	T: ser::Writeable + ser::Readable,
{
	pub fn data_file_path(&self) -> String {
		self.data_file_path.clone()
	}


	/// Append the provided elements to the backend storage.
	#[allow(unused_variables)]
	pub fn append(&mut self, data: Vec<T>) -> Result<(), String> {
		for d in data {
			self.data_file.append(&mut ser::ser_vec(&d).unwrap());
		}
		Ok(())
	}

	/// Get an Element by insertion position
	pub fn get(&self, position: u64) -> Option<T> {
		let shift = self.remove_log.get_shift(position);
		// Doing a read at the correct position
		let file_offset = ((position as usize + shift) as usize) * self.record_len;
		if file_offset + self.record_len > self.data_file.size().unwrap() as usize {
			return None;
		}
		let data = self.data_file.read(file_offset, self.record_len);
		match ser::deserialize(&mut &data[..]) {
			Ok(elem) => Some(elem),
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

	/// Rewind file to position
	pub fn rewind(&mut self, position: u64) -> Result<(), String> {
		/*self.remove_log
			.rewind(index)
			.map_err(|e| format!("Could not truncate remove log: {}", e))?;*/

		let shift = self.remove_log.get_shift(position);
		let file_pos = (position as usize + shift) * self.record_len;
		self.data_file.rewind(file_pos as u64);
		Ok(())
	}

	/// Remove element by insertion position
	pub fn remove(&mut self, positions: Vec<u64>) -> Result<(), String> {
		self.remove_log.append(positions, 0).map_err(|e| {
			format!("Could not write to log storage, disk full? {:?}", e)
		})
	}

	/// Instantiates a new Flatfile Store that will use the provided directly to
	/// store its files.
	pub fn new(data_dir: String, elem_size: usize) -> io::Result<FlatFileStore<T>> {
		let data_file_path = format!("{}/{}", data_dir, DATA_FILE);
		let data_file = AppendOnlyFile::open(data_file_path.clone())?;
		let rm_log = RemoveLog::open(format!("{}/{}", data_dir, RM_LOG_FILE))?;

		Ok(FlatFileStore {
			data_dir: data_dir,
			record_len: elem_size,
			data_file: data_file,
			data_file_path: data_file_path.to_string(),
			remove_log: rm_log,
			phantom: PhantomData,
		})
	}

	/// Syncs all files to disk. A call to sync is required to ensure all the
	/// data has been successfully written to disk.
	pub fn sync(&mut self) -> io::Result<()> {
		if let Err(e) = self.data_file.flush() {
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
		self.data_file.discard();
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

		// 1. save data file to a compact copy, skipping data that's in the
		// remove list
		let tmp_prune_file = format!("{}/{}.prune", self.data_dir, DATA_FILE);
		let to_rm = self.remove_log
			.removed
			.iter()
			.map(|&(pos, _)| {
				let shift = self.remove_log.get_shift(pos);
				(((pos as usize) + shift - 1) * self.record_len) as u64
			})
			.collect();
		self.data_file
			.save_prune(tmp_prune_file.clone(), to_rm, self.record_len as u64)?;

		// 2. move the compact copy to the data file and re-open it
		fs::rename(
			tmp_prune_file.clone(),
			format!("{}/{}", self.data_dir, DATA_FILE),
		)?;

		self.data_file = AppendOnlyFile::open(format!("{}/{}", self.data_dir, DATA_FILE))?;

		// 3. truncate the rm log
		self.remove_log.rewind(0)?;
		self.remove_log.flush()?;

		Ok(())
	}
}
