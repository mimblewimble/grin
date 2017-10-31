// Copyright 2016 The Grin Developers
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
use core::core::{Block, BlockHeader, Output};
use core::consensus::TargetError;
use core::core::target::Difficulty;
use grin_store::{self, option_to_not_found, to_key, Error, u64_to_key};

const STORE_SUBPATH: &'static str = "chain";

const BLOCK_HEADER_PREFIX: u8 = 'h' as u8;
const BLOCK_PREFIX: u8 = 'b' as u8;
const HEAD_PREFIX: u8 = 'H' as u8;
const HEADER_HEAD_PREFIX: u8 = 'I' as u8;
const HEADER_HEIGHT_PREFIX: u8 = '8' as u8;
const OUTPUT_COMMIT_PREFIX: u8 = 'o' as u8;
const HEADER_BY_OUTPUT_PREFIX: u8 = 'p' as u8;
const COMMIT_POS_PREFIX: u8 = 'c' as u8;
const KERNEL_POS_PREFIX: u8 = 'k' as u8;

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

	fn get_block(&self, h: &Hash) -> Result<Block, Error> {
		option_to_not_found(self.db.get_ser(&to_key(BLOCK_PREFIX, &mut h.to_vec())))
	}

	fn get_block_header(&self, h: &Hash) -> Result<BlockHeader, Error> {
		option_to_not_found(
			self.db
				.get_ser(&to_key(BLOCK_HEADER_PREFIX, &mut h.to_vec())),
		)
	}

	fn check_block_exists(&self, h: &Hash) -> Result<bool, Error> {
		self.db.exists(&to_key(BLOCK_PREFIX, &mut h.to_vec()))
	}

	fn save_block(&self, b: &Block) -> Result<(), Error> {
		// saving the block and its header
		let mut batch = self.db
			.batch()
			.put_ser(&to_key(BLOCK_PREFIX, &mut b.hash().to_vec())[..], b)?
			.put_ser(
				&to_key(BLOCK_HEADER_PREFIX, &mut b.hash().to_vec())[..],
				&b.header,
			)?;

		// saving the full output under its hash, as well as a commitment to hash index
		for out in &b.outputs {
			batch = batch
				.put_ser(
					&to_key(
						OUTPUT_COMMIT_PREFIX,
						&mut out.commitment().as_ref().to_vec(),
					)[..],
					out,
				)?
				.put_ser(
					&to_key(
						HEADER_BY_OUTPUT_PREFIX,
						&mut out.commitment().as_ref().to_vec(),
					)[..],
					&b.hash(),
				)?;
		}
		batch.write()
	}

	// lookup the block header hash by output commitment
 // lookup the block header based on this hash
 // to check the chain is correct compare this block header to
 // the block header currently indexed at the relevant block height (tbd if
 // actually necessary)
 //
 // NOTE: This index is not exhaustive.
 // This node may not have seen this full block, so may not have populated the
 // index.
 // Block headers older than some threshold (2 months?) will not necessarily be
 // included
 // in this index.
 //
	fn get_block_header_by_output_commit(&self, commit: &Commitment) -> Result<BlockHeader, Error> {
		let block_hash = self.db.get_ser(&to_key(
			HEADER_BY_OUTPUT_PREFIX,
			&mut commit.as_ref().to_vec(),
		))?;

		match block_hash {
			Some(hash) => {
				let block_header = self.get_block_header(&hash)?;
				let header_at_height = self.get_header_by_height(block_header.height)?;
				if block_header.hash() == header_at_height.hash() {
					Ok(block_header)
				} else {
					Err(Error::NotFoundErr)
				}
			}
			None => Err(Error::NotFoundErr),
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
	}

	fn get_output_by_commit(&self, commit: &Commitment) -> Result<Output, Error> {
		option_to_not_found(
			self.db
				.get_ser(&to_key(OUTPUT_COMMIT_PREFIX, &mut commit.as_ref().to_vec())),
		)
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

	fn save_kernel_pos(&self, excess: &Commitment, pos: u64) -> Result<(), Error> {
		self.db.put_ser(
			&to_key(KERNEL_POS_PREFIX, &mut excess.as_ref().to_vec())[..],
			&pos,
		)
	}

	fn get_kernel_pos(&self, excess: &Commitment) -> Result<u64, Error> {
		option_to_not_found(
			self.db
				.get_ser(&to_key(KERNEL_POS_PREFIX, &mut excess.as_ref().to_vec())),
		)
	}

	/// Maintain consistency of the "header_by_height" index by traversing back
	/// through the
	/// current chain and updating "header_by_height" until we reach a
	/// block_header
	/// that is consistent with its height (everything prior to this will be
	/// consistent)
	fn setup_height(&self, bh: &BlockHeader) -> Result<(), Error> {
		self.db
			.put_ser(&u64_to_key(HEADER_HEIGHT_PREFIX, bh.height), bh)?;
		if bh.height == 0 {
			return Ok(());
		}

		let mut prev_h = bh.previous;
		let mut prev_height = bh.height - 1;
		while prev_height > 0 {
			let prev = self.get_header_by_height(prev_height)?;
			if prev.hash() != prev_h {
				let real_prev = self.get_block_header(&prev_h)?;
				self.db
					.put_ser(
						&u64_to_key(HEADER_HEIGHT_PREFIX, real_prev.height),
						&real_prev,
					)
					.unwrap();
				prev_h = real_prev.previous;
				prev_height = real_prev.height - 1;
			} else {
				break;
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
			Err(e) => Some(Err(TargetError(e.to_string()))),
			Ok(bh) => {
				if bh.height == 0 {
					return None;
				}
				self.next = bh.previous;
				Some(Ok((bh.timestamp.to_timespec().sec as u64, bh.difficulty)))
			}
		}
	}
}
