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

//! Implementation of the persistent Backend for optional "extra" data associated with a PMMR.

use std::{fs, io, marker};

use croaring::Bitmap;

use core::core::hash::{Hash, Hashed};
use core::core::pmmr::{self, family, Backend};
use core::core::pmmr_extra::ExtraBackend;
use core::core::BlockHeader;
use core::ser::{self, PMMRable};
use leaf_set::LeafSet;
use prune_list::PruneList;
use types::{prune_noop, AppendOnlyFile};
use util::LOGGER;


const PMMR_DATA_FILE: &'static str = "pmmr_data.bin";
const PMMR_LEAF_FILE: &'static str = "pmmr_leaf.bin";

pub struct PMMRExtraBackend<T>
where
	T: PMMRable,
{
	data_dir: String,
	data_file: AppendOnlyFile,
	leaf_set: LeafSet,
	_marker: marker::PhantomData<T>,
}

impl<T> ExtraBackend<T> for PMMRExtraBackend<T>
where
	T: PMMRable + ::std::fmt::Debug,
{
	fn get(&self, position: u64) -> Option<T> {
		panic!("not yet implemented...");
	}

	fn rewind(
		&mut self,
		position: u64,
	) -> Result<(), String> {
		self.leaf_set.rewind(position, &Bitmap::create());

		let record_len = T::len() as u64;
		let file_pos = self.leaf_set.cardinality() * record_len;
		self.data_file.rewind(file_pos);

		Ok(())
	}
}

impl<T> PMMRExtraBackend<T>
where
	T: PMMRable,
{
	pub fn new(
		data_dir: String,
	) -> io::Result<PMMRExtraBackend<T>> {
		let data_file = AppendOnlyFile::open(format!("{}/{}", data_dir, PMMR_DATA_FILE))?;

		let leaf_set_path = format!("{}/{}", data_dir, PMMR_LEAF_FILE);
		let leaf_set = LeafSet::open(leaf_set_path.clone())?;

		Ok(PMMRExtraBackend {
			data_dir,
			data_file,
			leaf_set,
			_marker: marker::PhantomData,
		})
	}

	/// Syncs all files to disk. A call to sync is required to ensure all the
	/// data has been successfully written to disk.
	pub fn sync(&mut self) -> io::Result<()> {
		self.data_file.flush()?;
		self.leaf_set.flush()?;
		Ok(())
	}

	/// Discard the current, non synced state of the backend.
	pub fn discard(&mut self) {
		self.leaf_set.discard();
		self.data_file.discard();
	}

	pub fn max_pos(&self) -> u64 {
		self.leaf_set.max_pos()
	}
}
