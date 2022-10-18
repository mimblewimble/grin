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

//! Segment of a PMMR.

use crate::core::hash::Hash;
use crate::core::pmmr::{self, Backend, ReadablePMMR, ReadonlyPMMR};
use crate::ser::{Error, PMMRIndexHashable, PMMRable, Readable, Reader, Writeable, Writer};
use croaring::Bitmap;
use std::cmp::min;
use std::fmt::Debug;

#[derive(Clone, Debug, Eq, PartialEq)]
/// Possible segment types, according to this desegmenter
pub enum SegmentType {
	/// Output Bitmap
	Bitmap,
	/// Output
	Output,
	/// RangeProof
	RangeProof,
	/// Kernel
	Kernel,
}

/// Lumps possible types with segment ids to enable a unique identifier
/// for a segment with respect to a particular archive header
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SegmentTypeIdentifier {
	/// The type of this segment
	pub segment_type: SegmentType,
	/// The identfier itself
	pub identifier: SegmentIdentifier,
}

impl SegmentTypeIdentifier {
	/// Create
	pub fn new(segment_type: SegmentType, identifier: SegmentIdentifier) -> Self {
		Self {
			segment_type,
			identifier,
		}
	}
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
/// Error related to segment creation or validation
pub enum SegmentError {
	/// An expected leaf was missing
	#[error("Missing leaf at pos {0}")]
	MissingLeaf(u64),
	/// An expected hash was missing
	#[error("Missing hash at pos {0}")]
	MissingHash(u64),
	/// The segment does not exist
	#[error("Segment does not exist")]
	NonExistent,
	/// Mismatch between expected and actual root hash
	#[error("Root hash mismatch")]
	Mismatch,
}

/// Tuple that defines a segment of a given PMMR
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct SegmentIdentifier {
	/// Height of a segment
	pub height: u8,
	/// Zero-based index of the segment
	pub idx: u64,
}

impl Readable for SegmentIdentifier {
	fn read<R: Reader>(reader: &mut R) -> Result<Self, Error> {
		let height = reader.read_u8()?;
		let idx = reader.read_u64()?;
		Ok(Self { height, idx })
	}
}

impl Writeable for SegmentIdentifier {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), Error> {
		writer.write_u8(self.height)?;
		writer.write_u64(self.idx)
	}
}

impl SegmentIdentifier {
	/// Test helper to get an iterator of SegmentIdentifiers required to read a
	/// pmmr of size `target_mmr_size` in segments of height `segment_height`
	pub fn traversal_iter(
		target_mmr_size: u64,
		segment_height: u8,
	) -> impl Iterator<Item = SegmentIdentifier> {
		(0..SegmentIdentifier::count_segments_required(target_mmr_size, segment_height)).map(
			move |idx| SegmentIdentifier {
				height: segment_height,
				idx: idx as u64,
			},
		)
	}

	/// Returns number of segments required that would needed in order to read a
	/// pmmr of size `target_mmr_size` in segments of height `segment_height`
	pub fn count_segments_required(target_mmr_size: u64, segment_height: u8) -> usize {
		let d = 1 << segment_height;
		((pmmr::n_leaves(target_mmr_size) + d - 1) / d) as usize
	}

	/// Return pmmr size of number of segments of the given height
	pub fn pmmr_size(num_segments: usize, height: u8) -> u64 {
		pmmr::insertion_to_pmmr_index(num_segments as u64 * (1 << height))
	}

	/// Maximum number of leaves in a segment, given by `2**height`
	pub fn segment_capacity(&self) -> u64 {
		1 << self.height
	}

	/// Offset (in leaf idx) of first leaf in the segment
	fn leaf_offset(&self) -> u64 {
		self.idx * self.segment_capacity()
	}

	// Number of leaves in this segment. Equal to capacity except for the final segment, which can be smaller
	fn segment_unpruned_size(&self, mmr_size: u64) -> u64 {
		min(
			self.segment_capacity(),
			pmmr::n_leaves(mmr_size).saturating_sub(self.leaf_offset()),
		)
	}

	/// Inclusive (full) range of MMR positions for the segment that would be produced
	/// by this Identifier
	pub fn segment_pos_range(&self, mmr_size: u64) -> (u64, u64) {
		let segment_size = self.segment_unpruned_size(mmr_size);
		let leaf_offset = self.leaf_offset();
		let first = pmmr::insertion_to_pmmr_index(leaf_offset);
		let last = if self.full_segment(mmr_size) {
			pmmr::insertion_to_pmmr_index(leaf_offset + segment_size - 1) + (self.height as u64)
		} else {
			mmr_size - 1
		};
		(first, last)
	}

	/// Whether the segment is full (segment size == capacity)
	fn full_segment(&self, mmr_size: u64) -> bool {
		self.segment_unpruned_size(mmr_size) == self.segment_capacity()
	}
}

/// Segment of a PMMR: unpruned leaves and the necessary data to verify
/// segment membership in the original MMR.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Segment<T> {
	identifier: SegmentIdentifier,
	hash_pos: Vec<u64>,
	hashes: Vec<Hash>,
	leaf_pos: Vec<u64>,
	leaf_data: Vec<T>,
	proof: SegmentProof,
}

impl<T> Segment<T> {
	/// Creates an empty segment
	fn empty(identifier: SegmentIdentifier) -> Self {
		Segment {
			identifier,
			hash_pos: Vec::new(),
			hashes: Vec::new(),
			leaf_pos: Vec::new(),
			leaf_data: Vec::new(),
			proof: SegmentProof::empty(),
		}
	}

	/// Maximum number of leaves in a segment, given by `2**height`
	fn _segment_capacity(&self) -> u64 {
		self.identifier.segment_capacity()
	}

	/// Offset (in leaf idx) of first leaf in the segment
	fn _leaf_offset(&self) -> u64 {
		self.identifier.leaf_offset()
	}

	// Number of leaves in this segment. Equal to capacity except for the final segment, which can be smaller
	fn segment_unpruned_size(&self, mmr_size: u64) -> u64 {
		self.identifier.segment_unpruned_size(mmr_size)
	}

	/// Whether the segment is full (segment size == capacity)
	fn full_segment(&self, mmr_size: u64) -> bool {
		self.identifier.full_segment(mmr_size)
	}

	/// Inclusive range of MMR positions for this segment
	pub fn segment_pos_range(&self, mmr_size: u64) -> (u64, u64) {
		self.identifier.segment_pos_range(mmr_size)
	}

	/// TODO - binary_search_by_key() here (can we assume these are sorted by pos?)
	fn get_hash(&self, pos0: u64) -> Result<Hash, SegmentError> {
		self.hash_pos
			.iter()
			.zip(&self.hashes)
			.find(|&(&p, _)| p == pos0)
			.map(|(_, &h)| h)
			.ok_or_else(|| SegmentError::MissingHash(pos0))
	}

	/// Get the identifier associated with this segment
	pub fn identifier(&self) -> SegmentIdentifier {
		self.identifier
	}

	/// Consume the segment and return its parts
	pub fn parts(
		self,
	) -> (
		SegmentIdentifier,
		Vec<u64>,
		Vec<Hash>,
		Vec<u64>,
		Vec<T>,
		SegmentProof,
	) {
		(
			self.identifier,
			self.hash_pos,
			self.hashes,
			self.leaf_pos,
			self.leaf_data,
			self.proof,
		)
	}

	/// Construct a segment from its parts
	pub fn from_parts(
		identifier: SegmentIdentifier,
		hash_pos: Vec<u64>,
		hashes: Vec<Hash>,
		leaf_pos: Vec<u64>,
		leaf_data: Vec<T>,
		proof: SegmentProof,
	) -> Self {
		assert_eq!(hash_pos.len(), hashes.len());
		let mut last = 0;
		for &pos in &hash_pos {
			assert!(last == 0 || pos > last);
			last = pos;
		}
		assert_eq!(leaf_pos.len(), leaf_data.len());
		last = 0;
		for &pos in &leaf_pos {
			assert!(last == 0 || pos > last);
			last = pos;
		}

		Self {
			identifier,
			hash_pos,
			hashes,
			leaf_pos,
			leaf_data,
			proof,
		}
	}

	/// Iterator of all the leaves in the segment
	pub fn leaf_iter(&self) -> impl Iterator<Item = (u64, &T)> + '_ {
		self.leaf_pos.iter().map(|&p| p).zip(&self.leaf_data)
	}

	/// Iterator of all the hashes in the segment
	pub fn hash_iter(&self) -> impl Iterator<Item = (u64, Hash)> + '_ {
		self.hash_pos
			.iter()
			.zip(&self.hashes)
			.map(|(&p, &h)| (p, h))
	}

	/// Segment proof
	pub fn proof(&self) -> &SegmentProof {
		&self.proof
	}

	/// Segment identifier
	pub fn id(&self) -> SegmentIdentifier {
		self.identifier
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
		let mut segment = Segment::empty(segment_id);

		let mmr_size = pmmr.unpruned_size();
		if segment.segment_unpruned_size(mmr_size) == 0 {
			return Err(SegmentError::NonExistent);
		}

		// Fill leaf data and hashes
		let (segment_first_pos, segment_last_pos) = segment.segment_pos_range(mmr_size);
		for pos0 in segment_first_pos..=segment_last_pos {
			if pmmr::is_leaf(pos0) {
				if let Some(data) = pmmr.get_data_from_file(pos0) {
					segment.leaf_data.push(data);
					segment.leaf_pos.push(pos0);
					continue;
				} else if !prunable {
					return Err(SegmentError::MissingLeaf(pos0));
				}
			}
			// TODO: optimize, no need to send every intermediary hash
			if prunable {
				if let Some(hash) = pmmr.get_from_file(pos0) {
					segment.hashes.push(hash);
					segment.hash_pos.push(pos0);
				}
			}
		}

		let mut start_pos = None;
		// Fully pruned segment: only include a single hash, the first unpruned parent
		if segment.leaf_data.is_empty() && segment.hashes.is_empty() {
			let family_branch = pmmr::family_branch(segment_last_pos, mmr_size);
			for (pos0, _) in family_branch {
				if let Some(hash) = pmmr.get_from_file(pos0) {
					segment.hashes.push(hash);
					segment.hash_pos.push(pos0);
					start_pos = Some(1 + pos0);
					break;
				}
			}
		}

		// Segment merkle proof
		segment.proof = SegmentProof::generate(
			pmmr,
			mmr_size,
			1 + segment_first_pos,
			1 + segment_last_pos,
			start_pos,
		)?;

		Ok(segment)
	}
}

impl<T> Segment<T>
where
	T: PMMRIndexHashable,
{
	/// Calculate root hash of this segment
	/// Returns `None` iff the segment is full and completely pruned
	pub fn root(
		&self,
		mmr_size: u64,
		bitmap: Option<&Bitmap>,
	) -> Result<Option<Hash>, SegmentError> {
		let (segment_first_pos, segment_last_pos) = self.segment_pos_range(mmr_size);
		let mut hashes = Vec::<Option<Hash>>::with_capacity(2 * (self.identifier.height as usize));
		let mut leaves0 = self.leaf_pos.iter().zip(&self.leaf_data);
		for pos0 in segment_first_pos..=segment_last_pos {
			let height = pmmr::bintree_postorder_height(pos0);
			let hash = if height == 0 {
				// Leaf
				if bitmap
					.map(|b| {
						let idx_1 = pmmr::n_leaves(pos0 + 1) - 1;
						let idx_2 = if pmmr::is_left_sibling(pos0) {
							idx_1 + 1
						} else {
							idx_1 - 1
						};
						b.contains(idx_1 as u32) || b.contains(idx_2 as u32) || pos0 == mmr_size - 1
					})
					.unwrap_or(true)
				{
					// We require the data of this leaf if either the mmr is not prunable or if
					//  the bitmap indicates it (or its sibling) should be here.
					// Edge case: if the final segment has an uneven number of leaves, we
					//  require the last leaf to be present regardless of the status in the bitmap.
					// TODO: possibly remove requirement on the sibling when we no longer support
					//  syncing through the txhashset.zip method.
					let data = leaves0
						.find(|&(&p, _)| p == pos0)
						.map(|(_, l)| l)
						.ok_or_else(|| SegmentError::MissingLeaf(pos0))?;
					Some(data.hash_with_index(pos0))
				} else {
					None
				}
			} else {
				let left_child_pos = 1 + pos0 - (1 << height);
				let right_child_pos = pos0;

				let right_child = hashes.pop().unwrap();
				let left_child = hashes.pop().unwrap();

				if bitmap.is_some() {
					// Prunable MMR
					match (left_child, right_child) {
						(None, None) => None,
						(Some(l), Some(r)) => Some((l, r).hash_with_index(pos0)),
						(None, Some(r)) => {
							let l = self.get_hash(left_child_pos - 1)?;
							Some((l, r).hash_with_index(pos0))
						}
						(Some(l), None) => {
							let r = self.get_hash(right_child_pos - 1)?;
							Some((l, r).hash_with_index(pos0))
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
							.hash_with_index(pos0),
					)
				}
			};
			hashes.push(hash);
		}

		if self.full_segment(mmr_size) {
			// Full segment: last position of segment is subtree root
			Ok(hashes.pop().unwrap())
		} else {
			// Not full (only final segment): peaks in segment, bag them together
			let peaks = pmmr::peaks(mmr_size)
				.into_iter()
				.filter(|&pos0| pos0 >= segment_first_pos && pos0 <= segment_last_pos)
				.rev();
			let mut hash = None;
			for pos0 in peaks {
				let mut lhash = hashes
					.pop()
					.ok_or_else(|| SegmentError::MissingHash(1 + pos0))?;
				if lhash.is_none() && bitmap.is_some() {
					// If this entire peak is pruned, load it from the segment hashes
					lhash = Some(self.get_hash(pos0)?);
				}
				let lhash = lhash.ok_or_else(|| SegmentError::MissingHash(1 + pos0))?;

				hash = match hash {
					None => Some(lhash),
					Some(rhash) => Some((lhash, rhash).hash_with_index(mmr_size)),
				};
			}
			Ok(Some(hash.unwrap()))
		}
	}

	/// Get the first 1-based (sucks) unpruned parent hash of this segment
	pub fn first_unpruned_parent(
		&self,
		mmr_size: u64,
		bitmap: Option<&Bitmap>,
	) -> Result<(Hash, u64), SegmentError> {
		let root = self.root(mmr_size, bitmap)?;
		let (_, last) = self.segment_pos_range(mmr_size);
		if let Some(root) = root {
			return Ok((root, 1 + last));
		}
		let bitmap = bitmap.unwrap();
		let n_leaves = pmmr::n_leaves(mmr_size);

		let mut cardinality = 0;
		let mut pos0 = last;
		let mut hash = Err(SegmentError::MissingHash(last));
		let mut family_branch = pmmr::family_branch(last, mmr_size).into_iter();
		while cardinality == 0 {
			hash = self.get_hash(pos0).map(|h| (h, 1 + pos0));
			if hash.is_ok() {
				// Return early in case a lower level hash is already present
				// This can occur if both child trees are pruned but compaction hasn't run yet
				return hash;
			}

			if let Some((p0, _)) = family_branch.next() {
				pos0 = p0;
				let range = (pmmr::n_leaves(1 + pmmr::bintree_leftmost(p0)) - 1)
					..min(pmmr::n_leaves(1 + pmmr::bintree_rightmost(p0)), n_leaves);
				cardinality = bitmap.range_cardinality(range);
			} else {
				break;
			}
		}
		hash
	}

	/// Check validity of the segment by calculating its root and validating the merkle proof
	pub fn validate(
		&self,
		mmr_size: u64,
		bitmap: Option<&Bitmap>,
		mmr_root: Hash,
	) -> Result<(), SegmentError> {
		let (first, last) = self.segment_pos_range(mmr_size);
		let (segment_root, segment_unpruned_pos) = self.first_unpruned_parent(mmr_size, bitmap)?;
		self.proof.validate(
			mmr_size,
			mmr_root,
			first,
			last,
			segment_root,
			segment_unpruned_pos,
		)
	}

	/// Check validity of the segment by calculating its root and validating the merkle proof
	/// This function assumes a final hashing step together with `other_root`
	pub fn validate_with(
		&self,
		mmr_size: u64,
		bitmap: Option<&Bitmap>,
		mmr_root: Hash,
		hash_last_pos: u64,
		other_root: Hash,
		other_is_left: bool,
	) -> Result<(), SegmentError> {
		let (first, last) = self.segment_pos_range(mmr_size);
		let (segment_root, segment_unpruned_pos) = self.first_unpruned_parent(mmr_size, bitmap)?;
		self.proof.validate_with(
			mmr_size,
			mmr_root,
			first,
			last,
			segment_root,
			segment_unpruned_pos,
			hash_last_pos,
			other_root,
			other_is_left,
		)
	}
}

impl<T: Readable> Readable for Segment<T> {
	fn read<R: Reader>(reader: &mut R) -> Result<Self, Error> {
		let identifier = Readable::read(reader)?;

		let n_hashes = reader.read_u64()? as usize;
		let mut hash_pos = Vec::with_capacity(n_hashes);
		let mut last_pos = 0;
		for _ in 0..n_hashes {
			let pos = reader.read_u64()?;
			if pos <= last_pos {
				return Err(Error::SortError);
			}
			last_pos = pos;
			hash_pos.push(pos - 1);
		}

		let mut hashes = Vec::<Hash>::with_capacity(n_hashes);
		for _ in 0..n_hashes {
			hashes.push(Readable::read(reader)?);
		}

		let n_leaves = reader.read_u64()? as usize;
		let mut leaf_pos = Vec::with_capacity(n_leaves);
		last_pos = 0;
		for _ in 0..n_leaves {
			let pos = reader.read_u64()?;
			if pos <= last_pos {
				return Err(Error::SortError);
			}
			last_pos = pos;
			leaf_pos.push(pos - 1);
		}

		let mut leaf_data = Vec::<T>::with_capacity(n_leaves);
		for _ in 0..n_leaves {
			leaf_data.push(Readable::read(reader)?);
		}

		let proof = Readable::read(reader)?;

		Ok(Self {
			identifier,
			hash_pos,
			hashes,
			leaf_pos,
			leaf_data,
			proof,
		})
	}
}

impl<T: Writeable> Writeable for Segment<T> {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), Error> {
		Writeable::write(&self.identifier, writer)?;
		writer.write_u64(self.hashes.len() as u64)?;
		for &pos in &self.hash_pos {
			writer.write_u64(1 + pos)?;
		}
		for hash in &self.hashes {
			Writeable::write(hash, writer)?;
		}
		writer.write_u64(self.leaf_data.len() as u64)?;
		for &pos in &self.leaf_pos {
			writer.write_u64(1 + pos)?;
		}
		for data in &self.leaf_data {
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
		start_pos: Option<u64>,
	) -> Result<Self, SegmentError>
	where
		U: PMMRable,
		B: Backend<U>,
	{
		let family_branch = pmmr::family_branch(segment_last_pos - 1, last_pos);

		// 1. siblings along the path from the subtree root to the peak
		let hashes: Result<Vec<_>, _> = family_branch
			.iter()
			.filter(|&&(p0, _)| start_pos.map(|s| p0 >= s).unwrap_or(true))
			.map(|&(_, s0)| {
				pmmr.get_hash(s0)
					.ok_or_else(|| SegmentError::MissingHash(s0))
			})
			.collect();
		let mut proof = Self { hashes: hashes? };

		// 2. bagged peaks to the right
		let peak_pos = family_branch
			.last()
			.map(|&(p0, _)| p0)
			.unwrap_or(segment_last_pos - 1);
		if let Some(h) = pmmr.bag_the_rhs(peak_pos) {
			proof.hashes.push(h);
		}

		// 3. peaks to the left
		let peaks: Result<Vec<_>, _> = pmmr::peaks(last_pos)
			.into_iter()
			.filter(|&x| 1 + x < segment_first_pos)
			.rev()
			.map(|p| pmmr.get_hash(p).ok_or_else(|| SegmentError::MissingHash(p)))
			.collect();
		proof.hashes.extend(peaks?);

		Ok(proof)
	}

	/// Size of the proof in hashes.
	pub fn size(&self) -> usize {
		self.hashes.len()
	}

	/// Reconstruct PMMR root using this proof
	pub fn reconstruct_root(
		&self,
		last_pos: u64,
		segment_first_pos0: u64,
		segment_last_pos0: u64,
		segment_root: Hash,
		segment_unpruned_pos: u64,
	) -> Result<Hash, SegmentError> {
		let mut iter = self.hashes.iter();
		let family_branch = pmmr::family_branch(segment_last_pos0, last_pos);

		// 1. siblings along the path from the subtree root to the peak
		let mut root = segment_root;
		for &(p0, s0) in family_branch
			.iter()
			.filter(|&&(p0, _)| p0 >= segment_unpruned_pos)
		{
			let sibling_hash = iter
				.next()
				.ok_or_else(|| SegmentError::MissingHash(1 + s0))?;
			root = if pmmr::is_left_sibling(s0) {
				(sibling_hash, root).hash_with_index(p0)
			} else {
				(root, sibling_hash).hash_with_index(p0)
			};
		}

		// 2. bagged peaks to the right
		let peak_pos0 = family_branch
			.last()
			.map(|&(p0, _)| p0)
			.unwrap_or(segment_last_pos0);

		let rhs = pmmr::peaks(last_pos)
			.into_iter()
			.filter(|&x| x > peak_pos0)
			.next();

		if let Some(pos0) = rhs {
			root = (
				root,
				iter.next()
					.ok_or_else(|| SegmentError::MissingHash(1 + pos0))?,
			)
				.hash_with_index(last_pos)
		}

		// 3. peaks to the left
		let peaks = pmmr::peaks(last_pos)
			.into_iter()
			.filter(|&x| x < segment_first_pos0)
			.rev();
		for pos0 in peaks {
			root = (
				iter.next()
					.ok_or_else(|| SegmentError::MissingHash(1 + pos0))?,
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
		segment_unpruned_pos: u64,
	) -> Result<(), SegmentError> {
		let root = self.reconstruct_root(
			last_pos,
			segment_first_pos,
			segment_last_pos,
			segment_root,
			segment_unpruned_pos,
		)?;
		if root == mmr_root {
			Ok(())
		} else {
			Err(SegmentError::Mismatch)
		}
	}

	/// Check validity of the proof by equating the reconstructed root with the actual root
	/// This function assumes a final hashing step together with `other_root`
	pub fn validate_with(
		&self,
		last_pos: u64,
		mmr_root: Hash,
		segment_first_pos: u64,
		segment_last_pos: u64,
		segment_root: Hash,
		segment_unpruned_pos: u64,
		hash_last_pos: u64,
		other_root: Hash,
		other_is_left: bool,
	) -> Result<(), SegmentError> {
		let root = self.reconstruct_root(
			last_pos,
			segment_first_pos,
			segment_last_pos,
			segment_root,
			segment_unpruned_pos,
		)?;
		let root = if other_is_left {
			(other_root, root).hash_with_index(hash_last_pos)
		} else {
			(root, other_root).hash_with_index(hash_last_pos)
		};
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
