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

use crate::core::consensus::HeaderInfo;
use crate::core::core::hash::{Hash, Hashed};
use crate::core::core::{Block, BlockHeader, BlockSums};
use crate::core::pow::Difficulty;
use crate::types::Tip;
use crate::util::secp::pedersen::Commitment;
use croaring::Bitmap;
use grin_store as store;
use grin_store::{option_to_not_found, to_key, Error, SerIterator};
use std::sync::Arc;

const STORE_SUBPATH: &'static str = "chain";

const BLOCK_HEADER_PREFIX: u8 = 'h' as u8;
const BLOCK_PREFIX: u8 = 'b' as u8;
const HEAD_PREFIX: u8 = 'H' as u8;
const TAIL_PREFIX: u8 = 'T' as u8;
const HEADER_HEAD_PREFIX: u8 = 'I' as u8;
const SYNC_HEAD_PREFIX: u8 = 's' as u8;
const COMMIT_POS_PREFIX: u8 = 'c' as u8;
const BLOCK_INPUT_BITMAP_PREFIX: u8 = 'B' as u8;
const BLOCK_SUMS_PREFIX: u8 = 'M' as u8;

/// All chain-related database operations
pub struct ChainStore {
	db: store::Store,
}

impl ChainStore {
	/// Create new chain store
	pub fn new(db_root: &str) -> Result<ChainStore, Error> {
		let db = store::Store::new(db_root, None, Some(STORE_SUBPATH.clone()), None)?;
		Ok(ChainStore { db })
	}
}

impl ChainStore {
	/// The current chain head.
	pub fn head(&self) -> Result<Tip, Error> {
		option_to_not_found(self.db.get_ser(&vec![HEAD_PREFIX]), "HEAD")
	}

	/// The current chain "tail" (earliest block in the store).
	pub fn tail(&self) -> Result<Tip, Error> {
		option_to_not_found(self.db.get_ser(&vec![TAIL_PREFIX]), "TAIL")
	}

	/// Header of the block at the head of the block chain (not the same thing as header_head).
	pub fn head_header(&self) -> Result<BlockHeader, Error> {
		self.get_block_header(&self.head()?.last_block_h)
	}

	/// Head of the header chain (not the same thing as head_header).
	pub fn header_head(&self) -> Result<Tip, Error> {
		option_to_not_found(self.db.get_ser(&vec![HEADER_HEAD_PREFIX]), "HEADER_HEAD")
	}

	/// The "sync" head.
	pub fn get_sync_head(&self) -> Result<Tip, Error> {
		option_to_not_found(self.db.get_ser(&vec![SYNC_HEAD_PREFIX]), "SYNC_HEAD")
	}

	/// Get full block.
	pub fn get_block(&self, h: &Hash) -> Result<Block, Error> {
		option_to_not_found(
			self.db.get_ser(&to_key(BLOCK_PREFIX, &mut h.to_vec())),
			&format!("BLOCK: {}", h),
		)
	}

	/// Does this full block exist?
	pub fn block_exists(&self, h: &Hash) -> Result<bool, Error> {
		self.db.exists(&to_key(BLOCK_PREFIX, &mut h.to_vec()))
	}

	/// Get block_sums for the block hash.
	pub fn get_block_sums(&self, h: &Hash) -> Result<BlockSums, Error> {
		option_to_not_found(
			self.db.get_ser(&to_key(BLOCK_SUMS_PREFIX, &mut h.to_vec())),
			&format!("Block sums for block: {}", h),
		)
	}

	/// Get previous header.
	pub fn get_previous_header(&self, header: &BlockHeader) -> Result<BlockHeader, Error> {
		self.get_block_header(&header.prev_hash)
	}

	/// Get block header.
	pub fn get_block_header(&self, h: &Hash) -> Result<BlockHeader, Error> {
		option_to_not_found(
			self.db
				.get_ser(&to_key(BLOCK_HEADER_PREFIX, &mut h.to_vec())),
			&format!("BLOCK HEADER: {}", h),
		)
	}

	/// Get PMMR pos for the given output commitment.
	pub fn get_output_pos(&self, commit: &Commitment) -> Result<u64, Error> {
		option_to_not_found(
			self.db
				.get_ser(&to_key(COMMIT_POS_PREFIX, &mut commit.as_ref().to_vec())),
			&format!("Output position for: {:?}", commit),
		)
	}

	/// Builds a new batch to be used with this store.
	pub fn batch(&self) -> Result<Batch<'_>, Error> {
		Ok(Batch {
			db: self.db.batch()?,
		})
	}
}

/// An atomic batch in which all changes can be committed all at once or
/// discarded on error.
pub struct Batch<'a> {
	db: store::Batch<'a>,
}

impl<'a> Batch<'a> {
	/// The head.
	pub fn head(&self) -> Result<Tip, Error> {
		option_to_not_found(self.db.get_ser(&vec![HEAD_PREFIX]), "HEAD")
	}

	/// The tail.
	pub fn tail(&self) -> Result<Tip, Error> {
		option_to_not_found(self.db.get_ser(&vec![TAIL_PREFIX]), "TAIL")
	}

	/// Header of the block at the head of the block chain (not the same thing as header_head).
	pub fn head_header(&self) -> Result<BlockHeader, Error> {
		self.get_block_header(&self.head()?.last_block_h)
	}

	/// Head of the header chain (not the same thing as head_header).
	pub fn header_head(&self) -> Result<Tip, Error> {
		option_to_not_found(self.db.get_ser(&vec![HEADER_HEAD_PREFIX]), "HEADER_HEAD")
	}

	/// Get "sync" head.
	pub fn get_sync_head(&self) -> Result<Tip, Error> {
		option_to_not_found(self.db.get_ser(&vec![SYNC_HEAD_PREFIX]), "SYNC_HEAD")
	}

	/// Save body head to db.
	pub fn save_body_head(&self, t: &Tip) -> Result<(), Error> {
		self.db.put_ser(&vec![HEAD_PREFIX], t)
	}

	/// Save body "tail" to db.
	pub fn save_body_tail(&self, t: &Tip) -> Result<(), Error> {
		self.db.put_ser(&vec![TAIL_PREFIX], t)
	}

	/// Save header_head to db.
	pub fn save_header_head(&self, t: &Tip) -> Result<(), Error> {
		self.db.put_ser(&vec![HEADER_HEAD_PREFIX], t)
	}

	/// Save "sync" head to db.
	pub fn save_sync_head(&self, t: &Tip) -> Result<(), Error> {
		self.db.put_ser(&vec![SYNC_HEAD_PREFIX], t)
	}

	/// Reset sync_head to the current head of the header chain.
	pub fn reset_sync_head(&self) -> Result<(), Error> {
		let head = self.header_head()?;
		self.save_sync_head(&head)
	}

	/// Reset header_head to the current head of the body chain.
	pub fn reset_header_head(&self) -> Result<(), Error> {
		let tip = self.head()?;
		self.save_header_head(&tip)
	}

	/// get block
	pub fn get_block(&self, h: &Hash) -> Result<Block, Error> {
		option_to_not_found(
			self.db.get_ser(&to_key(BLOCK_PREFIX, &mut h.to_vec())),
			&format!("Block with hash: {}", h),
		)
	}

	/// Does the block exist?
	pub fn block_exists(&self, h: &Hash) -> Result<bool, Error> {
		self.db.exists(&to_key(BLOCK_PREFIX, &mut h.to_vec()))
	}

	/// Save the block and the associated input bitmap.
	/// Note: the block header is not saved to the db here, assumes this has already been done.
	pub fn save_block(&self, b: &Block) -> Result<(), Error> {
		// Build the "input bitmap" for this new block and store it in the db.
		self.build_and_store_block_input_bitmap(&b)?;

		// Save the block itself to the db.
		self.db
			.put_ser(&to_key(BLOCK_PREFIX, &mut b.hash().to_vec())[..], b)?;

		Ok(())
	}

	/// Delete a full block. Does not delete any record associated with a block
	/// header.
	pub fn delete_block(&self, bh: &Hash) -> Result<(), Error> {
		self.db
			.delete(&to_key(BLOCK_PREFIX, &mut bh.to_vec())[..])?;

		// Best effort at deleting associated data for this block.
		// Not an error if these fail.
		{
			let _ = self.delete_block_sums(bh);
			let _ = self.delete_block_input_bitmap(bh);
		}

		Ok(())
	}

	/// Save block header to db.
	pub fn save_block_header(&self, header: &BlockHeader) -> Result<(), Error> {
		let hash = header.hash();

		// Store the header itself indexed by hash.
		self.db
			.put_ser(&to_key(BLOCK_HEADER_PREFIX, &mut hash.to_vec())[..], header)?;

		Ok(())
	}

	/// Save output_pos to index.
	pub fn save_output_pos(&self, commit: &Commitment, pos: u64) -> Result<(), Error> {
		self.db.put_ser(
			&to_key(COMMIT_POS_PREFIX, &mut commit.as_ref().to_vec())[..],
			&pos,
		)
	}

	/// Get output_pos from index.
	pub fn get_output_pos(&self, commit: &Commitment) -> Result<u64, Error> {
		option_to_not_found(
			self.db
				.get_ser(&to_key(COMMIT_POS_PREFIX, &mut commit.as_ref().to_vec())),
			&format!("Output position for commit: {:?}", commit),
		)
	}

	/// Clear all entries from the output_pos index (must be rebuilt after).
	pub fn clear_output_pos(&self) -> Result<(), Error> {
		let key = to_key(COMMIT_POS_PREFIX, &mut "".to_string().into_bytes());
		for (k, _) in self.db.iter::<u64>(&key)? {
			self.db.delete(&k)?;
		}
		Ok(())
	}

	/// Get the previous header.
	pub fn get_previous_header(&self, header: &BlockHeader) -> Result<BlockHeader, Error> {
		self.get_block_header(&header.prev_hash)
	}

	/// Get block header.
	pub fn get_block_header(&self, h: &Hash) -> Result<BlockHeader, Error> {
		option_to_not_found(
			self.db
				.get_ser(&to_key(BLOCK_HEADER_PREFIX, &mut h.to_vec())),
			&format!("BLOCK HEADER: {}", h),
		)
	}

	/// Save the input bitmap for the block.
	fn save_block_input_bitmap(&self, bh: &Hash, bm: &Bitmap) -> Result<(), Error> {
		self.db.put(
			&to_key(BLOCK_INPUT_BITMAP_PREFIX, &mut bh.to_vec())[..],
			&bm.serialize(),
		)
	}

	/// Delete the block input bitmap.
	fn delete_block_input_bitmap(&self, bh: &Hash) -> Result<(), Error> {
		self.db
			.delete(&to_key(BLOCK_INPUT_BITMAP_PREFIX, &mut bh.to_vec()))
	}

	/// Save block_sums for the block.
	pub fn save_block_sums(&self, h: &Hash, sums: &BlockSums) -> Result<(), Error> {
		self.db
			.put_ser(&to_key(BLOCK_SUMS_PREFIX, &mut h.to_vec())[..], &sums)
	}

	/// Get block_sums for the block.
	pub fn get_block_sums(&self, h: &Hash) -> Result<BlockSums, Error> {
		option_to_not_found(
			self.db.get_ser(&to_key(BLOCK_SUMS_PREFIX, &mut h.to_vec())),
			&format!("Block sums for block: {}", h),
		)
	}

	/// Delete the block_sums for the block.
	fn delete_block_sums(&self, bh: &Hash) -> Result<(), Error> {
		self.db.delete(&to_key(BLOCK_SUMS_PREFIX, &mut bh.to_vec()))
	}

	/// Build the input bitmap for the given block.
	fn build_block_input_bitmap(&self, block: &Block) -> Result<Bitmap, Error> {
		let bitmap = block
			.inputs()
			.iter()
			.filter_map(|x| self.get_output_pos(&x.commitment()).ok())
			.map(|x| x as u32)
			.collect();
		Ok(bitmap)
	}

	/// Build and store the input bitmap for the given block.
	fn build_and_store_block_input_bitmap(&self, block: &Block) -> Result<Bitmap, Error> {
		// Build the bitmap.
		let bitmap = self.build_block_input_bitmap(block)?;

		// Save the bitmap to the db (via the batch).
		self.save_block_input_bitmap(&block.hash(), &bitmap)?;

		Ok(bitmap)
	}

	/// Get the block input bitmap from the db or build the bitmap from
	/// the full block from the db (if the block is found).
	pub fn get_block_input_bitmap(&self, bh: &Hash) -> Result<Bitmap, Error> {
		if let Ok(Some(bytes)) = self
			.db
			.get(&to_key(BLOCK_INPUT_BITMAP_PREFIX, &mut bh.to_vec()))
		{
			Ok(Bitmap::deserialize(&bytes))
		} else {
			match self.get_block(bh) {
				Ok(block) => {
					let bitmap = self.build_and_store_block_input_bitmap(&block)?;
					Ok(bitmap)
				}
				Err(e) => Err(e),
			}
		}
	}

	/// Commits this batch. If it's a child batch, it will be merged with the
	/// parent, otherwise the batch is written to db.
	pub fn commit(self) -> Result<(), Error> {
		self.db.commit()
	}

	/// Creates a child of this batch. It will be merged with its parent on
	/// commit, abandoned otherwise.
	pub fn child(&mut self) -> Result<Batch<'_>, Error> {
		Ok(Batch {
			db: self.db.child()?,
		})
	}

	/// An iterator to all block in db
	pub fn blocks_iter(&self) -> Result<SerIterator<Block>, Error> {
		let key = to_key(BLOCK_PREFIX, &mut "".to_string().into_bytes());
		self.db.iter(&key)
	}
}

/// An iterator on blocks, from latest to earliest, specialized to return
/// information pertaining to block difficulty calculation (timestamp and
/// previous difficulties). Mostly used by the consensus next difficulty
/// calculation.
pub struct DifficultyIter<'a> {
	start: Hash,
	store: Option<Arc<ChainStore>>,
	batch: Option<Batch<'a>>,

	// maintain state for both the "next" header in this iteration
	// and its previous header in the chain ("next next" in the iteration)
	// so we effectively read-ahead as we iterate through the chain back
	// toward the genesis block (while maintaining current state)
	header: Option<BlockHeader>,
	prev_header: Option<BlockHeader>,
}

impl<'a> DifficultyIter<'a> {
	/// Build a new iterator using the provided chain store and starting from
	/// the provided block hash.
	pub fn from<'b>(start: Hash, store: Arc<ChainStore>) -> DifficultyIter<'b> {
		DifficultyIter {
			start,
			store: Some(store),
			batch: None,
			header: None,
			prev_header: None,
		}
	}

	/// Build a new iterator using the provided chain store batch and starting from
	/// the provided block hash.
	pub fn from_batch(start: Hash, batch: Batch<'_>) -> DifficultyIter<'_> {
		DifficultyIter {
			start,
			store: None,
			batch: Some(batch),
			header: None,
			prev_header: None,
		}
	}
}

impl<'a> Iterator for DifficultyIter<'a> {
	type Item = HeaderInfo;

	fn next(&mut self) -> Option<Self::Item> {
		// Get both header and previous_header if this is the initial iteration.
		// Otherwise move prev_header to header and get the next prev_header.
		self.header = if self.header.is_none() {
			if let Some(ref batch) = self.batch {
				batch.get_block_header(&self.start).ok()
			} else {
				if let Some(ref store) = self.store {
					store.get_block_header(&self.start).ok()
				} else {
					None
				}
			}
		} else {
			self.prev_header.clone()
		};

		// If we have a header we can do this iteration.
		// Otherwise we are done.
		if let Some(header) = self.header.clone() {
			if let Some(ref batch) = self.batch {
				self.prev_header = batch.get_previous_header(&header).ok();
			} else {
				if let Some(ref store) = self.store {
					self.prev_header = store.get_previous_header(&header).ok();
				} else {
					self.prev_header = None;
				}
			}

			let prev_difficulty = self
				.prev_header
				.clone()
				.map_or(Difficulty::zero(), |x| x.total_difficulty());
			let difficulty = header.total_difficulty() - prev_difficulty;
			let scaling = header.pow.secondary_scaling;

			Some(HeaderInfo::new(
				header.timestamp.timestamp() as u64,
				difficulty,
				scaling,
				header.pow.is_secondary(),
			))
		} else {
			return None;
		}
	}
}
