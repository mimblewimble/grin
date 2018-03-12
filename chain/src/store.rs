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

//! Implements storage primitives required by the chain

use std::sync::Arc;

use util::secp::pedersen::Commitment;

use types::*;
use core::core::hash::{Hash, Hashed};
use core::core::{Block, BlockHeader};
use core::consensus::TargetError;
use core::core::target::Difficulty;
use grin_store::{self, option_to_not_found, to_key, Error, u64_to_key};

const STORE_SUBPATH: &'static str = "chain";

const BLOCK_HEADER_PREFIX: u8 = 'h' as u8;
const BLOCK_PREFIX: u8 = 'b' as u8;
const HEAD_PREFIX: u8 = 'H' as u8;
const HEADER_HEAD_PREFIX: u8 = 'I' as u8;
const SYNC_HEAD_PREFIX: u8 = 's' as u8;
const HEADER_HEIGHT_PREFIX: u8 = '8' as u8;
const COMMIT_POS_PREFIX: u8 = 'c' as u8;
const BLOCK_MARKER_PREFIX: u8 = 'm' as u8;
const BLOCK_PMMR_FILE_METADATA_PREFIX: u8 = 'p' as u8;

/// An implementation of the ChainStore trait backed by a simple key-value
/// store.
pub struct ChainKVStore {
	db: grin_store::Store,
}

impl ChainKVStore {
	/// Create new chain store
	pub fn new(root_path: String) -> Result<ChainKVStore, Error> {
		let db = grin_store::Store::open(format!("{}/{}", root_path, STORE_SUBPATH).as_str())?;
		Ok(ChainKVStore { db: db })
	}
}

impl ChainStore for ChainKVStore {
	fn head(&self) -> Result<Tip, Error> {
		option_to_not_found(self.db.get_ser(&vec![HEAD_PREFIX]))
	}

	fn head_header(&self) -> Result<BlockHeader, Error> {
		self.get_block_header(&try!(self.head()).last_block_h)
	}

	fn save_head(&self, t: &Tip) -> Result<(), Error> {
		self.db
			.batch()
			.put_ser(&vec![HEAD_PREFIX], t)?
			.put_ser(&vec![HEADER_HEAD_PREFIX], t)?
			.write()
	}

	fn save_body_head(&self, t: &Tip) -> Result<(), Error> {
		self.db.put_ser(&vec![HEAD_PREFIX], t)
	}

	fn get_header_head(&self) -> Result<Tip, Error> {
		option_to_not_found(self.db.get_ser(&vec![HEADER_HEAD_PREFIX]))
	}

	fn save_header_head(&self, t: &Tip) -> Result<(), Error> {
		self.db.put_ser(&vec![HEADER_HEAD_PREFIX], t)
	}

	fn get_sync_head(&self) -> Result<Tip, Error> {
		option_to_not_found(self.db.get_ser(&vec![SYNC_HEAD_PREFIX]))
	}

	fn save_sync_head(&self, t: &Tip) -> Result<(), Error> {
		self.db.put_ser(&vec![SYNC_HEAD_PREFIX], t)
	}

	// Reset both header_head and sync_head to the current head of the body chain
	fn reset_head(&self) -> Result<(), Error> {
		let tip = self.head()?;
		self.save_header_head(&tip)?;
		self.save_sync_head(&tip)
	}

	fn get_block(&self, h: &Hash) -> Result<Block, Error> {
		option_to_not_found(self.db.get_ser(&to_key(BLOCK_PREFIX, &mut h.to_vec())))
	}

	fn block_exists(&self, h: &Hash) -> Result<bool, Error> {
		self.db.exists(&to_key(BLOCK_PREFIX, &mut h.to_vec()))
	}

	fn get_block_header(&self, h: &Hash) -> Result<BlockHeader, Error> {
		option_to_not_found(
			self.db
				.get_ser(&to_key(BLOCK_HEADER_PREFIX, &mut h.to_vec())),
		)
	}

	/// Save the block and its header
	fn save_block(&self, b: &Block) -> Result<(), Error> {
		let batch = self.db
			.batch()
			.put_ser(&to_key(BLOCK_PREFIX, &mut b.hash().to_vec())[..], b)?
			.put_ser(
				&to_key(BLOCK_HEADER_PREFIX, &mut b.hash().to_vec())[..],
				&b.header,
			)?;
		batch.write()
	}

	/// Delete a full block. Does not delete any record associated with a block
	/// header.
	fn delete_block(&self, bh: &Hash) -> Result<(), Error> {
		self.db.delete(&to_key(BLOCK_PREFIX, &mut bh.to_vec())[..])
	}

	fn is_on_current_chain(&self, header: &BlockHeader) -> Result<(), Error> {
		let header_at_height = self.get_header_by_height(header.height)?;
		if header.hash() == header_at_height.hash() {
			Ok(())
		} else {
			Err(Error::NotFoundErr)
		}
	}

	fn save_block_header(&self, bh: &BlockHeader) -> Result<(), Error> {
		self.db.put_ser(
			&to_key(BLOCK_HEADER_PREFIX, &mut bh.hash().to_vec())[..],
			bh,
		)
	}

	fn get_header_by_height(&self, height: u64) -> Result<BlockHeader, Error> {
		option_to_not_found(self.db.get_ser(&u64_to_key(HEADER_HEIGHT_PREFIX, height)))
			.and_then(|hash| self.get_block_header(&hash))
	}

	fn save_header_height(&self, bh: &BlockHeader) -> Result<(), Error> {
		self.db
			.put_ser(&u64_to_key(HEADER_HEIGHT_PREFIX, bh.height), &bh.hash())
	}

	fn delete_header_by_height(&self, height: u64) -> Result<(), Error> {
		self.db.delete(&u64_to_key(HEADER_HEIGHT_PREFIX, height))
	}

	fn save_output_pos(&self, commit: &Commitment, pos: u64) -> Result<(), Error> {
		self.db.put_ser(
			&to_key(COMMIT_POS_PREFIX, &mut commit.as_ref().to_vec())[..],
			&pos,
		)
	}

	fn get_output_pos(&self, commit: &Commitment) -> Result<u64, Error> {
		option_to_not_found(
			self.db
				.get_ser(&to_key(COMMIT_POS_PREFIX, &mut commit.as_ref().to_vec())),
		)
	}

	fn delete_output_pos(&self, commit: &[u8]) -> Result<(), Error> {
		self.db
			.delete(&to_key(COMMIT_POS_PREFIX, &mut commit.to_vec()))
	}

	fn save_block_marker(&self, bh: &Hash, marker: &(u64, u64)) -> Result<(), Error> {
		self.db
			.put_ser(&to_key(BLOCK_MARKER_PREFIX, &mut bh.to_vec())[..], &marker)
	}

	fn get_block_marker(&self, bh: &Hash) -> Result<(u64, u64), Error> {
		option_to_not_found(
			self.db
				.get_ser(&to_key(BLOCK_MARKER_PREFIX, &mut bh.to_vec())),
		)
	}

	fn delete_block_marker(&self, bh: &Hash) -> Result<(), Error> {
		self.db
			.delete(&to_key(BLOCK_MARKER_PREFIX, &mut bh.to_vec()))
	}

	fn save_block_pmmr_file_metadata(
		&self,
		h: &Hash,
		md: &PMMRFileMetadataCollection,
	) -> Result<(), Error> {
		self.db.put_ser(
			&to_key(BLOCK_PMMR_FILE_METADATA_PREFIX, &mut h.to_vec())[..],
			&md,
		)
	}

	fn get_block_pmmr_file_metadata(&self, h: &Hash) -> Result<PMMRFileMetadataCollection, Error> {
		option_to_not_found(
			self.db
				.get_ser(&to_key(BLOCK_PMMR_FILE_METADATA_PREFIX, &mut h.to_vec())),
		)
	}

	fn delete_block_pmmr_file_metadata(&self, h: &Hash) -> Result<(), Error> {
		self.db
			.delete(&to_key(BLOCK_PMMR_FILE_METADATA_PREFIX, &mut h.to_vec())[..])
	}

	/// Maintain consistency of the "header_by_height" index by traversing back
	/// through the current chain and updating "header_by_height" until we reach
	/// a block_header that is consistent with its height (everything prior to
	/// this will be consistent).
	/// We need to handle the case where we have no index entry for a given
	/// height to account for the case where we just switched to a new fork and
	/// the height jumped beyond current chain height.
	fn setup_height(&self, header: &BlockHeader, old_tip: &Tip) -> Result<(), Error> {
		// remove headers ahead if we backtracked
		for n in header.height..old_tip.height {
			self.delete_header_by_height(n)?;
		}

		self.save_header_height(&header)?;

		if header.height > 0 {
			let mut prev_header = self.get_block_header(&header.previous)?;
			while prev_header.height > 0 {
				if let Ok(_) = self.is_on_current_chain(&prev_header) {
					break;
				}
				self.save_header_height(&prev_header)?;

				prev_header = self.get_block_header(&prev_header.previous)?;
			}
		}
		Ok(())
	}
}

/// An iterator on blocks, from latest to earliest, specialized to return
/// information pertaining to block difficulty calculation (timestamp and
/// previous difficulties). Mostly used by the consensus next difficulty
/// calculation.
pub struct DifficultyIter {
	next: Hash,
	store: Arc<ChainStore>,
}

impl DifficultyIter {
	/// Build a new iterator using the provided chain store and starting from
	/// the provided block hash.
	pub fn from(start: Hash, store: Arc<ChainStore>) -> DifficultyIter {
		DifficultyIter {
			next: start,
			store: store,
		}
	}
}

impl Iterator for DifficultyIter {
	type Item = Result<(u64, Difficulty), TargetError>;

	fn next(&mut self) -> Option<Self::Item> {
		let bhe = self.store.get_block_header(&self.next);
		match bhe {
			Err(_) => None,
			Ok(bh) => {
				self.next = bh.previous;
				Some(Ok((bh.timestamp.to_timespec().sec as u64, bh.difficulty)))
			}
		}
	}
}
