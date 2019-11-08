// Copyright 2019 The Grin Developers
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

use std::convert::TryFrom;
use std::time::Instant;

use bit_vec::BitVec;
use croaring::Bitmap;

use crate::core::core::hash::{DefaultHashable, Hash};
use crate::core::core::pmmr::{self, ReadonlyPMMR, VecBackend, PMMR};
use crate::core::ser::{
	self, FixedLength, PMMRIndexHashable, PMMRable, Readable, Reader, Writeable, Writer,
};
use crate::error::{Error, ErrorKind};

/// The "bitmap accumulator" allows us to commit to a specific bitmap by splitting it into
/// fragments and inserting these fragments into an MMR to produce an overall root hash.
/// Leaves in the MMR are fragments of the bitmap consisting of 1024 contiguous bits
/// from the overall bitmap. The first (leftmost) leaf in the MMR represents the first 1024 bits
/// of the bitmap, the next leaf is the next 1024 bits of the bitmap etc.
///
/// Flipping a single bit does not require the full bitmap to be rehashed, only the path from the
/// relevant leaf up to its associated peak.
///
/// Flipping multiple bits *within* a single chunk is no more expensive than flipping a single bit
/// as a leaf node in the MMR represents a sequence of 1024 bits. Flipping multiple bits located
/// close together is a relatively cheap operation with minimal rehashing required to update the
/// relevant peaks and the overall MMR root.
///
/// It is also possible to generate Merkle proofs for these 1024 bit fragments, proving
/// both inclusion and location in the overall "accumulator" MMR. We plan to take advantage of
/// this during fast sync, allowing for validation of partial data.
///
#[derive(Clone)]
pub struct BitmapAccumulator {
	backend: VecBackend<BitmapChunk>,
}

impl BitmapAccumulator {
	/// Crate a new empty bitmap accumulator.
	pub fn new() -> BitmapAccumulator {
		BitmapAccumulator {
			backend: VecBackend::new_hash_only(),
		}
	}

	/// Initialize a bitmap accumulator given the provided idx iterator.
	pub fn init<T: IntoIterator<Item = u64>>(&mut self, idx: T) -> Result<(), Error> {
		self.apply_from(idx, 0)
	}

	/// Apply the provided idx iterator starting at the provided chunk idx.
	fn apply_from<T>(&mut self, idx: T, from_chunk_idx: u64) -> Result<(), Error>
	where
		T: IntoIterator<Item = u64>,
	{
		let now = Instant::now();

		let mut chunk_idx = from_chunk_idx;
		let mut chunk = BitmapChunk::new();

		let mut leaves = idx.into_iter().peekable();
		while let Some(x) = leaves.peek() {
			if *x < chunk_idx * 1024 {
				// skip until we reach our first chunk
				leaves.next();
			} else if *x < (chunk_idx + 1) * 1024 {
				let idx = leaves.next().expect("next after peek");
				chunk.set(idx % 1024, true);
			} else {
				self.append_chunk(chunk)?;
				chunk_idx += 1;
				chunk = BitmapChunk::new();
			}
		}
		if chunk.any() {
			self.append_chunk(chunk)?;
		}
		debug!(
			"applied {} chunks from idx {} to idx {} ({}ms)",
			1 + chunk_idx - from_chunk_idx,
			from_chunk_idx,
			chunk_idx,
			now.elapsed().as_millis(),
		);
		Ok(())
	}

	/// Apply updates to the bitmap accumulator given an iterator of invalidated idx and
	/// an additional iterator of new idx to be added.
	/// We determine the existing chunks to be rebuilt given the invalidated idx.
	/// We then add new idx, extending the accumulator with new chunk(s) as necessary.
	pub fn apply<T, U>(&mut self, invalidated_idx: T, new_idx: U) -> Result<(), Error>
	where
		T: IntoIterator<Item = u64>,
		U: IntoIterator<Item = u64>,
	{
		// Determine the earliest chunk by looking at the min invalidated idx (assume sorted).
		// Rewind prior to this and reapply new_idx.
		// Note: We rebuild everything after rewind point but much of the bitmap may be
		// unchanged. This can be further optimized by only rebuilding necessary chunks and
		// rehashing.
		if let Some(first_chunk_idx) = invalidated_idx.into_iter().next().map(|x| x / 1024) {
			self.rewind_prior(first_chunk_idx)?;
			self.pad_left(first_chunk_idx)?;
			self.apply_from(new_idx, first_chunk_idx)?;
		}

		// Trim any "empty" chunks from the right to keep everything consistent.
		self.trim_right()?;

		Ok(())
	}

	/// Given the provided chunk idx rewind the bitmap accumulator to the end of the
	/// previous chunk ready for the updated chunk to be appended.
	fn rewind_prior(&mut self, chunk_idx: u64) -> Result<(), Error> {
		let last_pos = self.backend.size();
		let mut pmmr = PMMR::at(&mut self.backend, last_pos);
		let chunk_pos = pmmr::insertion_to_pmmr_index(chunk_idx + 1);
		let rewind_pos = chunk_pos.saturating_sub(1);
		pmmr.rewind(rewind_pos, &Bitmap::create())
			.map_err(|e| ErrorKind::Other(e))?;
		Ok(())
	}

	/// Make sure we append empty chunks to fill in any gap before we append the chunk
	/// we actually care about. This effectively pads the bitmap with 1024 chunks of 0s
	/// as necessary to put the new chunk at the correct place.
	fn pad_left(&mut self, chunk_idx: u64) -> Result<(), Error> {
		let last_pos = self.backend.size();
		let chunk_pos = pmmr::insertion_to_pmmr_index(chunk_idx);
		for _ in last_pos..chunk_pos {
			self.append_chunk(BitmapChunk::new())?;
		}
		Ok(())
	}

	/// Recursively trim the last chunk if empty. Stop when rightmost chunk is non-empty
	/// or the accumulator itself is empty.
	fn trim_right(&mut self) -> Result<(), Error> {
		let last_pos = self.backend.size();
		if last_pos == 0 {
			return Ok(());
		}
		let mut pmmr = PMMR::at(&mut self.backend, last_pos);
		if pmmr.get_hash(last_pos)
			== Some(BitmapChunk::new().hash_with_index(last_pos.saturating_sub(1)))
		{
			pmmr.rewind(last_pos.saturating_sub(1), &Bitmap::create())
				.map_err(|e| ErrorKind::Other(e))?;
			self.trim_right()
		} else {
			Ok(())
		}
	}

	/// Append a new chunk to the BitmapAccumulator.
	/// Append parent hashes (if any) as necessary to build associated peak.
	pub fn append_chunk(&mut self, chunk: BitmapChunk) -> Result<u64, Error> {
		let last_pos = self.backend.size();
		PMMR::at(&mut self.backend, last_pos)
			.push(&chunk)
			.map_err(|e| ErrorKind::Other(e).into())
	}

	/// The root hash of the bitmap accumulator MMR.
	pub fn root(&self) -> Hash {
		ReadonlyPMMR::at(&self.backend, self.backend.size()).root()
	}
}

/// A bitmap "chunk" representing 1024 contiguous bits of the overall bitmap.
/// The first 1024 bits belong in one chunk. The next 1024 bits in the next chunk, etc.
#[derive(Clone, Debug)]
pub struct BitmapChunk(BitVec);

impl BitmapChunk {
	const LEN_BITS: usize = 1024;
	const LEN_BYTES: usize = Self::LEN_BITS / 8;

	/// Create a new bitmap chunk, defaulting all bits in the chunk to false.
	pub fn new() -> BitmapChunk {
		BitmapChunk(BitVec::from_elem(Self::LEN_BITS, false))
	}

	/// Set a single bit in this chunk.
	/// 0-indexed from start of chunk.
	pub fn set(&mut self, idx: u64, value: bool) {
		let idx = usize::try_from(idx).expect("usize from u64");
		if idx < Self::LEN_BITS {
			self.0.set(idx, value)
		}
	}

	/// Does this bitmap chunk have any bits set to 1?
	pub fn any(&self) -> bool {
		self.0.any()
	}
}

impl PMMRable for BitmapChunk {
	type E = Self;

	fn as_elmt(&self) -> Self::E {
		self.clone()
	}
}

impl FixedLength for BitmapChunk {
	const LEN: usize = Self::LEN_BYTES;
}

impl DefaultHashable for BitmapChunk {}

impl Writeable for BitmapChunk {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		self.0.to_bytes().write(writer)
	}
}

impl Readable for BitmapChunk {
	/// Reading is not currently supported, just return an empty one for now.
	/// We store the underlying roaring bitmap externally for the bitmap accumulator
	/// and the "hash only" backend means we never actually read these chunks.
	fn read(_reader: &mut dyn Reader) -> Result<BitmapChunk, ser::Error> {
		Ok(BitmapChunk::new())
	}
}
