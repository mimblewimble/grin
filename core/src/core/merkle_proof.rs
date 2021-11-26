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

//! Merkle Proofs

use crate::core::hash::Hash;
use crate::core::pmmr;
use crate::ser;
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
pub struct MerkleProof {
	/// The size of the MMR at the time the proof was created.
	pub mmr_size: u64,
	/// The sibling path from the leaf up to the final sibling hashing to the
	/// root.
	pub path: Vec<Hash>,
}

impl Writeable for MerkleProof {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		writer.write_u64(self.mmr_size)?;
		writer.write_u64(self.path.len() as u64)?;
		self.path.write(writer)?;
		Ok(())
	}
}

impl Readable for MerkleProof {
	fn read<R: Reader>(reader: &mut R) -> Result<MerkleProof, ser::Error> {
		let mmr_size = reader.read_u64()?;
		let path_len = reader.read_u64()?;
		let mut path = Vec::with_capacity(path_len as usize);
		for _ in 0..path_len {
			let hash = Hash::read(reader)?;
			path.push(hash);
		}

		Ok(MerkleProof { mmr_size, path })
	}
}

impl Default for MerkleProof {
	fn default() -> MerkleProof {
		MerkleProof::empty()
	}
}

impl MerkleProof {
	/// The "empty" Merkle proof.
	pub fn empty() -> MerkleProof {
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
	pub fn from_hex(hex: &str) -> Result<MerkleProof, String> {
		let bytes = util::from_hex(hex).unwrap();
		let res = ser::deserialize_default(&mut &bytes[..])
			.map_err(|_| "failed to deserialize a Merkle Proof".to_string())?;
		Ok(res)
	}

	/// Verifies the Merkle proof against the provided
	/// root hash, element and position in the MMR.
	pub fn verify(
		&self,
		root: Hash,
		element: &dyn PMMRIndexHashable,
		node_pos: u64,
	) -> Result<(), MerkleProofError> {
		let mut proof = self.clone();
		// calculate the peaks once as these are based on overall MMR size
		// (and will not change)
		let peaks_pos = pmmr::peaks(self.mmr_size);
		proof.verify_consume(root, element, node_pos, &peaks_pos)
	}

	/// Consumes the Merkle proof while verifying it.
	/// The proof can no longer be used by the caller after dong this.
	/// Caller must clone() the proof first.
	fn verify_consume(
		&mut self,
		root: Hash,
		element: &dyn PMMRIndexHashable,
		node_pos0: u64,
		peaks_pos0: &[u64],
	) -> Result<(), MerkleProofError> {
		let node_hash = if node_pos0 >= self.mmr_size {
			element.hash_with_index(self.mmr_size)
		} else {
			element.hash_with_index(node_pos0)
		};

		// handle special case of only a single entry in the MMR
		// (no siblings to hash together)
		if self.path.is_empty() {
			if root == node_hash {
				return Ok(());
			} else {
				return Err(MerkleProofError::RootMismatch);
			}
		}

		let sibling = self.path.remove(0);
		let (parent_pos0, sibling_pos0) = pmmr::family(node_pos0);

		if let Ok(x) = peaks_pos0.binary_search(&(node_pos0)) {
			let parent = if x == peaks_pos0.len() - 1 {
				(sibling, node_hash)
			} else {
				(node_hash, sibling)
			};
			self.verify(root, &parent, parent_pos0)
		} else if parent_pos0 >= self.mmr_size {
			let parent = (sibling, node_hash);
			self.verify(root, &parent, parent_pos0)
		} else {
			let parent = if pmmr::is_left_sibling(sibling_pos0) {
				(sibling, node_hash)
			} else {
				(node_hash, sibling)
			};
			self.verify(root, &parent, parent_pos0)
		}
	}
}
