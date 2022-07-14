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

use std::cmp::min;
use std::convert::TryFrom;
use std::time::Instant;

use bit_vec::BitVec;
use croaring::Bitmap;

use crate::core::core::hash::{DefaultHashable, Hash};
use crate::core::core::pmmr::segment::{Segment, SegmentIdentifier, SegmentProof};
use crate::core::core::pmmr::{self, Backend, ReadablePMMR, ReadonlyPMMR, VecBackend, PMMR};
use crate::core::ser::{self, PMMRable, Readable, Reader, Writeable, Writer};
use crate::error::Error;
use enum_primitive::FromPrimitive;

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
	const NBITS: u64 = BitmapChunk::LEN_BITS as u64;

	/// Crate a new empty bitmap accumulator.
	pub fn new() -> BitmapAccumulator {
		BitmapAccumulator {
			backend: VecBackend::new(),
		}
	}

	/// Initialize a bitmap accumulator given the provided idx iterator.
	pub fn init<T: IntoIterator<Item = u64>>(&mut self, idx: T, size: u64) -> Result<(), Error> {
		self.apply_from(idx, 0, size)
	}

	/// Find the start of the first "chunk" of 1024 bits from the provided idx.
	/// Zero the last 10 bits to round down to multiple of 1024.
	pub fn chunk_start_idx(idx: u64) -> u64 {
		idx & !(Self::NBITS - 1)
	}

	/// The first 1024 belong to chunk 0, the next 1024 to chunk 1 etc.
	fn chunk_idx(idx: u64) -> u64 {
		idx / Self::NBITS
	}

	/// Apply the provided idx iterator to our bitmap accumulator.
	/// We start at the chunk containing from_idx and rebuild chunks as necessary
	/// for the bitmap, limiting it to size (in bits).
	/// If from_idx is 1023 and size is 1024 then we rebuild a single chunk.
	fn apply_from<T>(&mut self, idx: T, from_idx: u64, size: u64) -> Result<(), Error>
	where
		T: IntoIterator<Item = u64>,
	{
		let now = Instant::now();

		// Find the (1024 bit chunk) chunk_idx for the (individual bit) from_idx.
		let from_chunk_idx = BitmapAccumulator::chunk_idx(from_idx);
		let mut chunk_idx = from_chunk_idx;

		let mut chunk = BitmapChunk::new();

		let mut idx_iter = idx.into_iter().filter(|&x| x < size).peekable();
		while let Some(x) = idx_iter.peek() {
			if *x < chunk_idx * Self::NBITS {
				// NOTE we never get here if idx starts from from_idx
				// skip until we reach our first chunk
				idx_iter.next();
			} else if *x < (chunk_idx + 1) * Self::NBITS {
				let idx = idx_iter.next().expect("next after peek");
				chunk.set(idx % Self::NBITS, true);
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
	/// an iterator of idx to be set to true.
	/// We determine the existing chunks to be rebuilt given the invalidated idx.
	/// We then rebuild given idx, extending the accumulator with new chunk(s) as necessary.
	/// Resulting bitmap accumulator will contain sufficient bitmap chunks to cover size.
	/// If size is 1 then we will have a single chunk.
	/// If size is 1023 then we will have a single chunk (bits 0 to 1023 inclusive).
	/// If the size is 1024 then we will have two chunks.
	/// TODO: first argument is an iterator for no good reason;
	/// might as well pass from_idx as first argument
	pub fn apply<T, U>(&mut self, invalidated_idx: T, idx: U, size: u64) -> Result<(), Error>
	where
		T: IntoIterator<Item = u64>,
		U: IntoIterator<Item = u64>,
	{
		// Determine the earliest chunk by looking at the min invalidated idx (assume sorted).
		// Rewind prior to this and reapply new_idx.
		// Note: We rebuild everything after rewind point but much of the bitmap may be
		// unchanged. This can be further optimized by only rebuilding necessary chunks and
		// rehashing.
		if let Some(from_idx) = invalidated_idx.into_iter().next() {
			self.rewind_prior(from_idx)?;
			self.pad_left(from_idx)?;
			self.apply_from(idx, from_idx, size)?;
		}

		Ok(())
	}

	/// Given the provided (bit) idx rewind the bitmap accumulator to the end of the
	/// previous chunk ready for the updated chunk to be appended.
	fn rewind_prior(&mut self, from_idx: u64) -> Result<(), Error> {
		let chunk_idx = BitmapAccumulator::chunk_idx(from_idx);
		let last_pos = self.backend.size();
		let mut pmmr = PMMR::at(&mut self.backend, last_pos);
		let rewind_pos = pmmr::insertion_to_pmmr_index(chunk_idx);
		pmmr.rewind(rewind_pos, &Bitmap::create())
			.map_err(Error::Other)?;
		Ok(())
	}

	/// Make sure we append empty chunks to fill in any gap before we append the chunk
	/// we actually care about. This effectively pads the bitmap with 1024 chunks of 0s
	/// as necessary to put the new chunk at the correct place.
	fn pad_left(&mut self, from_idx: u64) -> Result<(), Error> {
		let chunk_idx = BitmapAccumulator::chunk_idx(from_idx);
		let current_chunk_idx = pmmr::n_leaves(self.backend.size());
		for _ in current_chunk_idx..chunk_idx {
			self.append_chunk(BitmapChunk::new())?;
		}
		Ok(())
	}

	/// Append a new chunk to the BitmapAccumulator.
	/// Append parent hashes (if any) as necessary to build associated peak.
	pub fn append_chunk(&mut self, chunk: BitmapChunk) -> Result<u64, Error> {
		let last_pos = self.backend.size();
		PMMR::at(&mut self.backend, last_pos)
			.push(&chunk)
			.map_err(Error::Other)
	}

	/// The root hash of the bitmap accumulator MMR.
	pub fn root(&self) -> Hash {
		self.readonly_pmmr().root().expect("no root, invalid tree")
	}

	/// Readonly access to our internal data.
	pub fn readonly_pmmr(&self) -> ReadonlyPMMR<BitmapChunk, VecBackend<BitmapChunk>> {
		ReadonlyPMMR::at(&self.backend, self.backend.size())
	}

	/// Return a raw in-memory bitmap of this accumulator
	pub fn as_bitmap(&self) -> Result<Bitmap, Error> {
		let mut bitmap = Bitmap::create();
		for (chunk_index, chunk_pos) in self.backend.leaf_pos_iter().enumerate() {
			//TODO: Unwrap
			let chunk = self.backend.get_data(chunk_pos as u64).unwrap();
			let additive = chunk.set_iter(chunk_index * 1024).collect::<Vec<u32>>();
			bitmap.add_many(&additive);
		}
		Ok(bitmap)
	}
}

/// A bitmap "chunk" representing 1024 contiguous bits of the overall bitmap.
/// The first 1024 bits belong in one chunk. The next 1024 bits in the next chunk, etc.
#[derive(Clone, Debug, PartialEq, Eq)]
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
	/// Panics if idx is outside the valid range of bits in a chunk.
	pub fn set(&mut self, idx: u64, value: bool) {
		let idx = usize::try_from(idx).expect("usize from u64");
		assert!(idx < Self::LEN_BITS);
		self.0.set(idx, value)
	}

	/// Does this bitmap chunk have any bits set to 1?
	pub fn any(&self) -> bool {
		self.0.any()
	}

	/// Iterator over the integer set represented by this chunk, applying the given
	/// offset to the values
	pub fn set_iter(&self, idx_offset: usize) -> impl Iterator<Item = u32> + '_ {
		self.0
			.iter()
			.enumerate()
			.filter(|(_, val)| *val)
			.map(move |(idx, _)| (idx as u32 + idx_offset as u32))
	}
}

impl PMMRable for BitmapChunk {
	type E = Self;

	fn as_elmt(&self) -> Self::E {
		self.clone()
	}

	fn elmt_size() -> Option<u16> {
		Some(Self::LEN_BYTES as u16)
	}
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
	fn read<R: Reader>(_reader: &mut R) -> Result<BitmapChunk, ser::Error> {
		Ok(BitmapChunk::new())
	}
}

///
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BitmapSegment {
	identifier: SegmentIdentifier,
	blocks: Vec<BitmapBlock>,
	proof: SegmentProof,
}

impl Writeable for BitmapSegment {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		Writeable::write(&self.identifier, writer)?;
		writer.write_u16(self.blocks.len() as u16)?;
		for block in &self.blocks {
			Writeable::write(block, writer)?;
		}
		Writeable::write(&self.proof, writer)?;
		Ok(())
	}
}

impl Readable for BitmapSegment {
	fn read<R: Reader>(reader: &mut R) -> Result<Self, ser::Error> {
		let identifier: SegmentIdentifier = Readable::read(reader)?;

		let n_blocks = reader.read_u16()? as usize;
		let mut blocks = Vec::<BitmapBlock>::with_capacity(n_blocks);
		for _ in 0..n_blocks {
			blocks.push(Readable::read(reader)?);
		}
		let proof = Readable::read(reader)?;

		Ok(Self {
			identifier,
			blocks,
			proof,
		})
	}
}

// TODO: this can be sped up with some `unsafe` code
impl From<Segment<BitmapChunk>> for BitmapSegment {
	fn from(segment: Segment<BitmapChunk>) -> Self {
		let (identifier, _, _, _, leaf_data, proof) = segment.parts();

		let mut chunks_left = leaf_data.len();
		let mut blocks =
			Vec::with_capacity((chunks_left + BitmapBlock::NCHUNKS - 1) / BitmapBlock::NCHUNKS);
		while chunks_left > 0 {
			let n_chunks = min(BitmapBlock::NCHUNKS, chunks_left);
			chunks_left = chunks_left.saturating_sub(n_chunks);
			blocks.push(BitmapBlock::new(n_chunks));
		}

		for (chunk_idx, chunk) in leaf_data.into_iter().enumerate() {
			assert_eq!(chunk.0.len(), BitmapChunk::LEN_BITS);
			let block = &mut blocks
				.get_mut(chunk_idx / BitmapBlock::NCHUNKS)
				.unwrap()
				.inner;
			let offset = (chunk_idx % BitmapBlock::NCHUNKS) * BitmapChunk::LEN_BITS;
			for (i, _) in chunk.0.iter().enumerate().filter(|&(_, v)| v) {
				block.set(offset + i, true);
			}
		}

		Self {
			identifier,
			blocks,
			proof,
		}
	}
}

// TODO: this can be sped up with some `unsafe` code
impl From<BitmapSegment> for Segment<BitmapChunk> {
	fn from(segment: BitmapSegment) -> Self {
		let BitmapSegment {
			identifier,
			blocks,
			proof,
		} = segment;

		// Count the number of chunks taking into account that the final block might be smaller
		let n_chunks = (blocks.len() - 1) * BitmapBlock::NCHUNKS
			+ blocks.last().map(|b| b.n_chunks()).unwrap_or(0);
		let mut leaf_pos = Vec::with_capacity(n_chunks);
		let mut chunks = Vec::with_capacity(n_chunks);
		let offset = (1 << identifier.height) * identifier.idx;
		for i in 0..(n_chunks as u64) {
			leaf_pos.push(pmmr::insertion_to_pmmr_index(offset + i));
			chunks.push(BitmapChunk::new());
		}

		for (block_idx, block) in blocks.into_iter().enumerate() {
			assert!(block.inner.len() <= BitmapBlock::NBITS as usize);
			let offset = block_idx * BitmapBlock::NCHUNKS;
			for (i, _) in block.inner.iter().enumerate().filter(|&(_, v)| v) {
				chunks
					.get_mut(offset + i / BitmapChunk::LEN_BITS)
					.unwrap()
					.0
					.set(i % BitmapChunk::LEN_BITS, true);
			}
		}

		Segment::from_parts(identifier, Vec::new(), Vec::new(), leaf_pos, chunks, proof)
	}
}

/// A block of 2^16 bits that provides an efficient (de)serialization
/// depending on the bitmap occupancy.
#[derive(Clone, Debug, PartialEq, Eq)]
struct BitmapBlock {
	inner: BitVec,
}

impl BitmapBlock {
	/// Maximum number of bits in a block
	const NBITS: u32 = 1 << 16;
	/// Maximum number of chunks in a block
	const NCHUNKS: usize = Self::NBITS as usize / BitmapChunk::LEN_BITS;

	fn new(n_chunks: usize) -> Self {
		assert!(n_chunks <= BitmapBlock::NCHUNKS);
		Self {
			inner: BitVec::from_elem(n_chunks * BitmapChunk::LEN_BITS, false),
		}
	}

	fn n_chunks(&self) -> usize {
		let length = self.inner.len();
		assert_eq!(length % BitmapChunk::LEN_BITS, 0);
		let n_chunks = length / BitmapChunk::LEN_BITS;
		assert!(n_chunks <= BitmapBlock::NCHUNKS);
		n_chunks
	}
}

impl Writeable for BitmapBlock {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		let length = self.inner.len();
		assert!(length <= Self::NBITS as usize);
		assert_eq!(length % BitmapChunk::LEN_BITS, 0);
		writer.write_u8((length / BitmapChunk::LEN_BITS) as u8)?;

		let count_pos = self.inner.iter().filter(|&v| v).count() as u32;

		// Negative count needs to be adjusted if the block is not full,
		// which affects the choice of serialization mode and size written
		let count_neg = length as u32 - count_pos;

		let threshold = Self::NBITS / 16;
		if count_pos < threshold {
			// Write positive indices
			Writeable::write(&BitmapBlockSerialization::Positive, writer)?;
			writer.write_u16(count_pos as u16)?;
			for (i, _) in self.inner.iter().enumerate().filter(|&(_, v)| v) {
				writer.write_u16(i as u16)?;
			}
		} else if count_neg < threshold {
			// Write negative indices
			Writeable::write(&BitmapBlockSerialization::Negative, writer)?;
			writer.write_u16(count_neg as u16)?;
			for (i, _) in self.inner.iter().enumerate().filter(|&(_, v)| !v) {
				writer.write_u16(i as u16)?;
			}
		} else {
			// Write raw bytes
			Writeable::write(&BitmapBlockSerialization::Raw, writer)?;
			let bytes = self.inner.to_bytes();
			assert!(bytes.len() <= Self::NBITS as usize / 8);
			writer.write_fixed_bytes(&bytes)?;
		}

		Ok(())
	}
}

impl Readable for BitmapBlock {
	fn read<R: Reader>(reader: &mut R) -> Result<Self, ser::Error> {
		let n_chunks = reader.read_u8()?;
		if n_chunks as usize > BitmapBlock::NCHUNKS {
			return Err(ser::Error::TooLargeReadErr);
		}
		let n_bits = n_chunks as usize * BitmapChunk::LEN_BITS;

		let mode = Readable::read(reader)?;
		let inner = match mode {
			BitmapBlockSerialization::Raw => {
				// Raw bytes
				let bytes = reader.read_fixed_bytes(n_bits / 8)?;
				BitVec::from_bytes(&bytes)
			}
			BitmapBlockSerialization::Positive => {
				// Positive indices
				let mut inner = BitVec::from_elem(n_bits, false);
				let n = reader.read_u16()?;
				for _ in 0..n {
					inner.set(reader.read_u16()? as usize, true);
				}
				inner
			}
			BitmapBlockSerialization::Negative => {
				// Negative indices
				let mut inner = BitVec::from_elem(n_bits, true);
				let n = reader.read_u16()?;
				for _ in 0..n {
					inner.set(reader.read_u16()? as usize, false);
				}
				inner
			}
		};

		Ok(BitmapBlock { inner })
	}
}

enum_from_primitive! {
	#[derive(Debug, Clone, Copy, PartialEq)]
	#[repr(u8)]
	enum BitmapBlockSerialization {
		Raw = 0,
		Positive = 1,
		Negative = 2,
	}
}

impl Writeable for BitmapBlockSerialization {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		writer.write_u8(*self as u8)
	}
}

impl Readable for BitmapBlockSerialization {
	fn read<R: Reader>(reader: &mut R) -> Result<Self, ser::Error> {
		Self::from_u8(reader.read_u8()?).ok_or(ser::Error::CorruptedData)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::core::ser::{
		BinReader, BinWriter, DeserializationMode, ProtocolVersion, Readable, Writeable,
	};
	use byteorder::ReadBytesExt;
	use grin_util::secp::rand::Rng;
	use rand::thread_rng;
	use std::io::Cursor;

	fn test_roundtrip(entries: usize, inverse: bool, encoding: u8, length: usize, n_blocks: usize) {
		let mut rng = thread_rng();
		let mut block = BitmapBlock::new(n_blocks);
		if inverse {
			block.inner.negate();
		}

		let range_size = n_blocks * BitmapChunk::LEN_BITS as usize;

		// Flip `entries` bits in random spots
		let mut count = 0;
		while count < entries {
			let idx = rng.gen_range(0, range_size);
			if block.inner.get(idx).unwrap() == inverse {
				count += 1;
				block.inner.set(idx, !inverse);
			}
		}

		// Serialize
		let mut cursor = Cursor::new(Vec::<u8>::new());
		let mut writer = BinWriter::new(&mut cursor, ProtocolVersion(1));
		Writeable::write(&block, &mut writer).unwrap();

		// Check encoding type and length
		cursor.set_position(1);
		assert_eq!(cursor.read_u8().unwrap(), encoding);
		let actual_length = cursor.get_ref().len();
		assert_eq!(actual_length, length);
		assert!(actual_length <= 2 + BitmapBlock::NBITS as usize / 8);

		// Deserialize
		cursor.set_position(0);
		let mut reader = BinReader::new(
			&mut cursor,
			ProtocolVersion(1),
			DeserializationMode::default(),
		);
		let block2: BitmapBlock = Readable::read(&mut reader).unwrap();
		assert_eq!(block, block2);
	}

	#[test]
	fn block_ser_roundtrip() {
		let threshold = BitmapBlock::NBITS as usize / 16;
		let entries = thread_rng().gen_range(threshold, 4 * threshold);
		test_roundtrip(entries, false, 0, 2 + BitmapBlock::NBITS as usize / 8, 64);
		test_roundtrip(entries, true, 0, 2 + BitmapBlock::NBITS as usize / 8, 64);
	}

	#[test]
	fn sparse_block_ser_roundtrip() {
		let entries =
			thread_rng().gen_range(BitmapChunk::LEN_BITS, BitmapBlock::NBITS as usize / 16);
		test_roundtrip(entries, false, 1, 4 + 2 * entries, 64);
	}

	#[test]
	fn sparse_unfull_block_ser_roundtrip() {
		let entries =
			thread_rng().gen_range(BitmapChunk::LEN_BITS, BitmapBlock::NBITS as usize / 16);
		test_roundtrip(entries, false, 1, 4 + 2 * entries, 61);
	}

	#[test]
	fn abdundant_block_ser_roundtrip() {
		let entries =
			thread_rng().gen_range(BitmapChunk::LEN_BITS, BitmapBlock::NBITS as usize / 16);
		test_roundtrip(entries, true, 2, 4 + 2 * entries, 64);
	}

	#[test]
	fn abdundant_unfull_block_ser_roundtrip() {
		let entries =
			thread_rng().gen_range(BitmapChunk::LEN_BITS, BitmapBlock::NBITS as usize / 16);
		test_roundtrip(entries, true, 2, 4 + 2 * entries, 61);
	}
}
