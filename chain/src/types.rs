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

use std::{error, fmt, io};

use util::secp;
use util::secp::pedersen::Commitment;
use util::secp_static;

use core::core::committed;
use core::core::hash::{Hash, Hashed};
use core::core::target::Difficulty;
use core::core::{block, transaction, Block, BlockHeader};
use core::ser::{self, Readable, Reader, Writeable, Writer};
use grin_store as store;
use keychain;

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
/// readable
#[derive(Debug)]
pub struct TxHashSetRoots {
	/// Output root
	pub output_root: Hash,
	/// Range Proof root
	pub rproof_root: Hash,
	/// Kernel root
	pub kernel_root: Hash,
}

/// Errors
#[derive(Debug)]
pub enum Error {
	/// The block doesn't fit anywhere in our chain
	Unfit(String),
	/// Special case of orphan blocks
	Orphan,
	/// Difficulty is too low either compared to ours or the block PoW hash
	DifficultyTooLow,
	/// Addition of difficulties on all previous block is wrong
	WrongTotalDifficulty,
	/// The proof of work is invalid
	InvalidPow,
	/// The block doesn't sum correctly or a tx signature is invalid
	InvalidBlockProof(block::Error),
	/// Block time is too old
	InvalidBlockTime,
	/// Block height is invalid (not previous + 1)
	InvalidBlockHeight,
	/// One of the root hashes in the block is invalid
	InvalidRoot,
	/// One of the MMR sizes in the block header is invalid
	InvalidMMRSize,
	/// Error from underlying keychain impl
	Keychain(keychain::Error),
	/// Error from underlying secp lib
	Secp(secp::Error),
	/// One of the inputs in the block has already been spent
	AlreadySpent(Commitment),
	/// An output with that commitment already exists (should be unique)
	DuplicateCommitment(Commitment),
	/// Attempt to spend a coinbase output before it sufficiently matures.
	ImmatureCoinbase,
	/// Error validating a Merkle proof (coinbase output)
	MerkleProof,
	/// output not found
	OutputNotFound,
	/// output spent
	OutputSpent,
	/// Invalid block version, either a mistake or outdated software
	InvalidBlockVersion(u16),
	/// We've been provided a bad txhashset
	InvalidTxHashSet(String),
	/// Internal issue when trying to save or load data from store
	StoreErr(store::Error, String),
	/// Internal issue when trying to save or load data from append only files
	FileReadErr(String),
	/// Error serializing or deserializing a type
	SerErr(ser::Error),
	/// Error with the txhashset
	TxHashSetErr(String),
	/// Tx not valid based on lock_height.
	TxLockHeight,
	/// No chain exists and genesis block is required
	GenesisBlockRequired,
	/// Error from underlying tx handling
	Transaction(transaction::Error),
	/// Anything else
	Other(String),
	/// Error from summing and verifying kernel sums via committed trait.
	Committed(committed::Error),
}

impl error::Error for Error {
	fn description(&self) -> &str {
		match *self {
			_ => "some kind of chain error",
		}
	}
}

impl fmt::Display for Error {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		match *self {
			_ => write!(f, "some kind of chain error"),
		}
	}
}

impl From<store::Error> for Error {
	fn from(e: store::Error) -> Error {
		Error::StoreErr(e, "wrapped".to_owned())
	}
}

impl From<ser::Error> for Error {
	fn from(e: ser::Error) -> Error {
		Error::SerErr(e)
	}
}

impl From<io::Error> for Error {
	fn from(e: io::Error) -> Error {
		Error::TxHashSetErr(e.to_string())
	}
}

impl From<keychain::Error> for Error {
	fn from(e: keychain::Error) -> Error {
		Error::Keychain(e)
	}
}

impl From<secp::Error> for Error {
	fn from(e: secp::Error) -> Error {
		Error::Secp(e)
	}
}

impl From<committed::Error> for Error {
	fn from(e: committed::Error) -> Error {
		Error::Committed(e)
	}
}

impl Error {
	/// Whether the error is due to a block that was intrinsically wrong
	pub fn is_bad_data(&self) -> bool {
		// shorter to match on all the "not the block's fault" errors
		match *self {
			Error::Unfit(_)
			| Error::Orphan
			| Error::StoreErr(_, _)
			| Error::SerErr(_)
			| Error::TxHashSetErr(_)
			| Error::GenesisBlockRequired
			| Error::Other(_) => false,
			_ => true,
		}
	}
}

impl From<transaction::Error> for Error {
	fn from(e: transaction::Error) -> Error {
		Error::Transaction(e)
	}
}

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

/// Dummy adapter used as a placeholder for real implementations
pub struct NoopAdapter {}

impl ChainAdapter for NoopAdapter {
	fn block_accepted(&self, _: &Block, _: Options) {}
}
