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

//! Error types for chain
use crate::core::core::pmmr::segment;
use crate::core::core::{block, committed, transaction};
use crate::core::ser;
use crate::keychain;
use crate::util::secp;
use crate::util::secp::pedersen::Commitment;
use grin_store as store;
use std::io;

/// Chain error definitions
#[derive(Clone, Eq, PartialEq, Debug, thiserror::Error)]
pub enum Error {
	/// The block doesn't fit anywhere in our chain
	#[error("Block is unfit: {0}")]
	Unfit(String),
	/// Special case of orphan blocks
	#[error("Orphan")]
	Orphan,
	/// Difficulty is too low either compared to ours or the block PoW hash
	#[error("Difficulty is too low compared to ours or the block PoW hash")]
	DifficultyTooLow,
	/// Addition of difficulties on all previous block is wrong
	#[error("Addition of difficulties on all previous blocks is wrong")]
	WrongTotalDifficulty,
	/// Block header edge_bits is lower than our min
	#[error("Cuckoo Size too small")]
	LowEdgebits,
	/// Scaling factor between primary and secondary PoW is invalid
	#[error("Wrong scaling factor")]
	InvalidScaling,
	/// The proof of work is invalid
	#[error("Invalid PoW")]
	InvalidPow,
	/// Peer abusively sending us an old block we already have
	#[error("Old Block")]
	OldBlock,
	/// The block doesn't sum correctly or a tx signature is invalid
	#[error("Invalid Block Proof")]
	InvalidBlockProof {
		#[from]
		/// Conversion
		source: block::Error,
	},
	/// Block time is too old
	#[error("Invalid Block Time")]
	InvalidBlockTime,
	/// Block height is invalid (not previous + 1)
	#[error("Invalid Block Height")]
	InvalidBlockHeight,
	/// One of the root hashes in the block is invalid
	#[error("Invalid Root")]
	InvalidRoot,
	/// One of the MMR sizes in the block header is invalid
	#[error("Invalid MMR Size")]
	InvalidMMRSize,
	/// Error from underlying keychain impl
	#[error("Keychain Error")]
	Keychain {
		#[from]
		/// Conversion
		source: keychain::Error,
	},
	/// Error from underlying secp lib
	#[error("Secp Lib Error")]
	Secp {
		#[from]
		/// Conversion
		source: secp::Error,
	},
	/// One of the inputs in the block has already been spent
	#[error("Already Spent: {0:?}")]
	AlreadySpent(Commitment),
	/// An output with that commitment already exists (should be unique)
	#[error("Duplicate Commitment: {0:?}")]
	DuplicateCommitment(Commitment),
	/// Attempt to spend a coinbase output before it sufficiently matures.
	#[error("Attempt to spend immature coinbase")]
	ImmatureCoinbase,
	/// Error validating a Merkle proof (coinbase output)
	#[error("Error validating merkle proof")]
	MerkleProof,
	/// Output not found
	#[error("Output not found")]
	OutputNotFound,
	/// Rangeproof not found
	#[error("Rangeproof not found")]
	RangeproofNotFound,
	/// Tx kernel not found
	#[error("Tx kernel not found")]
	TxKernelNotFound,
	/// output spent
	#[error("Output is spent")]
	OutputSpent,
	/// Invalid block version, either a mistake or outdated software
	#[error("Invalid Block Version: {0:?}")]
	InvalidBlockVersion(block::HeaderVersion),
	/// We've been provided a bad txhashset
	#[error("Invalid TxHashSet: {0}")]
	InvalidTxHashSet(String),
	/// Internal issue when trying to save or load data from store
	#[error("Store Error: {1}, reason: {0}")]
	StoreErr(store::Error, String),
	/// Internal issue when trying to save or load data from append only files
	#[error("File Read Error: {0}")]
	FileReadErr(String),
	/// Error serializing or deserializing a type
	#[error("Serialization Error")]
	SerErr {
		#[from]
		/// Conversion
		source: ser::Error,
	},
	/// Error with the txhashset
	#[error("TxHashSetErr: {0}")]
	TxHashSetErr(String),
	/// Tx not valid based on lock_height.
	#[error("Transaction Lock Height")]
	TxLockHeight,
	/// Tx is not valid due to NRD relative_height restriction.
	#[error("NRD Relative Height")]
	NRDRelativeHeight,
	/// No chain exists and genesis block is required
	#[error("Genesis Block Required")]
	GenesisBlockRequired,
	/// Error from underlying tx handling
	#[error("Transaction Validation Error: {source:?}")]
	Transaction {
		/// Conversion
		#[from]
		source: transaction::Error,
	},
	/// Error from underlying block handling
	#[error("Block Validation Error: {0:?}")]
	Block(block::Error),
	/// Attempt to retrieve a header at a height greater than
	/// the max allowed by u64 limits
	#[error("Invalid Header Height: {0:?}")]
	InvalidHeaderHeight(u64),
	/// Anything else
	#[error("Other Error: {0}")]
	Other(String),
	/// Error from summing and verifying kernel sums via committed trait.
	#[error("Committed Trait: Error summing and verifying kernel sums")]
	Committed {
		#[from]
		/// Conversion
		source: committed::Error,
	},
	/// We cannot process data once the Grin server has been stopped.
	#[error("Stopped (Grin Shutting Down)")]
	Stopped,
	/// Internal Roaring Bitmap error
	#[error("Roaring Bitmap error")]
	Bitmap,
	/// Error during chain sync
	#[error("Sync error")]
	SyncError(String),
	/// PIBD segment related error
	#[error("Segment error")]
	SegmentError {
		#[from]
		/// Conversion
		source: segment::SegmentError,
	},
	/// We've decided to halt the PIBD process due to lack of supporting peers or
	/// otherwise failing to progress for a certain amount of time
	#[error("Aborting PIBD error")]
	AbortingPIBDError,
	/// The segmenter is associated to a different block header
	#[error("Segmenter header mismatch")]
	SegmenterHeaderMismatch,
	/// Segment height not within allowed range
	#[error("Invalid segment height")]
	InvalidSegmentHeight,
	/// Other issue with segment
	#[error("Invalid segment: {0}")]
	InvalidSegment(String),
}

impl Error {
	/// Whether the error is due to a block that was intrinsically wrong
	pub fn is_bad_data(&self) -> bool {
		// shorter to match on all the "not the block's fault" errors
		match self {
			Error::Unfit(_)
			| Error::Orphan
			| Error::StoreErr(_, _)
			| Error::SerErr { .. }
			| Error::TxHashSetErr(_)
			| Error::GenesisBlockRequired
			| Error::Other(_) => false,
			_ => true,
		}
	}
}

impl From<store::Error> for Error {
	fn from(error: store::Error) -> Error {
		Error::StoreErr(error.clone(), format!("{:?}", error))
	}
}

impl From<io::Error> for Error {
	fn from(e: io::Error) -> Error {
		Error::TxHashSetErr(e.to_string())
	}
}
