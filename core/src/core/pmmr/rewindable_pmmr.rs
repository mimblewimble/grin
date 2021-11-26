// Copyright 2021 The Grin Developers
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

//! Rewindable (but still readonly) view of a PMMR.
//! Only supports non-pruneable backends (i.e. kernel MMR backend).

use std::marker;

use crate::core::pmmr::{round_up_to_leaf_pos, Backend, ReadonlyPMMR};
use crate::ser::PMMRable;

/// Rewindable (but still readonly) view of a PMMR.
pub struct RewindablePMMR<'a, T, B>
where
	T: PMMRable,
	B: Backend<T>,
{
	/// The last position in the PMMR
	last_pos: u64,
	/// The backend for this readonly PMMR
	backend: &'a B,
	// only needed to parameterise Backend
	_marker: marker::PhantomData<T>,
}

impl<'a, T, B> RewindablePMMR<'a, T, B>
where
	T: PMMRable,
	B: 'a + Backend<T>,
{
	/// Build a new readonly PMMR.
	pub fn new(backend: &'a B) -> RewindablePMMR<'_, T, B> {
		RewindablePMMR {
			backend,
			last_pos: 0,
			_marker: marker::PhantomData,
		}
	}

	/// Build a new readonly PMMR pre-initialized to
	/// last_pos with the provided backend.
	pub fn at(backend: &'a B, last_pos: u64) -> RewindablePMMR<'_, T, B> {
		RewindablePMMR {
			backend,
			last_pos,
			_marker: marker::PhantomData,
		}
	}

	/// Note: We only rewind the last_pos, we do not rewind the (readonly) backend.
	/// Prunable backends are not supported here.
	pub fn rewind(&mut self, position: u64) -> Result<(), String> {
		// Identify which actual position we should rewind to as the provided
		// position is a leaf. We traverse the MMR to include any parent(s) that
		// need to be included for the MMR to be valid.
		self.last_pos = round_up_to_leaf_pos(position);
		Ok(())
	}

	/// Allows conversion of a "rewindable" PMMR into a "readonly" PMMR.
	/// Intended usage is to create a rewindable PMMR, rewind it,
	/// then convert to "readonly" and read from it.
	pub fn as_readonly(&self) -> ReadonlyPMMR<'a, T, B> {
		ReadonlyPMMR::at(&self.backend, self.last_pos)
	}
}
