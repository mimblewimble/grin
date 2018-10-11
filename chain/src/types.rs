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

//! Base types that the block chain pipeline requires.

use core::core::hash::{Hash, Hashed};
use core::core::{Block, BlockHeader};
use core::pow::Difficulty;
use core::ser;

bitflags! {
/// Options for block validation
	pub struct Options: u32 {
		/// No flags
		const NONE = 0b00000000;
		/// Runs without checking the Proof of Work, mostly to make testing easier.
		const SKIP_POW = 0b00000001;
		/// Adds block while in syncing mode.
		const SYNC = 0b00000010;
		/// Block validation on a block we mined ourselves
		const MINE = 0b00000100;
	}
}

/// A helper to hold the roots of the txhashset in order to keep them
/// readable.
#[derive(Debug)]
pub struct TxHashSetRoots {
	/// Header root
	pub header_root: Hash,
	/// Output root
	pub output_root: Hash,
	/// Range Proof root
	pub rproof_root: Hash,
	/// Kernel root
	pub kernel_root: Hash,
}

/// The tip of a fork. A handle to the fork ancestry from its leaf in the
/// blockchain tree. References the max height and the latest and previous
/// blocks
/// for convenience and the total difficulty.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
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
			total_difficulty: bh.total_difficulty(),
		}
	}
}

/// Serialization of a tip, required to save to datastore.
impl ser::Writeable for Tip {
	fn write<W: ser::Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		writer.write_u64(self.height)?;
		writer.write_fixed_bytes(&self.last_block_h)?;
		writer.write_fixed_bytes(&self.prev_block_h)?;
		self.total_difficulty.write(writer)
	}
}

impl ser::Readable for Tip {
	fn read(reader: &mut ser::Reader) -> Result<Tip, ser::Error> {
		let height = reader.read_u64()?;
		let last = Hash::read(reader)?;
		let prev = Hash::read(reader)?;
		let diff = Difficulty::read(reader)?;
		Ok(Tip {
			height: height,
			last_block_h: last,
			prev_block_h: prev,
			total_difficulty: diff,
		})
	}
}

/// Bridge between the chain pipeline and the rest of the system. Handles
/// downstream processing of valid blocks by the rest of the system, most
/// importantly the broadcasting of blocks to our peers.
pub trait ChainAdapter {
	/// The blockchain pipeline has accepted this block as valid and added
	/// it to our chain.
	fn block_accepted(&self, b: &Block, opts: Options);
}

/// Inform the caller of the current status of a txhashset write operation,
/// as it can take quite a while to process. Each function is called in the
/// order defined below and can be used to provide some feedback to the
/// caller. Functions taking arguments can be called repeatedly to update
/// those values as the processing progresses.
pub trait TxHashsetWriteStatus {
	/// First setup of the txhashset
	fn on_setup(&self);
	/// Starting validation
	fn on_validation(&self, kernels: u64, kernel_total: u64, rproofs: u64, rproof_total: u64);
	/// Starting to save the txhashset and related data
	fn on_save(&self);
	/// Done writing a new txhashset
	fn on_done(&self);
}

/// Do-nothing implementation of TxHashsetWriteStatus
pub struct NoStatus;

impl TxHashsetWriteStatus for NoStatus {
	fn on_setup(&self) {}
	fn on_validation(&self, _ks: u64, _kts: u64, _rs: u64, _rt: u64) {}
	fn on_save(&self) {}
	fn on_done(&self) {}
}

/// Dummy adapter used as a placeholder for real implementations
pub struct NoopAdapter {}

impl ChainAdapter for NoopAdapter {
	fn block_accepted(&self, _: &Block, _: Options) {}
}
