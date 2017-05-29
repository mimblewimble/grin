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

//! Base types that the block chain pipeline requires.

use secp::pedersen::Commitment;

use grin_store::Error;
use core::core::{Block, BlockHeader, Output};
use core::core::hash::{Hash, Hashed};
use core::core::target::Difficulty;

/// The tip of a fork. A handle to the fork ancestry from its leaf in the
/// blockchain tree. References the max height and the latest and previous
/// blocks
/// for convenience and the total difficulty.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Tip {
	/// Height of the tip (max height of the fork)
	pub height: u64,
	/// Last block pushed to the fork
	pub last_block_h: Hash,
	/// Block previous to last
	pub prev_block_h: Hash,
	/// Total difficulty accumulated on that fork
	pub total_difficulty: Difficulty,
}

impl Tip {
	/// Creates a new tip at height zero and the provided genesis hash.
	pub fn new(gbh: Hash) -> Tip {
		Tip {
			height: 0,
			last_block_h: gbh,
			prev_block_h: gbh,
			total_difficulty: Difficulty::one(),
		}
	}

	/// Append a new block to this tip, returning a new updated tip.
	pub fn from_block(bh: &BlockHeader) -> Tip {
		Tip {
			height: bh.height,
			last_block_h: bh.hash(),
			prev_block_h: bh.previous,
			total_difficulty: bh.total_difficulty.clone(),
		}
	}
}

/// Trait the chain pipeline requires an implementor for in order to process
/// blocks.
pub trait ChainStore: Send + Sync {
	/// Get the tip that's also the head of the chain
	fn head(&self) -> Result<Tip, Error>;

	/// Block header for the chain head
	fn head_header(&self) -> Result<BlockHeader, Error>;

	/// Save the provided tip as the current head of our chain
	fn save_head(&self, t: &Tip) -> Result<(), Error>;

	/// Save the provided tip as the current head of the body chain, leaving the
	/// header chain alone.
	fn save_body_head(&self, t: &Tip) -> Result<(), Error>;

	/// Gets a block header by hash
	fn get_block(&self, h: &Hash) -> Result<Block, Error>;

	/// Gets a block header by hash
	fn get_block_header(&self, h: &Hash) -> Result<BlockHeader, Error>;

	/// Checks whether a block has been been processed and saved
	fn check_block_exists(&self, h: &Hash) -> Result<bool, Error>;

	/// Save the provided block in store
	fn save_block(&self, b: &Block) -> Result<(), Error>;

	/// Save the provided block header in store
	fn save_block_header(&self, bh: &BlockHeader) -> Result<(), Error>;

	/// Get the tip of the header chain
	fn get_header_head(&self) -> Result<Tip, Error>;

	/// Save the provided tip as the current head of the block header chain
	fn save_header_head(&self, t: &Tip) -> Result<(), Error>;

	/// Gets the block header at the provided height
	fn get_header_by_height(&self, height: u64) -> Result<BlockHeader, Error>;

	/// Gets an output by its commitment
	fn get_output_by_commit(&self, commit: &Commitment) -> Result<Output, Error>;

	/// Checks whether an output commitment exists and returns the output hash
	fn has_output_commit(&self, commit: &Commitment) -> Result<Hash, Error>;

	/// Saves the provided block header at the corresponding height. Also check
	/// the consistency of the height chain in store by assuring previous
	/// headers
	/// are also at their respective heights.
	fn setup_height(&self, bh: &BlockHeader) -> Result<(), Error>;
}

/// Bridge between the chain pipeline and the rest of the system. Handles
/// downstream processing of valid blocks by the rest of the system, most
/// importantly the broadcasting of blocks to our peers.
pub trait ChainAdapter {
	/// The blockchain pipeline has accepted this block as valid and added
	/// it to our chain.
	fn block_accepted(&self, b: &Block);
}

pub struct NoopAdapter { }
impl ChainAdapter for NoopAdapter {
	fn block_accepted(&self, b: &Block) {}
}
