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

use std::io;

use util::secp;
use util::secp::pedersen::Commitment;

use grin_store as store;
use core::core::{block, transaction, Block, BlockHeader};
use core::core::hash::{Hash, Hashed};
use core::core::target::Difficulty;
use core::ser::{self, Readable, Reader, Writeable, Writer};
use grin_store;
use grin_store::pmmr::PMMRFileMetadata;

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

/// A helper to hold the roots of the sumtrees in order to keep them
/// readable
pub struct SumTreeRoots {
	/// UTXO root
	pub utxo_root: Hash,
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
	/// Something does not look right with the switch commitment
	InvalidSwitchCommit,
	/// One of the inputs in the block has already been spent
	AlreadySpent(Commitment),
	/// An output with that commitment already exists (should be unique)
	DuplicateCommitment(Commitment),
	/// A kernel with that excess commitment already exists (should be unique)
	DuplicateKernel(Commitment),
	/// output not found
	OutputNotFound,
	/// output spent
	OutputSpent,
	/// Invalid block version, either a mistake or outdated software
	InvalidBlockVersion(u16),
	/// We've been provided a bad sumtree
	InvalidSumtree(String),
	/// Internal issue when trying to save or load data from store
	StoreErr(grin_store::Error, String),
	/// Internal issue when trying to save or load data from append only files
	FileReadErr(String),
	/// Error serializing or deserializing a type
	SerErr(ser::Error),
	/// Error with the sumtrees
	SumTreeErr(String),
	/// No chain exists and genesis block is required
	GenesisBlockRequired,
	/// Error from underlying tx handling
	Transaction(transaction::Error),
	/// Anything else
	Other(String),
}

impl From<grin_store::Error> for Error {
	fn from(e: grin_store::Error) -> Error {
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
		Error::SumTreeErr(e.to_string())
	}
}
impl From<secp::Error> for Error {
	fn from(e: secp::Error) -> Error {
		Error::SumTreeErr(format!("Sum validation error: {}", e.to_string()))
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
			| Error::SumTreeErr(_)
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
		try!(writer.write_u64(self.height));
		try!(writer.write_fixed_bytes(&self.last_block_h));
		try!(writer.write_fixed_bytes(&self.prev_block_h));
		self.total_difficulty.write(writer)
	}
}

impl ser::Readable for Tip {
	fn read(reader: &mut ser::Reader) -> Result<Tip, ser::Error> {
		let height = try!(reader.read_u64());
		let last = try!(Hash::read(reader));
		let prev = try!(Hash::read(reader));
		let diff = try!(Difficulty::read(reader));
		Ok(Tip {
			height: height,
			last_block_h: last,
			prev_block_h: prev,
			total_difficulty: diff,
		})
	}
}

/// Trait the chain pipeline requires an implementor for in order to process
/// blocks.
pub trait ChainStore: Send + Sync {
	/// Get the tip that's also the head of the chain
	fn head(&self) -> Result<Tip, store::Error>;

	/// Block header for the chain head
	fn head_header(&self) -> Result<BlockHeader, store::Error>;

	/// Save the provided tip as the current head of our chain
	fn save_head(&self, t: &Tip) -> Result<(), store::Error>;

	/// Save the provided tip as the current head of the body chain, leaving the
	/// header chain alone.
	fn save_body_head(&self, t: &Tip) -> Result<(), store::Error>;

	/// Gets a block header by hash
	fn get_block(&self, h: &Hash) -> Result<Block, store::Error>;

	/// Check whether we have a block without reading it
	fn block_exists(&self, h: &Hash) -> Result<bool, store::Error>;

	/// Gets a block header by hash
	fn get_block_header(&self, h: &Hash) -> Result<BlockHeader, store::Error>;

	/// Save the provided block in store
	fn save_block(&self, b: &Block) -> Result<(), store::Error>;

	/// Delete a full block. Does not delete any record associated with a block
	/// header.
	fn delete_block(&self, bh: &Hash) -> Result<(), store::Error>;

	/// Save the provided block header in store
	fn save_block_header(&self, bh: &BlockHeader) -> Result<(), store::Error>;

	/// Get the tip of the header chain
	fn get_header_head(&self) -> Result<Tip, store::Error>;

	/// Save the provided tip as the current head of the block header chain
	fn save_header_head(&self, t: &Tip) -> Result<(), store::Error>;

	/// Get the tip of the current sync header chain
	fn get_sync_head(&self) -> Result<Tip, store::Error>;

	/// Save the provided tip as the current head of the sync header chain
	fn save_sync_head(&self, t: &Tip) -> Result<(), store::Error>;

	/// Reset header_head and sync_head to head of current body chain
	fn reset_head(&self) -> Result<(), store::Error>;

	/// Gets the block header at the provided height
	fn get_header_by_height(&self, height: u64) -> Result<BlockHeader, store::Error>;

	/// Save a header as associated with its height
	fn save_header_height(&self, header: &BlockHeader) -> Result<(), store::Error>;

	/// Delete the block header at the height
	fn delete_header_by_height(&self, height: u64) -> Result<(), store::Error>;

	/// Is the block header on the current chain?
	/// Use the header_by_height index to verify the block header is where we think it is.
	fn is_on_current_chain(&self, header: &BlockHeader) -> Result<(), store::Error>;

	/// Saves the position of an output, represented by its commitment, in the
	/// UTXO MMR. Used as an index for spending and pruning.
	fn save_output_pos(&self, commit: &Commitment, pos: u64) -> Result<(), store::Error>;

	/// Gets the position of an output, represented by its commitment, in the
	/// UTXO MMR. Used as an index for spending and pruning.
	fn get_output_pos(&self, commit: &Commitment) -> Result<u64, store::Error>;

	/// Saves the position of a kernel, represented by its excess, in the
	/// UTXO MMR. Used as an index for spending and pruning.
	fn save_kernel_pos(&self, commit: &Commitment, pos: u64) -> Result<(), store::Error>;

	/// Gets the position of a kernel, represented by its excess, in the
	/// UTXO MMR. Used as an index for spending and pruning.
	fn get_kernel_pos(&self, commit: &Commitment) -> Result<u64, store::Error>;

	/// Saves information about the last written PMMR file positions for each
	/// committed block
	fn save_block_pmmr_file_metadata(
		&self,
		h: &Hash,
		md: &PMMRFileMetadataCollection,
	) -> Result<(), store::Error>;

	/// Retrieves stored pmmr file metadata information for a given block
	fn get_block_pmmr_file_metadata(
		&self,
		h: &Hash,
	) -> Result<PMMRFileMetadataCollection, store::Error>;

	/// Delete stored pmmr file metadata information for a given block
	fn delete_block_pmmr_file_metadata(&self, h: &Hash) -> Result<(), store::Error>;

	/// Saves the provided block header at the corresponding height. Also check
	/// the consistency of the height chain in store by assuring previous
	/// headers are also at their respective heights.
	fn setup_height(&self, bh: &BlockHeader, old_tip: &Tip) -> Result<(), store::Error>;
}

/// Single serializable struct to hold metadata about all PMMR file position
/// for a given block
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct PMMRFileMetadataCollection {
	/// file metadata for the utxo file
	pub utxo_file_md: PMMRFileMetadata,
	/// file metadata for the rangeproof file
	pub rproof_file_md: PMMRFileMetadata,
	/// file metadata for the kernel file
	pub kernel_file_md: PMMRFileMetadata,
}

impl Writeable for PMMRFileMetadataCollection {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		self.utxo_file_md.write(writer)?;
		self.rproof_file_md.write(writer)?;
		self.kernel_file_md.write(writer)?;
		Ok(())
	}
}

impl Readable for PMMRFileMetadataCollection {
	fn read(reader: &mut Reader) -> Result<PMMRFileMetadataCollection, ser::Error> {
		Ok(PMMRFileMetadataCollection {
			utxo_file_md: PMMRFileMetadata::read(reader)?,
			rproof_file_md: PMMRFileMetadata::read(reader)?,
			kernel_file_md: PMMRFileMetadata::read(reader)?,
		})
	}
}

impl PMMRFileMetadataCollection {
	/// Return empty with all file positions = 0
	pub fn empty() -> PMMRFileMetadataCollection {
		PMMRFileMetadataCollection {
			utxo_file_md: PMMRFileMetadata::empty(),
			rproof_file_md: PMMRFileMetadata::empty(),
			kernel_file_md: PMMRFileMetadata::empty(),
		}
	}

	/// Helper to create a new collection
	pub fn new(
		utxo_md: PMMRFileMetadata,
		rproof_md: PMMRFileMetadata,
		kernel_md: PMMRFileMetadata,
	) -> PMMRFileMetadataCollection {
		PMMRFileMetadataCollection {
			utxo_file_md: utxo_md,
			rproof_file_md: rproof_md,
			kernel_file_md: kernel_md,
		}
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
