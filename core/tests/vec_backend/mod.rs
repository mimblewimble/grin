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

extern crate croaring;

use croaring::Bitmap;

use core::core::BlockHeader;
use core::core::hash::Hash;
use core::core::pmmr::Backend;
use core::ser;
use core::ser::{PMMRable, Readable, Reader, Writeable, Writer};

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct TestElem(pub [u32; 4]);

impl PMMRable for TestElem {
	fn len() -> usize {
		16
	}
}

impl Writeable for TestElem {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		try!(writer.write_u32(self.0[0]));
		try!(writer.write_u32(self.0[1]));
		try!(writer.write_u32(self.0[2]));
		writer.write_u32(self.0[3])
	}
}

impl Readable for TestElem {
	fn read(reader: &mut Reader) -> Result<TestElem, ser::Error> {
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
pub struct VecBackend<T>
where
	T: PMMRable,
{
	/// Backend elements
	pub elems: Vec<Option<(Hash, Option<T>)>>,
	/// Positions of removed elements
	pub remove_list: Vec<u64>,
}

impl<T> Backend<T> for VecBackend<T>
where
	T: PMMRable,
{
	fn append(&mut self, _position: u64, data: Vec<(Hash, Option<T>)>) -> Result<(), String> {
		self.elems.append(&mut map_vec!(data, |d| Some(d.clone())));
		Ok(())
	}

	fn get_hash(&self, position: u64) -> Option<Hash> {
		if self.remove_list.contains(&position) {
			None
		} else {
			if let Some(ref elem) = self.elems[(position - 1) as usize] {
				Some(elem.0)
			} else {
				None
			}
		}
	}

	fn get_data(&self, position: u64) -> Option<T> {
		if self.remove_list.contains(&position) {
			None
		} else {
			if let Some(ref elem) = self.elems[(position - 1) as usize] {
				elem.1.clone()
			} else {
				None
			}
		}
	}

	fn get_from_file(&self, position: u64) -> Option<Hash> {
		if let Some(ref x) = self.elems[(position - 1) as usize] {
			Some(x.0)
		} else {
			None
		}
	}

	fn get_data_from_file(&self, position: u64) -> Option<T> {
		if let Some(ref x) = self.elems[(position - 1) as usize] {
			x.1.clone()
		} else {
			None
		}
	}

	fn remove(&mut self, position: u64) -> Result<(), String> {
		self.remove_list.push(position);
		Ok(())
	}

	fn rewind(
		&mut self,
		position: u64,
		_rewind_rm_pos: &Bitmap,
	) -> Result<(), String> {
		self.elems = self.elems[0..(position as usize) + 1].to_vec();
		Ok(())
	}

	fn snapshot(&self, _header: &BlockHeader) -> Result<(), String> {
		Ok(())
	}

	fn get_data_file_path(&self) -> String {
		"".to_string()
	}

	fn dump_stats(&self) {}
}

impl<T> VecBackend<T>
where
	T: PMMRable,
{
	/// Instantiates a new VecBackend<T>
	pub fn new() -> VecBackend<T> {
		VecBackend {
			elems: vec![],
			remove_list: vec![],
		}
	}

	// /// Current number of elements in the underlying Vec.
	// pub fn used_size(&self) -> usize {
	// 	let mut usz = self.elems.len();
	// 	for (idx, _) in self.elems.iter().enumerate() {
	// 		let idx = idx as u64;
	// 		if self.remove_list.contains(&idx) {
	// 			usz -= 1;
	// 		}
	// 	}
	// 	usz
	// }
}
