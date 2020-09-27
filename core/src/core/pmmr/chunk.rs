// Copyright 2020 The Grin Developers
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

use crate::core::hash::Hash;
use crate::core::pmmr::{self, Backend, ReadonlyPMMR};
use crate::ser::{PMMRIndexHashable, PMMRable, Readable, Writeable};
use croaring::Bitmap;
use std::cmp::min;
use std::collections::HashMap;
use std::fmt::{self, Debug};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChunkError {
	MissingLeaf(u64),
	MissingHash(u64),
	Empty,
	NotFound,
}

impl fmt::Display for ChunkError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			ChunkError::MissingLeaf(idx) => write!(f, "Missing leaf at pos {}", idx),
			ChunkError::MissingHash(idx) => write!(f, "Missing hash at pos {}", idx),
			ChunkError::Empty => write!(f, "Chunk is empty"),
			ChunkError::NotFound => write!(f, "Chunk not found"),
		}
	}
}

#[derive(Copy, Clone)]
pub struct ChunkIdentifier {
	pub block_hash: Hash,
	pub log_size: u8,
	pub idx: u64,
}

pub struct Chunk<T> {
	identifier: ChunkIdentifier,
	pub hashes: HashMap<u64, Hash>,
	pub leaf_data: HashMap<u64, T>,
	proof: ChunkProof,
}

impl<T> Chunk<T>
where
	T: Readable + Writeable + Debug,
{
	pub fn from_pmmr<U, B>(
		chunk_id: ChunkIdentifier,
		pmmr: ReadonlyPMMR<'_, U, B>,
	) -> Result<Self, ChunkError>
	where
		U: PMMRable<E = T>,
		B: Backend<U>,
	{
		let mut chunk = Chunk {
			identifier: chunk_id,
			hashes: HashMap::new(),
			leaf_data: HashMap::new(),
			proof: ChunkProof { hashes: Vec::new() },
		};

		let last_pos = pmmr.unpruned_size();
		if chunk.chunk_size(last_pos) == 0 {
			return Err(ChunkError::NotFound);
		}

		// Fill leave data and hashes
		let (chunk_first_pos, chunk_last_pos) = chunk.chunk_pos_range(last_pos);
		for pos in chunk_first_pos..=chunk_last_pos {
			if pmmr::is_leaf(pos) {
				if let Some(data) = pmmr.get_data(pos) {
					chunk.leaf_data.insert(pos, data);
				}
			}
			// TODO: optimize, no need to send every intermediary hash
			if let Some(hash) = pmmr.get_hash(pos) {
				chunk.hashes.insert(pos, hash);
			}
		}

		// Chunk merkle proof
		let family_branch = pmmr::family_branch(chunk_last_pos, last_pos);

		// 1. siblings along the path from the subtree root to the peak
		let hashes: Result<Vec<_>, _> = family_branch
			.iter()
			.map(|&(_, s)| pmmr.get_hash(s).ok_or_else(|| ChunkError::MissingHash(s)))
			.collect();
		chunk.proof.hashes = hashes?;

		// 2. bagged peaks to the right
		let peak_pos = family_branch
			.last()
			.map(|&(p, _)| p)
			.unwrap_or(chunk_last_pos);
		if let Some(h) = pmmr.bag_the_rhs(peak_pos) {
			chunk.proof.hashes.push(h);
		}

		// 3. peaks to the left
		let peaks: Result<Vec<_>, _> = pmmr::peaks(last_pos)
			.into_iter()
			.filter(|x| *x < peak_pos)
			.map(|p| pmmr.get_hash(p).ok_or_else(|| ChunkError::MissingHash(p)))
			.collect();
		let mut peaks = peaks?;
		peaks.reverse();
		chunk.proof.hashes.extend(peaks);

		Ok(chunk)
	}
}

impl<T> Chunk<T> {
	/// Maximum number of leaves in a chunk, given by `2**b`
	#[inline]
	fn chunk_capacity(&self) -> u64 {
		1u64 << self.identifier.log_size
	}

	/// Offset (in leaf idx) of first leaf in the chunk
	#[inline]
	fn leaf_offset(&self) -> u64 {
		self.identifier.idx * self.chunk_capacity()
	}

	/// Number of leaves in this chunk. Equal to capacity except for the final chunk, which can be smaller
	#[inline]
	fn chunk_size(&self, last_pos: u64) -> u64 {
		min(
			self.chunk_capacity(),
			pmmr::n_leaves(last_pos).saturating_sub(self.leaf_offset()),
		)
	}

	/// Inclusive range of MMR positions for this chunk
	#[inline]
	fn chunk_pos_range(&self, last_pos: u64) -> (u64, u64) {
		let chunk_size = self.chunk_size(last_pos);
		let leaf_offset = self.leaf_offset();
		let first = pmmr::insertion_to_pmmr_index(leaf_offset + 1);
		let last = if self.chunk_capacity() < chunk_size {
			last_pos
		} else {
			pmmr::insertion_to_pmmr_index(leaf_offset + chunk_size)
				+ (self.identifier.log_size as u64)
		};

		(first, last)
	}
}

impl<T> Chunk<T>
where
	T: PMMRIndexHashable,
{
	/// Calculate root hash of this chunk
	pub fn root(&self, last_pos: u64, bitmap: Option<&Bitmap>) -> Result<Hash, ChunkError> {
		let (chunk_first_pos, chunk_last_pos) = self.chunk_pos_range(last_pos);
		let mut hashes = HashMap::with_capacity(2 * (self.identifier.log_size as usize + 1));
		for pos in chunk_first_pos..=chunk_last_pos {
			let height = pmmr::bintree_postorder_height(pos);
			if height == 0 {
				// Leaf
				if bitmap
					.map(|b| b.contains((pmmr::n_leaves(pos) - 1) as u32))
					.unwrap_or(true)
				{
					// We require the data of this leaf if either the mmr is not prunable or if
					// the bitmap indicates it should be here
					let data = self
						.leaf_data
						.get(&pos)
						.ok_or_else(|| ChunkError::MissingLeaf(pos))?;
					hashes.insert(pos, data.hash_with_index(pos - 1));
				};
			} else {
				let left_child_pos = pos - (1 << height);
				let right_child_pos = pos - 1;

				let left_child = hashes.remove(&left_child_pos);
				let right_child = hashes.remove(&left_child_pos);

				// TODO: edge cases
				let (left_child, right_child) = if let Some(b) = bitmap {
					// Prunable MMR
					let l = left_child.or_else(|| self.hashes.get(&left_child_pos).map(|h| *h));
					let r = right_child.or_else(|| self.hashes.get(&right_child_pos).map(|h| *h));
					match (l, r) {
						(Some(l), Some(r)) => (l, r),
						(None, Some(_)) if height > 1 => {
							return Err(ChunkError::MissingHash(left_child_pos))
						}
						(Some(_), None) if height > 1 => {
							return Err(ChunkError::MissingHash(right_child_pos))
						}
						_ => continue,
					}
				} else {
					// Non-prunable MMR
					(
						left_child.ok_or_else(|| ChunkError::MissingHash(left_child_pos))?,
						right_child.ok_or_else(|| ChunkError::MissingHash(right_child_pos))?,
					)
				};

				let hash = (left_child, right_child).hash_with_index(pos - 1);
				hashes.insert(pos, hash);
			}
		}

		// TODO: bag peaks for final chunk
		hashes
			.remove(&chunk_last_pos)
			.ok_or_else(|| ChunkError::MissingHash(chunk_last_pos))
	}

	pub fn validate(&self, last_pos: u64, bitmap: Option<&Bitmap>, mmr_root: Hash) -> bool {
		if let Ok(chunk_root) = self.root(last_pos, bitmap) {
			let (_, chunk_root_pos) = self.chunk_pos_range(last_pos);
			self.proof
				.validate(last_pos, mmr_root, chunk_root_pos, chunk_root)
		} else {
			false
		}
	}
}

pub struct ChunkProof {
	hashes: Vec<Hash>,
}

impl ChunkProof {
	pub fn validate(
		&self,
		last_pos: u64,
		mmr_root: Hash,
		chunk_root_pos: u64,
		chunk_root: Hash,
	) -> bool {
		unimplemented!()
	}
}
