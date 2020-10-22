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

//! Merkle Proofs

use crate::core::pmmr;
use crate::ser;
use crate::ser::read_multi;
use crate::ser::{PMMRIndexHashable, Readable, Reader, Writeable, Writer};
use util::ToHex;

/// Merkle proof errors.
#[derive(Clone, Debug, PartialEq)]
pub enum MerkleProofError {
	/// Merkle proof root hash does not match when attempting to verify.
	RootMismatch,
}

/// A Merkle proof that proves a particular element exists in the MMR.
#[derive(Serialize, Deserialize, Debug, Eq, PartialEq, Clone, PartialOrd, Ord)]
pub struct MerkleProof<T: PMMRIndexHashable> {
	/// The size of the MMR at the time the proof was created.
	pub mmr_size: u64,
	/// The sibling path from the leaf up to the final sibling hashing to the
	/// root.
	pub path: Vec<T::H>,
}

impl<T: PMMRIndexHashable> Writeable for MerkleProof<T> {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		writer.write_u64(self.mmr_size)?;
		writer.write_u64(self.path.len() as u64)?;
		self.path.write(writer)?;
		Ok(())
	}
}

impl<T: PMMRIndexHashable> Readable for MerkleProof<T> {
	fn read<R: Reader>(reader: &mut R) -> Result<MerkleProof<T>, ser::Error> {
		let mmr_size = reader.read_u64()?;
		let path_len = reader.read_u64()?;
		let path = read_multi(reader, path_len)?;
		Ok(MerkleProof { mmr_size, path })
	}
}

impl<T: PMMRIndexHashable> Default for MerkleProof<T> {
	fn default() -> MerkleProof<T> {
		MerkleProof::empty()
	}
}

impl<T: PMMRIndexHashable> MerkleProof<T> {
	/// The "empty" Merkle proof.
	pub fn empty() -> MerkleProof<T> {
		MerkleProof {
			mmr_size: 0,
			path: Vec::default(),
		}
	}

	/// Serialize the Merkle proof as a hex string (for api json endpoints)
	pub fn to_hex(&self) -> String {
		let mut vec = Vec::new();
		ser::serialize_default(&mut vec, &self).expect("serialization failed");
		vec.to_hex()
	}

	/// Convert hex string representation back to a Merkle proof instance
	pub fn from_hex(hex: &str) -> Result<MerkleProof<T>, String> {
		let bytes = util::from_hex(hex).unwrap();
		let res = ser::deserialize_default(&mut &bytes[..])
			.map_err(|_| "failed to deserialize a Merkle Proof".to_string())?;
		Ok(res)
	}

	/// Verifies the Merkle proof against the provided
	/// root hash, element and position in the MMR.
	pub fn verify(&mut self, root: T::H, node: &T, node_pos: u64) -> Result<(), MerkleProofError> {
		// TODO - can we just max or min this?
		let index = if node_pos > self.mmr_size {
			self.mmr_size
		} else {
			node_pos - 1
		};
		let hash = node.hash_with_index(index);
		self.verify_consume(root, hash, node_pos)
	}

	fn verify_children(
		&mut self,
		root: T::H,
		lc: T::H,
		rc: T::H,
		node_pos: u64,
	) -> Result<(), MerkleProofError> {
		// TODO - can we just max or min this?
		let index = if node_pos > self.mmr_size {
			self.mmr_size
		} else {
			node_pos - 1
		};
		let hash = T::hash_children(index, lc, rc);
		self.verify_consume(root, hash, node_pos)
	}

	/// Internal verify fn.
	/// Takes a mut proof as it removes elements from the path as it progresses.
	fn verify_consume(
		&mut self,
		root: T::H,
		node: T::H,
		node_pos: u64,
	) -> Result<(), MerkleProofError> {
		// handle special case of only a single entry in the MMR
		// (no siblings to hash together)
		if self.path.is_empty() {
			if root == node {
				return Ok(());
			} else {
				return Err(MerkleProofError::RootMismatch);
			}
		}

		let sibling = self.path.remove(0);
		let (parent_pos, sibling_pos) = pmmr::family(node_pos);

		// parent here is a tuple of child hash entries (lc, rc)
		// these will be hashed with the parent_pos in the next iteration as we
		// progress up the path toward the root
		let peaks_pos = pmmr::peaks(self.mmr_size);
		let (lc, rc) = if let Ok(x) = peaks_pos.binary_search(&node_pos) {
			if x == peaks_pos.len() - 1 {
				(sibling, node)
			} else {
				(node, sibling)
			}
		} else if parent_pos > self.mmr_size {
			(sibling, node)
		} else {
			if pmmr::is_left_sibling(sibling_pos) {
				(sibling, node)
			} else {
				(node, sibling)
			}
		};

		self.verify_children(root, lc, rc, parent_pos)
	}
}
