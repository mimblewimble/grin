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

use std::marker;

use croaring::Bitmap;

use core::hash::Hash;
use core::merkle_proof::MerkleProof;
use core::BlockHeader;
use core::pmmr::is_leaf;
use ser::{PMMRIndexHashable, PMMRable};
use util::LOGGER;

pub trait ExtraBackend<T>
where
	T: PMMRable,
{
	fn append(&mut self, position: u64, entry: T) -> Result<(), String>;

	fn get(&self, position: u64) -> Option<T>;

	fn rewind(&mut self, position: u64) -> Result<(), String>;
}

pub struct PMMRExtra<'a, T, B>
where
	T: PMMRable,
	B: 'a + ExtraBackend<T>,
{
	backend: &'a mut B,
	// only needed to parameterise Backend
	_marker: marker::PhantomData<T>,
}

impl<'a, T, B> PMMRExtra<'a, T, B>
where
	T: PMMRable + ::std::fmt::Debug,
	B: 'a + ExtraBackend<T>,
{
	pub fn new(backend: &'a mut B) -> PMMRExtra<T, B> {
		PMMRExtra {
			backend,
			_marker: marker::PhantomData,
		}
	}

	pub fn at(backend: &'a mut B) -> PMMRExtra<T, B> {
		PMMRExtra {
			backend,
			_marker: marker::PhantomData,
		}
	}

	pub fn append(&mut self, pos: u64, entry: T) -> Result<(), String> {
		assert!(is_leaf(pos), "extra data only supported for leaf pos");
		self.backend.append(pos, entry)?;
		Ok(())
	}

	/// Get the "extra" data at provided position in the MMR.
	pub fn get(&self, pos: u64) -> Option<T> {
		assert!(is_leaf(pos), "extra data only supported for leaf pos");
		self.backend.get(pos)
	}

	/// Rewind the PMMR to a previous position, as if all push operations after
	/// that had been canceled. Expects a position in the PMMR to rewind to.
	pub fn rewind(&mut self, pos: u64) -> Result<(), String> {
		self.backend.rewind(pos)?;
		Ok(())
	}
}
