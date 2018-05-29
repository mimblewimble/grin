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

//! Experimental - Types and traits that should be provided by a wallet
//! implementation
use std::fmt::{self, Display};

use serde;

use failure::{Backtrace, Context, Fail, ResultExt};

use core::core::hash::Hash;
use core::core::pmmr::MerkleProof;

use keychain::{Identifier, Keychain};

/// TODO:
pub trait WalletBackend {
	/// Allows for reading wallet data (without needing to acquire the write
	/// lock).
	fn read_wallet<T, F>(f: F) -> Result<T, Error>
	where
		F: FnOnce(&WalletBackend) -> Result<T, Error>;

	/// Get all outputs from a wallet impl (probably with some sort
	/// of query param)
	fn with_wallet<T, F>(f: F) -> Result<T, Error>
	where
		F: FnOnce(&mut WalletBackend) -> T;

	/// Add an output
	fn add_output(&mut self, out: OutputData);

	/// Delete an output
	fn delete_output(&mut self, id: &Identifier) -> Option<&OutputData>;

	/// Lock an output
	fn lock_output(&mut self, out: &OutputData);

	/// get a single output
	fn get_output(&self, key_id: &Identifier) -> Option<&OutputData>;
}

/// TODO:
#[derive(Debug, Clone)]
pub struct Wallet<'a, T: 'a>
where
	T: WalletBackend,
{
	/// All wallets need access to a unique instantiated keychain
	pub keychain: &'a Keychain,
	/// Wallet backend
	pub backend: &'a T,
}

/// Information about an output that's being tracked by the wallet. Must be
/// enough to reconstruct the commitment associated with the ouput when the
/// root private key is known.*/

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, PartialOrd, Eq, Ord)]
pub struct OutputData {
	/// Root key_id that the key for this output is derived from
	pub root_key_id: Identifier,
	/// Derived key for this output
	pub key_id: Identifier,
	/// How many derivations down from the root key
	pub n_child: u32,
	/// Value of the output, necessary to rebuild the commitment
	pub value: u64,
	/// Current status of the output
	pub status: OutputStatus,
	/// Height of the output
	pub height: u64,
	/// Height we are locked until
	pub lock_height: u64,
	/// Is this a coinbase output? Is it subject to coinbase locktime?
	pub is_coinbase: bool,
	/// Hash of the block this output originated from.
	pub block: Option<BlockIdentifier>,
	pub merkle_proof: Option<MerkleProofWrapper>,
}

impl OutputData {
	/// Lock a given output to avoid conflicting use
	pub fn lock(&mut self) {
		self.status = OutputStatus::Locked;
	}

	/// How many confirmations has this output received?
	/// If height == 0 then we are either Unconfirmed or the output was
	/// cut-through
	/// so we do not actually know how many confirmations this output had (and
	/// never will).
	pub fn num_confirmations(&self, current_height: u64) -> u64 {
		if self.status == OutputStatus::Unconfirmed {
			0
		} else if self.height == 0 {
			0
		} else {
			// if an output has height n and we are at block n
			// then we have a single confirmation (the block it originated in)
			1 + (current_height - self.height)
		}
	}

	/// Check if output is eligible to spend based on state and height and
	/// confirmations
	pub fn eligible_to_spend(&self, current_height: u64, minimum_confirmations: u64) -> bool {
		if [OutputStatus::Spent, OutputStatus::Locked].contains(&self.status) {
			return false;
		} else if self.status == OutputStatus::Unconfirmed && self.is_coinbase {
			return false;
		} else if self.is_coinbase && self.block.is_none() {
			// if we do not have a block hash for coinbase output we cannot spent it
			// block index got compacted before we refreshed our wallet?
			return false;
		} else if self.is_coinbase && self.merkle_proof.is_none() {
			// if we do not have a Merkle proof for coinbase output we cannot spent it
			// block index got compacted before we refreshed our wallet?
			return false;
		} else if self.lock_height > current_height {
			return false;
		} else if self.status == OutputStatus::Unspent
			&& self.num_confirmations(current_height) >= minimum_confirmations
		{
			return true;
		} else if self.status == OutputStatus::Unconfirmed && minimum_confirmations == 0 {
			return true;
		} else {
			return false;
		}
	}

	/// Marks this output as unspent if it was previously unconfirmed
	pub fn mark_unspent(&mut self) {
		match self.status {
			OutputStatus::Unconfirmed => self.status = OutputStatus::Unspent,
			_ => (),
		}
	}

	pub fn mark_spent(&mut self) {
		match self.status {
			OutputStatus::Unspent => self.status = OutputStatus::Spent,
			OutputStatus::Locked => self.status = OutputStatus::Spent,
			_ => (),
		}
	}
}
/// Status of an output that's being tracked by the wallet. Can either be
/// unconfirmed, spent, unspent, or locked (when it's been used to generate
/// a transaction but we don't have confirmation that the transaction was
/// broadcasted or mined).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Ord, PartialOrd)]
pub enum OutputStatus {
	Unconfirmed,
	Unspent,
	Locked,
	Spent,
}

impl fmt::Display for OutputStatus {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		match *self {
			OutputStatus::Unconfirmed => write!(f, "Unconfirmed"),
			OutputStatus::Unspent => write!(f, "Unspent"),
			OutputStatus::Locked => write!(f, "Locked"),
			OutputStatus::Spent => write!(f, "Spent"),
		}
	}
}

#[derive(Debug, Clone, PartialEq, PartialOrd, Eq, Ord)]
pub struct MerkleProofWrapper(pub MerkleProof);

impl MerkleProofWrapper {
	pub fn merkle_proof(&self) -> MerkleProof {
		self.0.clone()
	}
}

impl serde::ser::Serialize for MerkleProofWrapper {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: serde::ser::Serializer,
	{
		serializer.serialize_str(&self.0.to_hex())
	}
}

impl<'de> serde::de::Deserialize<'de> for MerkleProofWrapper {
	fn deserialize<D>(deserializer: D) -> Result<MerkleProofWrapper, D::Error>
	where
		D: serde::de::Deserializer<'de>,
	{
		deserializer.deserialize_str(MerkleProofWrapperVisitor)
	}
}

struct MerkleProofWrapperVisitor;

impl<'de> serde::de::Visitor<'de> for MerkleProofWrapperVisitor {
	type Value = MerkleProofWrapper;

	fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
		formatter.write_str("a merkle proof")
	}

	fn visit_str<E>(self, s: &str) -> Result<Self::Value, E>
	where
		E: serde::de::Error,
	{
		let merkle_proof = MerkleProof::from_hex(s).unwrap();
		Ok(MerkleProofWrapper(merkle_proof))
	}
}

#[derive(Debug, Clone, PartialEq, PartialOrd, Eq, Ord)]
pub struct BlockIdentifier(pub Hash);

impl BlockIdentifier {
	pub fn hash(&self) -> Hash {
		self.0
	}

	pub fn from_hex(hex: &str) -> Result<BlockIdentifier, Error> {
		let hash = Hash::from_hex(hex).context(ErrorKind::GenericError("Invalid hex"))?;
		Ok(BlockIdentifier(hash))
	}
}

impl serde::ser::Serialize for BlockIdentifier {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: serde::ser::Serializer,
	{
		serializer.serialize_str(&self.0.to_hex())
	}
}

impl<'de> serde::de::Deserialize<'de> for BlockIdentifier {
	fn deserialize<D>(deserializer: D) -> Result<BlockIdentifier, D::Error>
	where
		D: serde::de::Deserializer<'de>,
	{
		deserializer.deserialize_str(BlockIdentifierVisitor)
	}
}

struct BlockIdentifierVisitor;

impl<'de> serde::de::Visitor<'de> for BlockIdentifierVisitor {
	type Value = BlockIdentifier;

	fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
		formatter.write_str("a block hash")
	}

	fn visit_str<E>(self, s: &str) -> Result<Self::Value, E>
	where
		E: serde::de::Error,
	{
		let block_hash = Hash::from_hex(s).unwrap();
		Ok(BlockIdentifier(block_hash))
	}
}

/// Amount in request to build a coinbase output.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum WalletReceiveRequest {
	Coinbase(BlockFees),
	PartialTransaction(String),
	Finalize(String),
}

/// Fees in block to use for coinbase amount calculation
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BlockFees {
	pub fees: u64,
	pub height: u64,
	pub key_id: Option<Identifier>,
}

impl BlockFees {
	pub fn key_id(&self) -> Option<Identifier> {
		self.key_id.clone()
	}
}

/// Response to build a coinbase output.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CbData {
	pub output: String,
	pub kernel: String,
	pub key_id: String,
}

/// a contained wallet info struct, so automated tests can parse wallet info
/// can add more fields here over time as needed
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct WalletInfo {
	// height from which info was taken
	pub current_height: u64,
	// total amount in the wallet
	pub total: u64,
	// amount awaiting confirmation
	pub amount_awaiting_confirmation: u64,
	// confirmed but locked
	pub amount_confirmed_but_locked: u64,
	// amount currently spendable
	pub amount_currently_spendable: u64,
	// amount locked by previous transactions
	pub amount_locked: u64,
	// whether the data was confirmed against a live node
	pub data_confirmed: bool,
	// node confirming the data
	pub data_confirmed_from: String,
}

#[derive(Debug)]
pub struct Error {
	inner: Context<ErrorKind>,
}

/// Wallet errors, mostly wrappers around underlying crypto or I/O errors.
#[derive(Copy, Clone, Eq, PartialEq, Debug, Fail)]
pub enum ErrorKind {
	#[fail(display = "Not enough funds")]
	NotEnoughFunds(u64),

	#[fail(display = "Fee dispute: sender fee {}, recipient fee {}", sender_fee, recipient_fee)]
	FeeDispute { sender_fee: u64, recipient_fee: u64 },

	#[fail(display = "Fee exceeds amount: sender amount {}, recipient fee {}", sender_amount,
	       recipient_fee)]
	FeeExceedsAmount {
		sender_amount: u64,
		recipient_fee: u64,
	},

	#[fail(display = "Keychain error")]
	Keychain,

	#[fail(display = "Transaction error")]
	Transaction,

	#[fail(display = "Secp error")]
	Secp,

	#[fail(display = "LibWallet error")]
	LibWalletError,

	#[fail(display = "Wallet data error: {}", _0)]
	WalletData(&'static str),

	/// An error in the format of the JSON structures exchanged by the wallet
	#[fail(display = "JSON format error")]
	Format,

	#[fail(display = "I/O error")]
	IO,

	/// Error when contacting a node through its API
	#[fail(display = "Node API error")]
	Node,

	/// Error originating from hyper.
	#[fail(display = "Hyper error")]
	Hyper,

	/// Error originating from hyper uri parsing.
	#[fail(display = "Uri parsing error")]
	Uri,

	#[fail(display = "Signature error")]
	Signature(&'static str),

	/// Attempt to use duplicate transaction id in separate transactions
	#[fail(display = "Duplicate transaction ID error")]
	DuplicateTransactionId,

	/// Wallet seed already exists
	#[fail(display = "Wallet seed exists error")]
	WalletSeedExists,

	/// Wallet seed doesn't exist
	#[fail(display = "Wallet seed doesn't exist error")]
	WalletSeedDoesntExist,

	#[fail(display = "Generic error: {}", _0)]
	GenericError(&'static str),
}

impl Fail for Error {
	fn cause(&self) -> Option<&Fail> {
		self.inner.cause()
	}

	fn backtrace(&self) -> Option<&Backtrace> {
		self.inner.backtrace()
	}
}

impl Display for Error {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		Display::fmt(&self.inner, f)
	}
}

impl Error {
	pub fn kind(&self) -> ErrorKind {
		*self.inner.get_context()
	}
}

impl From<ErrorKind> for Error {
	fn from(kind: ErrorKind) -> Error {
		Error {
			inner: Context::new(kind),
		}
	}
}

impl From<Context<ErrorKind>> for Error {
	fn from(inner: Context<ErrorKind>) -> Error {
		Error { inner: inner }
	}
}
