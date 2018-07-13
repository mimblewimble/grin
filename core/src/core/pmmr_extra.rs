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
use ser::{PMMRIndexHashable, PMMRable};
use util::LOGGER;

pub trait ExtraBackend<T>
where
	T: PMMRable,
{
	fn get(&self, position: u64) -> Option<T>;
}

impl<'a, T, B> PMMRExtra<'a, T, B>
where
	T: PMMRable + ::std::fmt::Debug,
	B: 'a + ExtraBackend<T>,
{
	pub fn new(backend: &'a mut B) -> PMMRExtra<T, B> {
		PMMRExtra {
			last_pos: 0,
			backend: backend,
			_marker: marker::PhantomData,
		}
	}

	pub fn at(backend: &'a mut B, last_pos: u64) -> PMMRExtra<T, B> {
		PMMRExtra {
			last_pos: last_pos,
			backend: backend,
			_marker: marker::PhantomData,
		}
	}
}

pub struct PMMRExtra<'a, T, B>
where
	T: PMMRable,
	B: 'a + ExtraBackend<T>,
{
	/// The last position in the PMMR
	pub last_pos: u64,
	backend: &'a mut B,
	// only needed to parameterise Backend
	_marker: marker::PhantomData<T>,
}
