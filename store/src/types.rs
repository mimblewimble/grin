// Copyright 2021 The Grin Developers
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
use tempfile::tempfile;

use crate::core::ser::{
	self, BinWriter, DeserializationMode, ProtocolVersion, Readable, Reader, StreamingReader,
	Writeable, Writer,
};
use std::fmt::Debug;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Seek, SeekFrom, Write};
use std::marker;
use std::path::{Path, PathBuf};

/// Represents a single entry in the size_file.
/// Offset (in bytes) and size (in bytes) of a variable sized entry
/// in the corresponding data_file.
/// i.e. To read a single entry from the data_file at position p, read
/// the entry in the size_file to obtain the offset (and size) and then
/// read those bytes from the data_file.
#[derive(Clone, Debug)]
pub struct SizeEntry {
	/// Offset (bytes) in the corresponding data_file.
	pub offset: u64,
	/// Size (bytes) in the corresponding data_file.
	pub size: u16,
}

impl SizeEntry {
	/// Length of a size entry (8 + 2 bytes) for convenience.
	pub const LEN: u16 = 8 + 2;
}

impl Readable for SizeEntry {
	fn read<R: Reader>(reader: &mut R) -> Result<SizeEntry, ser::Error> {
		Ok(SizeEntry {
			offset: reader.read_u64()?,
			size: reader.read_u16()?,
		})
	}
}

impl Writeable for SizeEntry {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		writer.write_u64(self.offset)?;
		writer.write_u16(self.size)?;
		Ok(())
	}
}

/// Are we dealing with "fixed size" data or "variable size" data in a data file?
pub enum SizeInfo {
	/// Fixed size data.
	FixedSize(u16),
	/// Variable size data.
	VariableSize(Box<AppendOnlyFile<SizeEntry>>),
}

/// Data file (MMR) wrapper around an append-only file.
pub struct DataFile<T> {
	file: AppendOnlyFile<T>,
}

impl<T> DataFile<T>
where
	T: Readable + Writeable + Debug,
{
	/// Open (or create) a file at the provided path on disk.
	pub fn open<P>(
		path: P,
		size_info: SizeInfo,
		version: ProtocolVersion,
	) -> io::Result<DataFile<T>>
	where
		P: AsRef<Path> + Debug,
	{
		Ok(DataFile {
			file: AppendOnlyFile::open(path, size_info, version)?,
		})
	}

	/// Append an element to the file.
	/// Will not be written to disk until flush() is subsequently called.
	/// Alternatively discard() may be called to discard any pending changes.
	pub fn append(&mut self, data: &T) -> io::Result<u64> {
		self.file.append_elmt(data)?;
		Ok(self.size_unsync())
	}

	/// Append a slice of multiple elements to the file.
	/// Will not be written to disk until flush() is subsequently called.
	/// Alternatively discard() may be called to discard any pending changes.
	pub fn extend_from_slice(&mut self, data: &[T]) -> io::Result<u64> {
		self.file.append_elmts(data)?;
		Ok(self.size_unsync())
	}

	/// Read an element from the file by position.
	/// Assumes we have already "shifted" the position to account for pruned data.
	/// Note: PMMR API is 1-indexed, but backend storage is 0-indexed.
	///
	/// Makes no assumptions about the size of the elements in bytes.
	/// Elements can be of variable size (handled internally in the append-only file impl).
	///
	pub fn read(&self, position: u64) -> Option<T> {
		self.file.read_as_elmt(position - 1).ok()
	}

	/// Rewind the backend file to the specified position.
	pub fn rewind(&mut self, position: u64) {
		self.file.rewind(position)
	}

	/// Flush unsynced changes to the file to disk.
	pub fn flush(&mut self) -> io::Result<()> {
		self.file.flush()
	}

	/// Discard any unsynced changes to the file.
	pub fn discard(&mut self) {
		self.file.discard()
	}

	/// Size of the file in number of elements (not bytes).
	pub fn size(&self) -> u64 {
		self.file.size_in_elmts().unwrap_or(0)
	}

	/// Size of the unsync'd file, in elements (not bytes).
	fn size_unsync(&self) -> u64 {
		self.file.size_unsync_in_elmts().unwrap_or(0)
	}

	/// Path of the underlying file
	pub fn path(&self) -> &Path {
		self.file.path()
	}

	/// Drop underlying file handles
	pub fn release(&mut self) {
		self.file.release();
	}

	/// Write the file out to disk, pruning removed elements.
	pub fn write_tmp_pruned(&self, prune_pos: &[u64]) -> io::Result<()> {
		// Need to convert from 1-index to 0-index (don't ask).
		let prune_idx: Vec<_> = prune_pos.iter().map(|x| x - 1).collect();
		self.file.write_tmp_pruned(prune_idx.as_slice())
	}

	/// Replace with file at tmp path.
	/// Rebuild and initialize from new file.
	pub fn replace_with_tmp(&mut self) -> io::Result<()> {
		self.file.replace_with_tmp()
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
pub struct AppendOnlyFile<T> {
	path: PathBuf,
	file: Option<File>,
	size_info: SizeInfo,
	version: ProtocolVersion,
	mmap: Option<memmap::Mmap>,

	// Buffer of unsync'd bytes. These bytes will be appended to the file when flushed.
	buffer: Vec<u8>,
	buffer_start_pos: u64,
	buffer_start_pos_bak: u64,
	_marker: marker::PhantomData<T>,
}

impl AppendOnlyFile<SizeEntry> {
	fn sum_sizes(&self) -> io::Result<u64> {
		let mut sum = 0;
		for pos in 0..self.buffer_start_pos {
			let entry = self.read_as_elmt(pos)?;
			sum += entry.size as u64;
		}
		Ok(sum)
	}
}

impl<T> AppendOnlyFile<T>
where
	T: Debug + Readable + Writeable,
{
	/// Open a file (existing or not) as append-only, backed by a mmap.
	pub fn open<P>(
		path: P,
		size_info: SizeInfo,
		version: ProtocolVersion,
	) -> io::Result<AppendOnlyFile<T>>
	where
		P: AsRef<Path> + Debug,
	{
		let mut aof = AppendOnlyFile {
			file: None,
			path: path.as_ref().to_path_buf(),
			size_info,
			version,
			mmap: None,
			buffer: vec![],
			buffer_start_pos: 0,
			buffer_start_pos_bak: 0,
			_marker: marker::PhantomData,
		};
		aof.init()?;

		// (Re)build the size file if inconsistent with the data file.
		// This will occur during "fast sync" as we do not sync the size_file
		// and must build it locally.
		// And we can *only* do this after init() the data file (so we know sizes).
		let expected_size = aof.size()?;
		if let SizeInfo::VariableSize(ref mut size_file) = &mut aof.size_info {
			if size_file.sum_sizes()? != expected_size {
				aof.rebuild_size_file()?;

				// (Re)init the entire file as we just rebuilt the size_file
				// and things may have changed.
				aof.init()?;
			}
		}

		Ok(aof)
	}

	/// (Re)init an underlying file and its associated memmap.
	/// Taking care to initialize the mmap_offset_cache for each element.
	pub fn init(&mut self) -> io::Result<()> {
		if let SizeInfo::VariableSize(ref mut size_file) = self.size_info {
			size_file.init()?;
		}

		self.file = Some(
			OpenOptions::new()
				.read(true)
				.append(true)
				.create(true)
				.open(self.path.clone())?,
		);

		// If we have a non-empty file then mmap it.
		if self.size()? == 0 {
			self.buffer_start_pos = 0;
		} else {
			self.mmap = Some(unsafe { memmap::Mmap::map(&self.file.as_ref().unwrap())? });
			self.buffer_start_pos = self.size_in_elmts()?;
		}

		Ok(())
	}

	fn size_in_elmts(&self) -> io::Result<u64> {
		match self.size_info {
			SizeInfo::FixedSize(elmt_size) => Ok(self.size()? / elmt_size as u64),
			SizeInfo::VariableSize(ref size_file) => size_file.size_in_elmts(),
		}
	}

	fn size_unsync_in_elmts(&self) -> io::Result<u64> {
		match self.size_info {
			SizeInfo::FixedSize(elmt_size) => {
				Ok(self.buffer_start_pos + (self.buffer.len() as u64 / elmt_size as u64))
			}
			SizeInfo::VariableSize(ref size_file) => size_file.size_unsync_in_elmts(),
		}
	}

	/// Append element to append-only file by serializing it to bytes and appending the bytes.
	fn append_elmt(&mut self, data: &T) -> io::Result<()> {
		let mut bytes = ser::ser_vec(data, self.version)
			.map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
		self.append(&mut bytes)?;
		Ok(())
	}

	/// Iterate over the slice and append each element.
	fn append_elmts(&mut self, data: &[T]) -> io::Result<()> {
		for x in data {
			self.append_elmt(x)?;
		}
		Ok(())
	}

	/// Append data to the file. Until the append-only file is synced, data is
	/// only written to memory.
	pub fn append(&mut self, bytes: &mut [u8]) -> io::Result<()> {
		if let SizeInfo::VariableSize(ref mut size_file) = &mut self.size_info {
			let next_pos = size_file.size_unsync_in_elmts()?;
			let offset = if next_pos == 0 {
				0
			} else {
				let prev_entry = size_file.read_as_elmt(next_pos - 1)?;
				prev_entry.offset + prev_entry.size as u64
			};
			size_file.append_elmt(&SizeEntry {
				offset,
				size: bytes.len() as u16,
			})?;
		}

		self.buffer.extend_from_slice(bytes);
		Ok(())
	}

	// Returns the offset and size of bytes to read.
	// If pos is in the buffer then caller needs to remember to account for this
	// when reading from the buffer.
	fn offset_and_size(&self, pos: u64) -> io::Result<(u64, u16)> {
		match self.size_info {
			SizeInfo::FixedSize(elmt_size) => Ok((pos * elmt_size as u64, elmt_size)),
			SizeInfo::VariableSize(ref size_file) => {
				// Otherwise we need to calculate offset and size from entries in the size_file.
				let entry = size_file.read_as_elmt(pos)?;
				Ok((entry.offset, entry.size))
			}
		}
	}

	/// Rewinds the data file back to a previous position.
	/// We simply "rewind" the buffer_start_pos to the specified position.
	/// Note: We do not currently support rewinding within the buffer itself.
	pub fn rewind(&mut self, pos: u64) {
		if let SizeInfo::VariableSize(ref mut size_file) = &mut self.size_info {
			size_file.rewind(pos);
		}

		if self.buffer_start_pos_bak == 0 {
			self.buffer_start_pos_bak = self.buffer_start_pos;
		}
		self.buffer_start_pos = pos;
	}

	/// Syncs all writes (fsync), reallocating the memory map to make the newly
	/// written data accessible.
	pub fn flush(&mut self) -> io::Result<()> {
		if let SizeInfo::VariableSize(ref mut size_file) = &mut self.size_info {
			// Flush the associated size_file if we have one.
			size_file.flush()?
		}

		if self.buffer_start_pos_bak > 0 {
			// Flushing a rewound state, we need to truncate via set_len() before applying.
			// Drop and recreate, or windows throws an access error
			self.mmap = None;
			self.file = None;
			{
				let file = OpenOptions::new()
					.read(true)
					.create(true)
					.write(true)
					.open(&self.path)?;

				// Set length of the file to truncate it as necessary.
				if self.buffer_start_pos == 0 {
					file.set_len(0)?;
				} else {
					let (offset, size) = self.offset_and_size(self.buffer_start_pos - 1)?;
					file.set_len(offset + size as u64)?;
				};
			}
		}

		{
			let file = OpenOptions::new()
				.read(true)
				.create(true)
				.append(true)
				.open(&self.path)?;
			self.file = Some(file);
			self.buffer_start_pos_bak = 0;
		}

		self.file.as_mut().unwrap().write_all(&self.buffer[..])?;
		self.file.as_mut().unwrap().sync_all()?;

		self.buffer.clear();
		self.buffer_start_pos = self.size_in_elmts()?;

		// Note: file must be non-empty to memory map it
		if self.file.as_ref().unwrap().metadata()?.len() == 0 {
			self.mmap = None;
		} else {
			self.mmap = Some(unsafe { memmap::Mmap::map(&self.file.as_ref().unwrap())? });
		}

		Ok(())
	}

	/// Discard the current non-flushed data.
	pub fn discard(&mut self) {
		if self.buffer_start_pos_bak > 0 {
			// discarding a rewound state, restore the buffer start
			self.buffer_start_pos = self.buffer_start_pos_bak;
			self.buffer_start_pos_bak = 0;
		}

		// Discarding the data file will discard the associated size file if we have one.
		if let SizeInfo::VariableSize(ref mut size_file) = &mut self.size_info {
			size_file.discard();
		}

		self.buffer = vec![];
	}

	/// Read the bytes representing the element at the given position (0-indexed).
	/// Uses the offset cache to determine the offset to read from and the size
	/// in bytes to actually read.
	/// Leverages the memory map.
	pub fn read(&self, pos: u64) -> io::Result<&[u8]> {
		if pos >= self.size_unsync_in_elmts()? {
			return Ok(<&[u8]>::default());
		}
		let (offset, length) = self.offset_and_size(pos)?;
		let res = if pos < self.buffer_start_pos {
			self.read_from_mmap(offset, length)
		} else {
			let (buffer_offset, _) = self.offset_and_size(self.buffer_start_pos)?;
			self.read_from_buffer(offset.saturating_sub(buffer_offset), length)
		};
		Ok(res)
	}

	fn read_as_elmt(&self, pos: u64) -> io::Result<T> {
		let data = self.read(pos)?;
		ser::deserialize(&mut &data[..], self.version, DeserializationMode::default())
			.map_err(|e| io::Error::new(io::ErrorKind::Other, e))
	}

	// Read length bytes starting at offset from the buffer.
	// Return empty vec if we do not have enough bytes in the buffer to read
	// the full length bytes.
	fn read_from_buffer(&self, offset: u64, length: u16) -> &[u8] {
		if self.buffer.len() < (offset as usize + length as usize) {
			<&[u8]>::default()
		} else {
			&self.buffer[(offset as usize)..(offset as usize + length as usize)]
		}
	}

	// Read length bytes starting at offset from the mmap.
	// Return empty vec if we do not have enough bytes in the buffer to read
	// the full length bytes.
	// Return empty vec if we have no mmap currently.
	fn read_from_mmap(&self, offset: u64, length: u16) -> &[u8] {
		if let Some(mmap) = &self.mmap {
			if mmap.len() < (offset as usize + length as usize) {
				<&[u8]>::default()
			} else {
				&mmap[(offset as usize)..(offset as usize + length as usize)]
			}
		} else {
			<&[u8]>::default()
		}
	}

	/// Create a new tempfile containing the contents of this append only file.
	/// This allows callers to see a consistent view of the data without
	/// locking the append only file.
	pub fn as_temp_file(&self) -> io::Result<File> {
		let mut reader = BufReader::new(File::open(&self.path)?);
		let mut writer = BufWriter::new(tempfile()?);
		io::copy(&mut reader, &mut writer)?;

		// Remember to seek back to start of the file as the caller is likely
		// to read this file directly without reopening it.
		writer.seek(SeekFrom::Start(0))?;

		let file = writer.into_inner()?;
		Ok(file)
	}

	fn tmp_path(&self) -> PathBuf {
		self.path.with_extension("tmp")
	}

	/// Saves a copy of the current file content, skipping data at the provided
	/// prune positions. prune_pos must be ordered.
	pub fn write_tmp_pruned(&self, prune_pos: &[u64]) -> io::Result<()> {
		let reader = File::open(&self.path)?;
		let mut buf_reader = BufReader::new(reader);
		let mut streaming_reader = StreamingReader::new(&mut buf_reader, self.version);

		let mut buf_writer = BufWriter::new(File::create(&self.tmp_path())?);
		let mut bin_writer = BinWriter::new(&mut buf_writer, self.version);

		let mut current_pos = 0;
		let mut prune_pos = prune_pos;
		while let Ok(elmt) = T::read(&mut streaming_reader) {
			if prune_pos.contains(&current_pos) {
				// Pruned pos, moving on.
				prune_pos = &prune_pos[1..];
			} else {
				// Not pruned, write to file.
				elmt.write(&mut bin_writer)
					.map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
			}
			current_pos += 1;
		}
		buf_writer.flush()?;
		Ok(())
	}

	/// Replace the underlying file with the file at tmp path.
	/// Rebuild and initialize from the new file.
	pub fn replace_with_tmp(&mut self) -> io::Result<()> {
		// Replace the underlying file -
		// pmmr_data.tmp -> pmmr_data.bin
		self.replace(&self.tmp_path())?;

		// Now rebuild our size file to reflect the pruned data file.
		// This will replace the underlying file internally.
		if let SizeInfo::VariableSize(_) = &self.size_info {
			self.rebuild_size_file()?;
		}

		// Now (re)init the file and associated size_file so everything is consistent.
		self.init()?;

		Ok(())
	}

	fn rebuild_size_file(&mut self) -> io::Result<()> {
		if let SizeInfo::VariableSize(ref mut size_file) = &mut self.size_info {
			// Note: Reading from data file and writing sizes to the associated (tmp) size_file.
			let tmp_path = size_file.path.with_extension("tmp");
			debug!("rebuild_size_file: {:?}", tmp_path);

			// Scope the reader and writer to within the block so we can safely replace files later on.
			{
				let reader = File::open(&self.path)?;
				let mut buf_reader = BufReader::new(reader);
				let mut streaming_reader = StreamingReader::new(&mut buf_reader, self.version);

				let mut buf_writer = BufWriter::new(File::create(&tmp_path)?);
				let mut bin_writer = BinWriter::new(&mut buf_writer, self.version);

				let mut current_offset = 0;
				while let Ok(_) = T::read(&mut streaming_reader) {
					let size = streaming_reader
						.total_bytes_read()
						.saturating_sub(current_offset) as u16;
					let entry = SizeEntry {
						offset: current_offset,
						size,
					};

					// Not pruned, write to file.
					entry
						.write(&mut bin_writer)
						.map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

					current_offset += size as u64;
				}
				buf_writer.flush()?;
			}

			// Replace the underlying file for our size_file -
			// pmmr_size.tmp -> pmmr_size.bin
			size_file.replace(&tmp_path)?;
		}

		Ok(())
	}

	/// Replace the underlying file with another file, deleting the original.
	/// Takes an optional size_file path in addition to path.
	fn replace<P>(&mut self, with: P) -> io::Result<()>
	where
		P: AsRef<Path> + Debug,
	{
		self.release();
		fs::remove_file(&self.path)?;
		fs::rename(with, &self.path)?;
		Ok(())
	}

	/// Release underlying file handles.
	pub fn release(&mut self) {
		self.mmap = None;
		self.file = None;

		// Remember to release the size_file as well if we have one.
		if let SizeInfo::VariableSize(ref mut size_file) = &mut self.size_info {
			size_file.release();
		}
	}

	/// Current size of the file in bytes.
	pub fn size(&self) -> io::Result<u64> {
		fs::metadata(&self.path).map(|md| md.len())
	}

	/// Path of the underlying file
	pub fn path(&self) -> &Path {
		&self.path
	}
}
