// Copyright 2018 The Grin Developers
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

use self::core::core::hash::Hash;
use self::core::core::pmmr::{self, Backend};
use self::core::core::BlockHeader;
use self::core::ser;
use self::core::ser::{FixedLength, PMMRable, Readable, Reader, Writeable, Writer};
use croaring;
use croaring::Bitmap;
use grin_core as core;
use std::path::Path;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct TestElem(pub [u32; 4]);

impl FixedLength for TestElem {
	const LEN: usize = 16;
}

impl PMMRable for TestElem {
	type E = Self;

	fn as_elmt(&self) -> Self::E {
		self.clone()
	}
}

impl Writeable for TestElem {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		r#try!(writer.write_u32(self.0[0]));
		r#try!(writer.write_u32(self.0[1]));
		r#try!(writer.write_u32(self.0[2]));
		writer.write_u32(self.0[3])
	}
}

impl Readable for TestElem {
	fn read(reader: &mut dyn Reader) -> Result<TestElem, ser::Error> {
		Ok(TestElem([
			reader.read_u32()?,
			reader.read_u32()?,
			reader.read_u32()?,
			reader.read_u32()?,
		]))
	}
}

/// Simple MMR backend implementation based on a Vector. Pruning does not
/// compact the Vec itself.
#[derive(Clone, Debug)]
pub struct VecBackend<T: PMMRable> {
	/// Backend elements
	pub data: Vec<T>,
	pub hashes: Vec<Hash>,
	/// Positions of removed elements
	pub remove_list: Vec<u64>,
}

impl<T: PMMRable> Backend<T> for VecBackend<T> {
	fn append(&mut self, data: &T, hashes: Vec<Hash>) -> Result<(), String> {
		self.data.push(data.clone());
		self.hashes.append(&mut hashes.clone());
		Ok(())
	}

	fn get_hash(&self, position: u64) -> Option<Hash> {
		if self.remove_list.contains(&position) {
			None
		} else {
			self.get_from_file(position)
		}
	}

	fn get_data(&self, position: u64) -> Option<T::E> {
		if self.remove_list.contains(&position) {
			None
		} else {
			self.get_data_from_file(position)
		}
	}

	fn get_from_file(&self, position: u64) -> Option<Hash> {
		let hash = &self.hashes[(position - 1) as usize];
		Some(hash.clone())
	}

	fn get_data_from_file(&self, position: u64) -> Option<T::E> {
		let idx = pmmr::n_leaves(position);
		let data = self.data[(idx - 1) as usize].clone();
		Some(data.as_elmt())
	}

	fn remove(&mut self, position: u64) -> Result<(), String> {
		self.remove_list.push(position);
		Ok(())
	}

	fn rewind(&mut self, position: u64, _rewind_rm_pos: &Bitmap) -> Result<(), String> {
		let idx = pmmr::n_leaves(position);
		self.data = self.data[0..(idx as usize) + 1].to_vec();
		self.hashes = self.hashes[0..(position as usize) + 1].to_vec();
		Ok(())
	}

	fn snapshot(&self, _header: &BlockHeader) -> Result<(), String> {
		Ok(())
	}

	fn get_data_file_path(&self) -> &Path {
		Path::new("")
	}

	fn release_files(&mut self) {}

	fn dump_stats(&self) {}
}

impl<T: PMMRable> VecBackend<T> {
	/// Instantiates a new VecBackend<T>
	pub fn new() -> VecBackend<T> {
		VecBackend {
			data: vec![],
			hashes: vec![],
			remove_list: vec![],
		}
	}
}
