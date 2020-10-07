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

//! Segment of a PMMR.

use crate::core::hash::Hash;
use crate::core::pmmr::{self, Backend, ReadablePMMR, ReadonlyPMMR};
use crate::ser::{Error, PMMRIndexHashable, PMMRable, Readable, Reader, Writeable, Writer};
use croaring::Bitmap;
use std::cmp::min;
use std::fmt::{self, Debug};

#[derive(Clone, Debug, PartialEq, Eq)]
/// Error related to segment creation or validation
pub enum SegmentError {
	/// An expected leaf was missing
	MissingLeaf(u64),
	/// An expected hash was missing
	MissingHash(u64),
	/// The segment does not exist
	NonExistent,
	/// The segment is fully pruned
	Pruned,
	/// Mismatch between expected and actual root hash
	Mismatch,
}

impl fmt::Display for SegmentError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			SegmentError::MissingLeaf(idx) => write!(f, "Missing leaf at pos {}", idx),
			SegmentError::MissingHash(idx) => write!(f, "Missing hash at pos {}", idx),
			SegmentError::NonExistent => write!(f, "Segment does not exist"),
			SegmentError::Pruned => write!(f, "Segment is fully pruned"),
			SegmentError::Mismatch => write!(f, "Root hash mismatch"),
		}
	}
}

/// Tuple that defines a segment of a given PMMR
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct SegmentIdentifier {
	/// 2-Log of the size of a segment
	pub log_size: u8,
	/// Zero-based index of the segment
	pub idx: u64,
}

impl Readable for SegmentIdentifier {
	fn read<R: Reader>(reader: &mut R) -> Result<Self, Error> {
		let log_size = reader.read_u8()?;
		let idx = reader.read_u64()?;
		Ok(Self { log_size, idx })
	}
}

impl Writeable for SegmentIdentifier {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), Error> {
		writer.write_u8(self.log_size)?;
		writer.write_u64(self.idx)
	}
}

/// Segment of a PMMR: unpruned leaves and the necessary data to verify
/// segment membership in the original MMR.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Segment<T> {
	identifier: SegmentIdentifier,
	hashes: Vec<(u64, Hash)>,
	leaf_data: Vec<(u64, T)>,
	proof: SegmentProof,
}

impl<T> Segment<T> {
	/// Maximum number of leaves in a segment, given by `2**b`
	#[inline]
	fn segment_capacity(&self) -> u64 {
		1 << self.identifier.log_size
	}

	/// Offset (in leaf idx) of first leaf in the segment
	#[inline]
	fn leaf_offset(&self) -> u64 {
		self.identifier.idx * self.segment_capacity()
	}

	/// Number of leaves in this segment. Equal to capacity except for the final segment, which can be smaller
	#[inline]
	fn segment_unpruned_size(&self, last_pos: u64) -> u64 {
		min(
			self.segment_capacity(),
			pmmr::n_leaves(last_pos).saturating_sub(self.leaf_offset()),
		)
	}

	/// Whether the segment is full (size == capacity)
	#[inline]
	fn full_segment(&self, last_pos: u64) -> bool {
		self.segment_unpruned_size(last_pos) == self.segment_capacity()
	}

	/// Inclusive range of MMR positions for this segment
	#[inline]
	fn segment_pos_range(&self, last_pos: u64) -> (u64, u64) {
		let segment_size = self.segment_unpruned_size(last_pos);
		let leaf_offset = self.leaf_offset();
		let first = pmmr::insertion_to_pmmr_index(leaf_offset + 1);
		let last = if self.full_segment(last_pos) {
			pmmr::insertion_to_pmmr_index(leaf_offset + segment_size)
				+ (self.identifier.log_size as u64)
		} else {
			last_pos
		};

		(first, last)
	}

	fn get_hash(&self, pos: u64) -> Result<Hash, SegmentError> {
		self.hashes
			.binary_search_by(|&(p, _)| p.cmp(&pos))
			.map(|p| self.hashes[p].1)
			.map_err(|_| SegmentError::MissingHash(pos))
	}
}

impl<T> Segment<T>
where
	T: Readable + Writeable + Debug,
{
	/// Generate a segment from a PMMR
	pub fn from_pmmr<U, B>(
		segment_id: SegmentIdentifier,
		pmmr: &ReadonlyPMMR<'_, U, B>,
		prunable: bool,
	) -> Result<Self, SegmentError>
	where
		U: PMMRable<E = T>,
		B: Backend<U>,
	{
		let mut segment = Segment {
			identifier: segment_id,
			hashes: Vec::new(),
			leaf_data: Vec::new(),
			proof: SegmentProof::empty(),
		};

		let last_pos = pmmr.unpruned_size();
		if segment.segment_unpruned_size(last_pos) == 0 {
			return Err(SegmentError::NonExistent);
		}

		// Fill leaf data and hashes
		let (segment_first_pos, segment_last_pos) = segment.segment_pos_range(last_pos);
		for pos in segment_first_pos..=segment_last_pos {
			if pmmr::is_leaf(pos) {
				if let Some(data) = pmmr.get_data_from_file(pos) {
					segment.leaf_data.push((pos, data));
					continue;
				} else if !prunable {
					return Err(SegmentError::MissingLeaf(pos));
				}
			}
			// TODO: optimize, no need to send every intermediary hash
			if prunable {
				if let Some(hash) = pmmr.get_from_file(pos) {
					segment.hashes.push((pos, hash));
				}
			}
		}

		if segment.leaf_data.is_empty() {
			return Err(SegmentError::Pruned);
		}

		// Segment merkle proof
		segment.proof =
			SegmentProof::generate(pmmr, last_pos, segment_first_pos, segment_last_pos)?;

		Ok(segment)
	}
}

impl<T> Segment<T>
where
	T: PMMRIndexHashable,
{
	/// Calculate root hash of this segment
	pub fn root(&self, last_pos: u64, bitmap: Option<&Bitmap>) -> Result<Hash, SegmentError> {
		let (segment_first_pos, segment_last_pos) = self.segment_pos_range(last_pos);
		let mut hashes =
			Vec::<Option<Hash>>::with_capacity(2 * (self.identifier.log_size as usize));
		let mut leaves = self.leaf_data.iter();
		for pos in segment_first_pos..=segment_last_pos {
			let height = pmmr::bintree_postorder_height(pos);
			let hash = if height == 0 {
				// Leaf
				if bitmap
					.map(|b| {
						let idx_1 = pmmr::n_leaves(pos);
						let idx_2 = if pmmr::is_left_sibling(pos) {
							idx_1 + 1
						} else {
							idx_1 - 1
						};
						b.contains(idx_1 as u32) || b.contains(idx_2 as u32)
					})
					.unwrap_or(true)
				{
					// We require the data of this leaf if either the mmr is not prunable or if
					// the bitmap indicates it (or its sibling) should be here
					// TODO: possibly remove requirement on the sibling when we no longer support
					//  syncing through the txhashset.zip method
					let data = leaves
						.find(|&&(p, _)| p == pos)
						.map(|(_, l)| l)
						.ok_or_else(|| SegmentError::MissingLeaf(pos))?;
					Some(data.hash_with_index(pos - 1))
				} else {
					None
				}
			} else {
				let left_child_pos = pos - (1 << height);
				let right_child_pos = pos - 1;

				let right_child = hashes.pop().unwrap();
				let left_child = hashes.pop().unwrap();

				if bitmap.is_some() {
					// Prunable MMR
					match (left_child, right_child) {
						(None, None) => None,
						(Some(l), Some(r)) => Some((l, r).hash_with_index(pos - 1)),
						(None, Some(r)) => {
							let l = self.get_hash(left_child_pos)?;
							Some((l, r).hash_with_index(pos - 1))
						}
						(Some(l), None) => {
							let r = self.get_hash(right_child_pos)?;
							Some((l, r).hash_with_index(pos - 1))
						}
					}
				} else {
					// Non-prunable MMR: require both children
					Some(
						(
							left_child.ok_or_else(|| SegmentError::MissingHash(left_child_pos))?,
							right_child
								.ok_or_else(|| SegmentError::MissingHash(right_child_pos))?,
						)
							.hash_with_index(pos - 1),
					)
				}
			};
			hashes.push(hash);
		}

		if self.full_segment(last_pos) {
			// Full segment: last position of segment is subtree root
			hashes
				.pop()
				.flatten()
				.ok_or_else(|| SegmentError::MissingHash(segment_last_pos))
		} else {
			// Not full (only final segment): peaks in segment, bag them together
			let peaks = pmmr::peaks(last_pos)
				.into_iter()
				.filter(|&pos| pos >= segment_first_pos && pos <= segment_last_pos)
				.rev();
			let mut hash = None;
			for pos in peaks {
				let mut lhash = hashes.pop().ok_or_else(|| SegmentError::MissingHash(pos))?;
				if lhash.is_none() && bitmap.is_some() {
					// If this entire peak is pruned, load it from the segment hashes
					lhash = Some(self.get_hash(pos)?);
				}
				let lhash = lhash.ok_or_else(|| SegmentError::MissingHash(pos))?;

				hash = match hash {
					None => Some(lhash),
					Some(rhash) => Some((lhash, rhash).hash_with_index(last_pos)),
				};
			}
			hash.ok_or_else(|| SegmentError::MissingHash(0))
		}
	}

	/// Check validity of the segment by calculating its root and validating the merkle proof
	pub fn validate(
		&self,
		last_pos: u64,
		bitmap: Option<&Bitmap>,
		mmr_root: Hash,
	) -> Result<(), SegmentError> {
		let (first, last) = self.segment_pos_range(last_pos);
		let segment_root = self.root(last_pos, bitmap)?;
		self.proof
			.validate(last_pos, mmr_root, first, last, segment_root)
	}
}

impl<T: Readable> Readable for Segment<T> {
	fn read<R: Reader>(reader: &mut R) -> Result<Self, Error> {
		let identifier = Readable::read(reader)?;

		let n_hashes = reader.read_u64()? as usize;
		let mut hashes = Vec::with_capacity(n_hashes);
		for _ in 0..n_hashes {
			let pos = reader.read_u64()?;
			let hash: Hash = Readable::read(reader)?;
			hashes.push((pos, hash));
		}

		let n_leaves = reader.read_u64()? as usize;
		let mut leaf_data = Vec::with_capacity(n_leaves);
		for _ in 0..n_leaves {
			let pos = reader.read_u64()?;
			let data: T = Readable::read(reader)?;
			leaf_data.push((pos, data));
		}

		let proof = Readable::read(reader)?;

		Ok(Self {
			identifier,
			hashes,
			leaf_data,
			proof,
		})
	}
}

impl<T: Writeable> Writeable for Segment<T> {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), Error> {
		Writeable::write(&self.identifier, writer)?;
		writer.write_u64(self.hashes.len() as u64)?;
		for (pos, hash) in &self.hashes {
			writer.write_u64(*pos)?;
			Writeable::write(hash, writer)?;
		}
		writer.write_u64(self.leaf_data.len() as u64)?;
		for (pos, data) in &self.leaf_data {
			writer.write_u64(*pos)?;
			Writeable::write(data, writer)?;
		}
		Writeable::write(&self.proof, writer)?;
		Ok(())
	}
}

/// Merkle proof of a segment
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SegmentProof {
	hashes: Vec<Hash>,
}

impl SegmentProof {
	fn empty() -> Self {
		Self { hashes: Vec::new() }
	}

	fn generate<U, B>(
		pmmr: &ReadonlyPMMR<'_, U, B>,
		last_pos: u64,
		segment_first_pos: u64,
		segment_last_pos: u64,
	) -> Result<Self, SegmentError>
	where
		U: PMMRable,
		B: Backend<U>,
	{
		let family_branch = pmmr::family_branch(segment_last_pos, last_pos);

		// 1. siblings along the path from the subtree root to the peak
		let hashes: Result<Vec<_>, _> = family_branch
			.iter()
			.map(|&(_, s)| pmmr.get_hash(s).ok_or_else(|| SegmentError::MissingHash(s)))
			.collect();
		let mut proof = Self { hashes: hashes? };

		// 2. bagged peaks to the right
		let peak_pos = family_branch
			.last()
			.map(|&(p, _)| p)
			.unwrap_or(segment_last_pos);
		if let Some(h) = pmmr.bag_the_rhs(peak_pos) {
			proof.hashes.push(h);
		}

		// 3. peaks to the left
		let peaks: Result<Vec<_>, _> = pmmr::peaks(last_pos)
			.into_iter()
			.filter(|&x| x < segment_first_pos)
			.rev()
			.map(|p| pmmr.get_hash(p).ok_or_else(|| SegmentError::MissingHash(p)))
			.collect();
		proof.hashes.extend(peaks?);

		Ok(proof)
	}

	/// Reconstruct PMMR root using this proof
	pub fn reconstruct_root(
		&self,
		last_pos: u64,
		segment_first_pos: u64,
		segment_last_pos: u64,
		segment_root: Hash,
	) -> Result<Hash, SegmentError> {
		let mut iter = self.hashes.iter();
		let family_branch = pmmr::family_branch(segment_last_pos, last_pos);

		// 1. siblings along the path from the subtree root to the peak
		let mut root = segment_root;
		for &(p, s) in &family_branch {
			let sibling_hash = iter.next().ok_or_else(|| SegmentError::MissingHash(s))?;
			root = if pmmr::is_left_sibling(s) {
				(sibling_hash, root).hash_with_index(p - 1)
			} else {
				(root, sibling_hash).hash_with_index(p - 1)
			};
		}

		// 2. bagged peaks to the right
		let peak_pos = family_branch
			.last()
			.map(|&(p, _)| p)
			.unwrap_or(segment_last_pos);

		let rhs = pmmr::peaks(last_pos)
			.into_iter()
			.filter(|&x| x > peak_pos)
			.next();

		if let Some(pos) = rhs {
			root = (
				root,
				iter.next().ok_or_else(|| SegmentError::MissingHash(pos))?,
			)
				.hash_with_index(last_pos)
		}

		// 3. peaks to the left
		let peaks = pmmr::peaks(last_pos)
			.into_iter()
			.filter(|&x| x < segment_first_pos)
			.rev();
		for pos in peaks {
			root = (
				iter.next().ok_or_else(|| SegmentError::MissingHash(pos))?,
				root,
			)
				.hash_with_index(last_pos);
		}

		Ok(root)
	}

	/// Check validity of the proof by equating the reconstructed root with the actual root
	pub fn validate(
		&self,
		last_pos: u64,
		mmr_root: Hash,
		segment_first_pos: u64,
		segment_last_pos: u64,
		segment_root: Hash,
	) -> Result<(), SegmentError> {
		let root =
			self.reconstruct_root(last_pos, segment_first_pos, segment_last_pos, segment_root)?;
		if root == mmr_root {
			Ok(())
		} else {
			Err(SegmentError::Mismatch)
		}
	}
}

impl Readable for SegmentProof {
	fn read<R: Reader>(reader: &mut R) -> Result<Self, Error> {
		let n_hashes = reader.read_u64()? as usize;
		let mut hashes = Vec::with_capacity(n_hashes);
		for _ in 0..n_hashes {
			let hash: Hash = Readable::read(reader)?;
			hashes.push(hash);
		}
		Ok(Self { hashes })
	}
}

impl Writeable for SegmentProof {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), Error> {
		writer.write_u64(self.hashes.len() as u64)?;
		for hash in &self.hashes {
			Writeable::write(hash, writer)?;
		}
		Ok(())
	}
}
