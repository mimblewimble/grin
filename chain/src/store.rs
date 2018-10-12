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

use std::sync::{Arc, RwLock};

use croaring::Bitmap;
use lmdb;
use lru_cache::LruCache;

use util::secp::pedersen::Commitment;

use core::consensus::HeaderInfo;
use core::core::hash::{Hash, Hashed};
use core::core::{Block, BlockHeader, BlockSums};
use core::pow::Difficulty;
use grin_store as store;
use grin_store::{option_to_not_found, to_key, u64_to_key, Error};
use types::Tip;

const STORE_SUBPATH: &'static str = "chain";

const BLOCK_HEADER_PREFIX: u8 = 'h' as u8;
const BLOCK_PREFIX: u8 = 'b' as u8;
const HEAD_PREFIX: u8 = 'H' as u8;
const HEADER_HEAD_PREFIX: u8 = 'I' as u8;
const SYNC_HEAD_PREFIX: u8 = 's' as u8;
const HEADER_HEIGHT_PREFIX: u8 = '8' as u8;
const COMMIT_POS_PREFIX: u8 = 'c' as u8;
const BLOCK_INPUT_BITMAP_PREFIX: u8 = 'B' as u8;
const BLOCK_SUMS_PREFIX: u8 = 'M' as u8;

/// All chain-related database operations
pub struct ChainStore {
	db: store::Store,
	header_cache: Arc<RwLock<LruCache<Hash, BlockHeader>>>,
	block_input_bitmap_cache: Arc<RwLock<LruCache<Hash, Vec<u8>>>>,
	block_sums_cache: Arc<RwLock<LruCache<Hash, BlockSums>>>,
}

impl ChainStore {
	/// Create new chain store
	pub fn new(db_env: Arc<lmdb::Environment>) -> Result<ChainStore, Error> {
		let db = store::Store::open(db_env, STORE_SUBPATH);
		Ok(ChainStore {
			db,
			header_cache: Arc::new(RwLock::new(LruCache::new(1_000))),
			block_input_bitmap_cache: Arc::new(RwLock::new(LruCache::new(1_000))),
			block_sums_cache: Arc::new(RwLock::new(LruCache::new(1_000))),
		})
	}
}

#[allow(missing_docs)]
impl ChainStore {
	pub fn head(&self) -> Result<Tip, Error> {
		option_to_not_found(self.db.get_ser(&vec![HEAD_PREFIX]), "HEAD")
	}

	/// Header of the block at the head of the block chain (not the same thing as header_head).
	pub fn head_header(&self) -> Result<BlockHeader, Error> {
		self.get_block_header(&self.head()?.last_block_h)
	}

	/// Head of the header chain (not the same thing as head_header).
	pub fn header_head(&self) -> Result<Tip, Error> {
		option_to_not_found(self.db.get_ser(&vec![HEADER_HEAD_PREFIX]), "HEADER_HEAD")
	}

	pub fn get_sync_head(&self) -> Result<Tip, Error> {
		option_to_not_found(self.db.get_ser(&vec![SYNC_HEAD_PREFIX]), "SYNC_HEAD")
	}

	pub fn get_block(&self, h: &Hash) -> Result<Block, Error> {
		option_to_not_found(
			self.db.get_ser(&to_key(BLOCK_PREFIX, &mut h.to_vec())),
			&format!("BLOCK: {}", h),
		)
	}

	pub fn block_exists(&self, h: &Hash) -> Result<bool, Error> {
		self.db.exists(&to_key(BLOCK_PREFIX, &mut h.to_vec()))
	}

	pub fn get_block_sums(&self, h: &Hash) -> Result<BlockSums, Error> {
		{
			let mut block_sums_cache = self.block_sums_cache.write().unwrap();

			// cache hit - return the value from the cache
			if let Some(block_sums) = block_sums_cache.get_mut(h) {
				return Ok(block_sums.clone());
			}
		}

		let block_sums: Result<BlockSums, Error> = option_to_not_found(
			self.db.get_ser(&to_key(BLOCK_SUMS_PREFIX, &mut h.to_vec())),
			&format!("Block sums for block: {}", h),
		);

		// cache miss - so adding to the cache for next time
		if let Ok(block_sums) = block_sums {
			{
				let mut block_sums_cache = self.block_sums_cache.write().unwrap();
				block_sums_cache.insert(*h, block_sums.clone());
			}
			Ok(block_sums)
		} else {
			block_sums
		}
	}

	pub fn get_block_header(&self, h: &Hash) -> Result<BlockHeader, Error> {
		{
			let mut header_cache = self.header_cache.write().unwrap();

			// cache hit - return the value from the cache
			if let Some(header) = header_cache.get_mut(h) {
				return Ok(header.clone());
			}
		}

		let header: Result<BlockHeader, Error> = option_to_not_found(
			self.db
				.get_ser(&to_key(BLOCK_HEADER_PREFIX, &mut h.to_vec())),
			&format!("BLOCK HEADER: {}", h),
		);

		// cache miss - so adding to the cache for next time
		if let Ok(header) = header {
			{
				let mut header_cache = self.header_cache.write().unwrap();
				header_cache.insert(*h, header.clone());
			}
			Ok(header)
		} else {
			header
		}
	}

	pub fn get_hash_by_height(&self, height: u64) -> Result<Hash, Error> {
		option_to_not_found(
			self.db.get_ser(&u64_to_key(HEADER_HEIGHT_PREFIX, height)),
			&format!("Hash at height: {}", height),
		)
	}

	// We are on the current chain if -
	// * the header by height index matches the header, and
	// * we are not ahead of the current head
	pub fn is_on_current_chain(&self, header: &BlockHeader) -> Result<(), Error> {
		let head = self.head()?;

		// check we are not out ahead of the current head
		if header.height > head.height {
			return Err(Error::NotFoundErr(String::from(
				"header.height > head.height",
			)));
		}

		let header_at_height = self.get_header_by_height(header.height)?;
		if header.hash() == header_at_height.hash() {
			Ok(())
		} else {
			Err(Error::NotFoundErr(String::from(
				"header.hash == header_at_height.hash",
			)))
		}
	}

	pub fn get_header_by_height(&self, height: u64) -> Result<BlockHeader, Error> {
		option_to_not_found(
			self.db.get_ser(&u64_to_key(HEADER_HEIGHT_PREFIX, height)),
			&format!("Header at height: {}", height),
		).and_then(|hash| self.get_block_header(&hash))
	}

	pub fn get_output_pos(&self, commit: &Commitment) -> Result<u64, Error> {
		option_to_not_found(
			self.db
				.get_ser(&to_key(COMMIT_POS_PREFIX, &mut commit.as_ref().to_vec())),
			&format!("Output position for: {:?}", commit),
		)
	}

	/// Builds a new batch to be used with this store.
	pub fn batch(&self) -> Result<Batch, Error> {
		Ok(Batch {
			db: self.db.batch()?,
			header_cache: self.header_cache.clone(),
			block_input_bitmap_cache: self.block_input_bitmap_cache.clone(),
			block_sums_cache: self.block_sums_cache.clone(),
		})
	}
}

/// An atomic batch in which all changes can be committed all at once or
/// discarded on error.
pub struct Batch<'a> {
	db: store::Batch<'a>,
	header_cache: Arc<RwLock<LruCache<Hash, BlockHeader>>>,
	block_sums_cache: Arc<RwLock<LruCache<Hash, BlockSums>>>,
	block_input_bitmap_cache: Arc<RwLock<LruCache<Hash, Vec<u8>>>>,
}

#[allow(missing_docs)]
impl<'a> Batch<'a> {
	pub fn head(&self) -> Result<Tip, Error> {
		option_to_not_found(self.db.get_ser(&vec![HEAD_PREFIX]), "HEAD")
	}

	/// Header of the block at the head of the block chain (not the same thing as header_head).
	pub fn head_header(&self) -> Result<BlockHeader, Error> {
		self.get_block_header(&self.head()?.last_block_h)
	}

	/// Head of the header chain (not the same thing as head_header).
	pub fn header_head(&self) -> Result<Tip, Error> {
		option_to_not_found(self.db.get_ser(&vec![HEADER_HEAD_PREFIX]), "HEADER_HEAD")
	}

	pub fn get_sync_head(&self) -> Result<Tip, Error> {
		option_to_not_found(self.db.get_ser(&vec![SYNC_HEAD_PREFIX]), "SYNC_HEAD")
	}

	pub fn save_head(&self, t: &Tip) -> Result<(), Error> {
		self.db.put_ser(&vec![HEAD_PREFIX], t)?;
		self.db.put_ser(&vec![HEADER_HEAD_PREFIX], t)
	}

	pub fn save_body_head(&self, t: &Tip) -> Result<(), Error> {
		self.db.put_ser(&vec![HEAD_PREFIX], t)
	}

	pub fn save_header_head(&self, t: &Tip) -> Result<(), Error> {
		self.db.put_ser(&vec![HEADER_HEAD_PREFIX], t)
	}

	pub fn get_hash_by_height(&self, height: u64) -> Result<Hash, Error> {
		option_to_not_found(
			self.db.get_ser(&u64_to_key(HEADER_HEIGHT_PREFIX, height)),
			&format!("Hash at height: {}", height),
		)
	}

	pub fn save_sync_head(&self, t: &Tip) -> Result<(), Error> {
		self.db.put_ser(&vec![SYNC_HEAD_PREFIX], t)
	}

	pub fn reset_sync_head(&self) -> Result<(), Error> {
		let head = self.header_head()?;
		self.save_sync_head(&head)
	}

	// Reset both header_head and sync_head to the current head of the body chain
	pub fn reset_head(&self) -> Result<(), Error> {
		let tip = self.head()?;
		self.save_header_head(&tip)?;
		self.save_sync_head(&tip)
	}

	/// get block
	pub fn get_block(&self, h: &Hash) -> Result<Block, Error> {
		option_to_not_found(
			self.db.get_ser(&to_key(BLOCK_PREFIX, &mut h.to_vec())),
			&format!("Block with hash: {}", h),
		)
	}

	pub fn block_exists(&self, h: &Hash) -> Result<bool, Error> {
		self.db.exists(&to_key(BLOCK_PREFIX, &mut h.to_vec()))
	}

	/// Save the block and its header, caching the header.
	pub fn save_block(&self, b: &Block) -> Result<(), Error> {
		self.save_block_header(&b.header)?;
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

	pub fn save_block_header(&self, header: &BlockHeader) -> Result<(), Error> {
		let hash = header.hash();

		{
			let mut header_cache = self.header_cache.write().unwrap();
			header_cache.insert(hash, header.clone());
		}

		self.db
			.put_ser(&to_key(BLOCK_HEADER_PREFIX, &mut hash.to_vec())[..], header)?;
		Ok(())
	}

	pub fn save_header_height(&self, bh: &BlockHeader) -> Result<(), Error> {
		self.db
			.put_ser(&u64_to_key(HEADER_HEIGHT_PREFIX, bh.height), &bh.hash())
	}

	pub fn delete_header_by_height(&self, height: u64) -> Result<(), Error> {
		self.db.delete(&u64_to_key(HEADER_HEIGHT_PREFIX, height))
	}

	pub fn save_output_pos(&self, commit: &Commitment, pos: u64) -> Result<(), Error> {
		self.db.put_ser(
			&to_key(COMMIT_POS_PREFIX, &mut commit.as_ref().to_vec())[..],
			&pos,
		)
	}

	pub fn get_output_pos(&self, commit: &Commitment) -> Result<u64, Error> {
		option_to_not_found(
			self.db
				.get_ser(&to_key(COMMIT_POS_PREFIX, &mut commit.as_ref().to_vec())),
			&format!("Output position for commit: {:?}", commit),
		)
	}

	pub fn delete_output_pos(&self, commit: &[u8]) -> Result<(), Error> {
		self.db
			.delete(&to_key(COMMIT_POS_PREFIX, &mut commit.to_vec()))
	}

	pub fn get_block_header(&self, h: &Hash) -> Result<BlockHeader, Error> {
		{
			let mut header_cache = self.header_cache.write().unwrap();

			// cache hit - return the value from the cache
			if let Some(header) = header_cache.get_mut(h) {
				return Ok(header.clone());
			}
		}

		let header: Result<BlockHeader, Error> = option_to_not_found(
			self.db
				.get_ser(&to_key(BLOCK_HEADER_PREFIX, &mut h.to_vec())),
			&format!("BLOCK HEADER: {}", h),
		);

		// cache miss - so adding to the cache for next time
		if let Ok(header) = header {
			{
				let mut header_cache = self.header_cache.write().unwrap();
				header_cache.insert(*h, header.clone());
			}
			Ok(header)
		} else {
			header
		}
	}

	fn save_block_input_bitmap(&self, bh: &Hash, bm: &Bitmap) -> Result<(), Error> {
		self.db.put(
			&to_key(BLOCK_INPUT_BITMAP_PREFIX, &mut bh.to_vec())[..],
			bm.serialize(),
		)
	}

	fn delete_block_input_bitmap(&self, bh: &Hash) -> Result<(), Error> {
		self.db
			.delete(&to_key(BLOCK_INPUT_BITMAP_PREFIX, &mut bh.to_vec()))
	}

	pub fn save_block_sums(&self, h: &Hash, sums: &BlockSums) -> Result<(), Error> {
		{
			let mut block_sums_cache = self.block_sums_cache.write().unwrap();
			block_sums_cache.insert(*h, sums.clone());
		}

		self.db
			.put_ser(&to_key(BLOCK_SUMS_PREFIX, &mut h.to_vec())[..], &sums)
	}

	pub fn get_block_sums(&self, h: &Hash) -> Result<BlockSums, Error> {
		{
			let mut block_sums_cache = self.block_sums_cache.write().unwrap();

			// cache hit - return the value from the cache
			if let Some(block_sums) = block_sums_cache.get_mut(h) {
				return Ok(block_sums.clone());
			}
		}

		let block_sums: Result<BlockSums, Error> = option_to_not_found(
			self.db.get_ser(&to_key(BLOCK_SUMS_PREFIX, &mut h.to_vec())),
			&format!("Block sums for block: {}", h),
		);

		// cache miss - so adding to the cache for next time
		if let Ok(block_sums) = block_sums {
			{
				let mut block_sums_cache = self.block_sums_cache.write().unwrap();
				block_sums_cache.insert(*h, block_sums.clone());
			}
			Ok(block_sums)
		} else {
			block_sums
		}
	}

	fn delete_block_sums(&self, bh: &Hash) -> Result<(), Error> {
		self.db.delete(&to_key(BLOCK_SUMS_PREFIX, &mut bh.to_vec()))
	}

	// We are on the current chain if -
	// * the header by height index matches the header, and
	// * we are not ahead of the current head
	pub fn is_on_current_chain(&self, header: &BlockHeader) -> Result<(), Error> {
		let head = self.head()?;

		// check we are not out ahead of the current head
		if header.height > head.height {
			return Err(Error::NotFoundErr(String::from(
				"header.height > head.height",
			)));
		}

		let header_at_height = self.get_header_by_height(header.height)?;
		if header.hash() == header_at_height.hash() {
			Ok(())
		} else {
			Err(Error::NotFoundErr(String::from(
				"header.hash == header_at_height.hash",
			)))
		}
	}

	pub fn get_header_by_height(&self, height: u64) -> Result<BlockHeader, Error> {
		option_to_not_found(
			self.db.get_ser(&u64_to_key(HEADER_HEIGHT_PREFIX, height)),
			&format!("Header at height: {}", height),
		).and_then(|hash| self.get_block_header(&hash))
	}

	/// Maintain consistency of the "header_by_height" index by traversing back
	/// through the current chain and updating "header_by_height" until we reach
	/// a block_header that is consistent with its height (everything prior to
	/// this will be consistent).
	/// We need to handle the case where we have no index entry for a given
	/// height to account for the case where we just switched to a new fork and
	/// the height jumped beyond current chain height.
	pub fn setup_height(&self, header: &BlockHeader, old_tip: &Tip) -> Result<(), Error> {
		// remove headers ahead if we backtracked
		for n in header.height..old_tip.height {
			self.delete_header_by_height(n + 1)?;
		}
		self.build_by_height_index(header, false)
	}

	pub fn build_by_height_index(&self, header: &BlockHeader, force: bool) -> Result<(), Error> {
		self.save_header_height(&header)?;

		if header.height > 0 {
			let mut prev_header = self.get_block_header(&header.previous)?;
			while prev_header.height > 0 {
				if !force {
					if let Ok(_) = self.is_on_current_chain(&prev_header) {
						break;
					}
				}
				self.save_header_height(&prev_header)?;

				prev_header = self.get_block_header(&prev_header.previous)?;
			}
		}
		Ok(())
	}

	fn build_block_input_bitmap(&self, block: &Block) -> Result<Bitmap, Error> {
		let bitmap = block
			.inputs()
			.iter()
			.filter_map(|x| self.get_output_pos(&x.commitment()).ok())
			.map(|x| x as u32)
			.collect();
		Ok(bitmap)
	}

	pub fn build_and_cache_block_input_bitmap(&self, block: &Block) -> Result<Bitmap, Error> {
		// Build the bitmap.
		let bitmap = self.build_block_input_bitmap(block)?;

		// Save the bitmap to the db (via the batch).
		self.save_block_input_bitmap(&block.hash(), &bitmap)?;

		// Finally cache it locally for use later.
		let mut cache = self.block_input_bitmap_cache.write().unwrap();
		cache.insert(block.hash(), bitmap.serialize());

		Ok(bitmap)
	}

	pub fn get_block_input_bitmap(&self, bh: &Hash) -> Result<Bitmap, Error> {
		{
			let mut cache = self.block_input_bitmap_cache.write().unwrap();

			// cache hit - return the value from the cache
			if let Some(bytes) = cache.get_mut(bh) {
				return Ok(Bitmap::deserialize(&bytes));
			}
		}

		// cache miss - get it from db (build it, store it and cache it as necessary)
		self.get_block_input_bitmap_db(bh)
	}

	// Get the block input bitmap from the db or build the bitmap from
	// the full block from the db (if the block is found).
	// (bool, Bitmap) : (false if bitmap was built and not found in db)
	fn get_block_input_bitmap_db(&self, bh: &Hash) -> Result<Bitmap, Error> {
		if let Ok(Some(bytes)) = self
			.db
			.get(&to_key(BLOCK_INPUT_BITMAP_PREFIX, &mut bh.to_vec()))
		{
			Ok(Bitmap::deserialize(&bytes))
		} else {
			match self.get_block(bh) {
				Ok(block) => {
					let bitmap = self.build_and_cache_block_input_bitmap(&block)?;
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
	pub fn child(&mut self) -> Result<Batch, Error> {
		Ok(Batch {
			db: self.db.child()?,
			header_cache: self.header_cache.clone(),
			block_sums_cache: self.block_sums_cache.clone(),
			block_input_bitmap_cache: self.block_input_bitmap_cache.clone(),
		})
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
	pub fn from_batch(start: Hash, batch: Batch) -> DifficultyIter {
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
				self.prev_header = batch.get_block_header(&header.previous).ok();
			} else {
				if let Some(ref store) = self.store {
					self.prev_header = store.get_block_header(&header.previous).ok();
				} else {
					self.prev_header = None;
				}
			}

			let prev_difficulty = self
				.prev_header
				.clone()
				.map_or(Difficulty::zero(), |x| x.total_difficulty());
			let difficulty = header.total_difficulty() - prev_difficulty;
			let scaling = header.pow.scaling_difficulty;

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
