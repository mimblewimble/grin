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

//! Implements storage primitives required by the chain

use crate::core::consensus::HeaderInfo;
use crate::core::core::hash::{Hash, Hashed};
use crate::core::core::{Block, BlockHeader, BlockSums};
use crate::core::pow::Difficulty;
use crate::core::ser::ProtocolVersion;
use crate::linked_list::MultiIndex;
use crate::types::{CommitPos, Tip};
use crate::util::secp::pedersen::Commitment;
use croaring::Bitmap;
use grin_store as store;
use grin_store::{option_to_not_found, to_key, Error, SerIterator};
use std::convert::TryInto;
use std::sync::Arc;

const STORE_SUBPATH: &str = "chain";

const BLOCK_HEADER_PREFIX: u8 = b'h';
const BLOCK_PREFIX: u8 = b'b';
const HEAD_PREFIX: u8 = b'H';
const TAIL_PREFIX: u8 = b'T';
const HEADER_HEAD_PREFIX: u8 = b'G';
const OUTPUT_POS_PREFIX: u8 = b'p';

/// Prefix for NRD kernel pos index lists.
pub const NRD_KERNEL_LIST_PREFIX: u8 = b'K';
/// Prefix for NRD kernel pos index entries.
pub const NRD_KERNEL_ENTRY_PREFIX: u8 = b'k';

const BLOCK_INPUT_BITMAP_PREFIX: u8 = b'B';
const BLOCK_SUMS_PREFIX: u8 = b'M';
const BLOCK_SPENT_PREFIX: u8 = b'S';

/// All chain-related database operations
pub struct ChainStore {
	db: store::Store,
}

impl ChainStore {
	/// Create new chain store
	pub fn new(db_root: &str) -> Result<ChainStore, Error> {
		let db = store::Store::new(db_root, None, Some(STORE_SUBPATH), None)?;
		Ok(ChainStore { db })
	}

	/// Create a new instance of the chain store based on this instance
	/// but with the provided protocol version. This is used when migrating
	/// data in the db to a different protocol version, reading using one version and
	/// writing back to the db with a different version.
	pub fn with_version(&self, version: ProtocolVersion) -> ChainStore {
		let db_with_version = self.db.with_version(version);
		ChainStore {
			db: db_with_version,
		}
	}

	/// The current chain head.
	pub fn head(&self) -> Result<Tip, Error> {
		option_to_not_found(self.db.get_ser(&[HEAD_PREFIX]), || "HEAD".to_owned())
	}

	/// The current header head (may differ from chain head).
	pub fn header_head(&self) -> Result<Tip, Error> {
		option_to_not_found(self.db.get_ser(&[HEADER_HEAD_PREFIX]), || {
			"HEADER_HEAD".to_owned()
		})
	}

	/// The current chain "tail" (earliest block in the store).
	pub fn tail(&self) -> Result<Tip, Error> {
		option_to_not_found(self.db.get_ser(&[TAIL_PREFIX]), || "TAIL".to_owned())
	}

	/// Header of the block at the head of the block chain (not the same thing as header_head).
	pub fn head_header(&self) -> Result<BlockHeader, Error> {
		self.get_block_header(&self.head()?.last_block_h)
	}

	/// Get full block.
	pub fn get_block(&self, h: &Hash) -> Result<Block, Error> {
		option_to_not_found(self.db.get_ser(&to_key(BLOCK_PREFIX, h)), || {
			format!("BLOCK: {}", h)
		})
	}

	/// Does this full block exist?
	pub fn block_exists(&self, h: &Hash) -> Result<bool, Error> {
		self.db.exists(&to_key(BLOCK_PREFIX, h))
	}

	/// Get block_sums for the block hash.
	pub fn get_block_sums(&self, h: &Hash) -> Result<BlockSums, Error> {
		option_to_not_found(self.db.get_ser(&to_key(BLOCK_SUMS_PREFIX, h)), || {
			format!("Block sums for block: {}", h)
		})
	}

	/// Get previous header.
	pub fn get_previous_header(&self, header: &BlockHeader) -> Result<BlockHeader, Error> {
		self.get_block_header(&header.prev_hash)
	}

	/// Get block header.
	pub fn get_block_header(&self, h: &Hash) -> Result<BlockHeader, Error> {
		option_to_not_found(self.db.get_ser(&to_key(BLOCK_HEADER_PREFIX, h)), || {
			format!("BLOCK HEADER: {}", h)
		})
	}

	/// Get PMMR pos for the given output commitment.
	pub fn get_output_pos(&self, commit: &Commitment) -> Result<u64, Error> {
		match self.get_output_pos_height(commit)? {
			Some(pos) => Ok(pos.pos),
			None => Err(Error::NotFoundErr(format!(
				"Output position for: {:?}",
				commit
			))),
		}
	}

	/// Get PMMR pos and block height for the given output commitment.
	pub fn get_output_pos_height(&self, commit: &Commitment) -> Result<Option<CommitPos>, Error> {
		self.db.get_ser(&to_key(OUTPUT_POS_PREFIX, commit))
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
	/// The underlying db instance.
	pub db: store::Batch<'a>,
}

impl<'a> Batch<'a> {
	/// The head.
	pub fn head(&self) -> Result<Tip, Error> {
		option_to_not_found(self.db.get_ser(&[HEAD_PREFIX]), || "HEAD".to_owned())
	}

	/// The tail.
	pub fn tail(&self) -> Result<Tip, Error> {
		option_to_not_found(self.db.get_ser(&[TAIL_PREFIX]), || "TAIL".to_owned())
	}

	/// The current header head (may differ from chain head).
	pub fn header_head(&self) -> Result<Tip, Error> {
		option_to_not_found(self.db.get_ser(&[HEADER_HEAD_PREFIX]), || {
			"HEADER_HEAD".to_owned()
		})
	}

	/// Header of the block at the head of the block chain (not the same thing as header_head).
	pub fn head_header(&self) -> Result<BlockHeader, Error> {
		self.get_block_header(&self.head()?.last_block_h)
	}

	/// Save body head to db.
	pub fn save_body_head(&self, t: &Tip) -> Result<(), Error> {
		self.db.put_ser(&[HEAD_PREFIX], t)
	}

	/// Save body "tail" to db.
	pub fn save_body_tail(&self, t: &Tip) -> Result<(), Error> {
		self.db.put_ser(&[TAIL_PREFIX], t)
	}

	/// Save header head to db.
	pub fn save_header_head(&self, t: &Tip) -> Result<(), Error> {
		self.db.put_ser(&[HEADER_HEAD_PREFIX], t)
	}

	/// get block
	pub fn get_block(&self, h: &Hash) -> Result<Block, Error> {
		option_to_not_found(self.db.get_ser(&to_key(BLOCK_PREFIX, h)), || {
			format!("Block with hash: {}", h)
		})
	}

	/// Does the block exist?
	pub fn block_exists(&self, h: &Hash) -> Result<bool, Error> {
		self.db.exists(&to_key(BLOCK_PREFIX, h))
	}

	/// Save the block to the db.
	/// Note: the block header is not saved to the db here, assumes this has already been done.
	pub fn save_block(&self, b: &Block) -> Result<(), Error> {
		debug!(
			"save_block: {} at {} ({} -> v{})",
			b.header.hash(),
			b.header.height,
			b.inputs().version_str(),
			self.db.protocol_version(),
		);
		self.db.put_ser(&to_key(BLOCK_PREFIX, b.hash())[..], b)?;
		Ok(())
	}

	/// We maintain a "spent" index for each full block to allow the output_pos
	/// to be easily reverted during rewind.
	pub fn save_spent_index(&self, h: &Hash, spent: &[CommitPos]) -> Result<(), Error> {
		self.db
			.put_ser(&to_key(BLOCK_SPENT_PREFIX, h)[..], &spent.to_vec())?;
		Ok(())
	}

	/// Migrate a block stored in the db by serializing it using the provided protocol version.
	/// Block may have been read using a previous protocol version but we do not actually care.
	pub fn migrate_block(&self, b: &Block, version: ProtocolVersion) -> Result<(), Error> {
		self.db
			.put_ser_with_version(&to_key(BLOCK_PREFIX, b.hash())[..], b, version)?;
		Ok(())
	}

	/// Low level function to delete directly by raw key.
	pub fn delete(&self, key: &[u8]) -> Result<(), Error> {
		self.db.delete(key)
	}

	/// Delete a full block. Does not delete any record associated with a block
	/// header.
	pub fn delete_block(&self, bh: &Hash) -> Result<(), Error> {
		self.db.delete(&to_key(BLOCK_PREFIX, bh)[..])?;

		// Best effort at deleting associated data for this block.
		// Not an error if these fail.
		{
			let _ = self.delete_block_sums(bh);
			let _ = self.delete_spent_index(bh);
		}

		Ok(())
	}

	/// Save block header to db.
	pub fn save_block_header(&self, header: &BlockHeader) -> Result<(), Error> {
		let hash = header.hash();

		// Store the header itself indexed by hash.
		self.db
			.put_ser(&to_key(BLOCK_HEADER_PREFIX, hash)[..], header)?;

		Ok(())
	}

	/// Save output_pos and block height to index.
	pub fn save_output_pos_height(&self, commit: &Commitment, pos: CommitPos) -> Result<(), Error> {
		self.db
			.put_ser(&to_key(OUTPUT_POS_PREFIX, commit)[..], &pos)
	}

	/// Delete the output_pos index entry for a spent output.
	pub fn delete_output_pos_height(&self, commit: &Commitment) -> Result<(), Error> {
		self.db.delete(&to_key(OUTPUT_POS_PREFIX, commit))
	}

	/// When using the output_pos iterator we have access to the index keys but not the
	/// original commitment that the key is constructed from. So we need a way of comparing
	/// a key with another commitment without reconstructing the commitment from the key bytes.
	pub fn is_match_output_pos_key(&self, key: &[u8], commit: &Commitment) -> bool {
		let commit_key = to_key(OUTPUT_POS_PREFIX, commit);
		commit_key == key
	}

	/// Iterator over the output_pos index.
	pub fn output_pos_iter(&self) -> Result<SerIterator<(u64, u64)>, Error> {
		let key = to_key(OUTPUT_POS_PREFIX, "");
		self.db.iter(&key)
	}

	/// Get output_pos from index.
	pub fn get_output_pos(&self, commit: &Commitment) -> Result<u64, Error> {
		match self.get_output_pos_height(commit)? {
			Some(pos) => Ok(pos.pos),
			None => Err(Error::NotFoundErr(format!(
				"Output position for: {:?}",
				commit
			))),
		}
	}

	/// Get output_pos and block height from index.
	pub fn get_output_pos_height(&self, commit: &Commitment) -> Result<Option<CommitPos>, Error> {
		self.db.get_ser(&to_key(OUTPUT_POS_PREFIX, commit))
	}

	/// Get the previous header.
	pub fn get_previous_header(&self, header: &BlockHeader) -> Result<BlockHeader, Error> {
		self.get_block_header(&header.prev_hash)
	}

	/// Get block header.
	pub fn get_block_header(&self, h: &Hash) -> Result<BlockHeader, Error> {
		option_to_not_found(self.db.get_ser(&to_key(BLOCK_HEADER_PREFIX, h)), || {
			format!("BLOCK HEADER: {}", h)
		})
	}

	/// Delete the block spent index.
	fn delete_spent_index(&self, bh: &Hash) -> Result<(), Error> {
		// Clean up the legacy input bitmap as well.
		let _ = self.db.delete(&to_key(BLOCK_INPUT_BITMAP_PREFIX, bh));

		self.db.delete(&to_key(BLOCK_SPENT_PREFIX, bh))
	}

	/// Save block_sums for the block.
	pub fn save_block_sums(&self, h: &Hash, sums: BlockSums) -> Result<(), Error> {
		self.db.put_ser(&to_key(BLOCK_SUMS_PREFIX, h)[..], &sums)
	}

	/// Get block_sums for the block.
	pub fn get_block_sums(&self, h: &Hash) -> Result<BlockSums, Error> {
		option_to_not_found(self.db.get_ser(&to_key(BLOCK_SUMS_PREFIX, h)), || {
			format!("Block sums for block: {}", h)
		})
	}

	/// Delete the block_sums for the block.
	fn delete_block_sums(&self, bh: &Hash) -> Result<(), Error> {
		self.db.delete(&to_key(BLOCK_SUMS_PREFIX, bh))
	}

	/// Get the block input bitmap based on our spent index.
	/// Fallback to legacy block input bitmap from the db.
	pub fn get_block_input_bitmap(&self, bh: &Hash) -> Result<Bitmap, Error> {
		if let Ok(spent) = self.get_spent_index(bh) {
			let bitmap = spent
				.into_iter()
				.map(|x| x.pos.try_into().unwrap())
				.collect();
			Ok(bitmap)
		} else {
			self.get_legacy_input_bitmap(bh)
		}
	}

	fn get_legacy_input_bitmap(&self, bh: &Hash) -> Result<Bitmap, Error> {
		option_to_not_found(
			self.db
				.get_with(&to_key(BLOCK_INPUT_BITMAP_PREFIX, bh), Bitmap::deserialize),
			|| "legacy block input bitmap".to_string(),
		)
	}

	/// Get the "spent index" from the db for the specified block.
	/// If we need to rewind a block then we use this to "unspend" the spent outputs.
	pub fn get_spent_index(&self, bh: &Hash) -> Result<Vec<CommitPos>, Error> {
		option_to_not_found(self.db.get_ser(&to_key(BLOCK_SPENT_PREFIX, bh)), || {
			format!("spent index: {}", bh)
		})
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
		let key = to_key(BLOCK_PREFIX, "");
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
			} else if let Some(ref store) = self.store {
				store.get_block_header(&self.start).ok()
			} else {
				None
			}
		} else {
			self.prev_header.clone()
		};

		// If we have a header we can do this iteration.
		// Otherwise we are done.
		if let Some(header) = self.header.clone() {
			if let Some(ref batch) = self.batch {
				self.prev_header = batch.get_previous_header(&header).ok();
			} else if let Some(ref store) = self.store {
				self.prev_header = store.get_previous_header(&header).ok();
			} else {
				self.prev_header = None;
			}

			let prev_difficulty = self
				.prev_header
				.clone()
				.map_or(Difficulty::zero(), |x| x.total_difficulty());
			let difficulty = header.total_difficulty() - prev_difficulty;
			let scaling = header.pow.secondary_scaling;

			Some(HeaderInfo::new(
				header.hash(),
				header.timestamp.timestamp() as u64,
				difficulty,
				scaling,
				header.pow.is_secondary(),
			))
		} else {
			None
		}
	}
}

/// Init the NRD "recent history" kernel index backed by the underlying db.
/// List index supports multiple entries per key, maintaining insertion order.
/// Allows for fast lookup of the most recent entry per excess commitment.
pub fn nrd_recent_kernel_index() -> MultiIndex<CommitPos> {
	MultiIndex::init(NRD_KERNEL_LIST_PREFIX, NRD_KERNEL_ENTRY_PREFIX)
}
