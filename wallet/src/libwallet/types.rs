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

//! Types and traits that should be provided by a wallet
//! implementation

use std::collections::HashMap;
use std::fmt;

use serde;

use failure::ResultExt;

use core::core::hash::Hash;
use core::core::pmmr::MerkleProof;
use keychain::{Identifier, Keychain};

use libtx::slate::Slate;
use libwallet::error::{Error, ErrorKind};

use util::secp::pedersen;

/// TODO:
/// Wallets should implement this backend for their storage. All functions
/// here expect that the wallet instance has instantiated itself or stored
/// whatever credentials it needs
pub trait WalletBackend<K>
where
	K: Keychain,
{
	/// Initialize with whatever stored credentials we have
	fn open_with_credentials(&mut self) -> Result<(), Error>;

	/// Close wallet and remove any stored credentials (TBD)
	fn close(&mut self) -> Result<(), Error>;

	/// Return the keychain being used
	fn keychain(&mut self) -> &mut K;

	/// Return the outputs directly
	fn outputs(&mut self) -> &mut HashMap<String, OutputData>;

	/// Allows for reading wallet data (read-only)
	fn read_wallet<T, F>(&mut self, f: F) -> Result<T, Error>
	where
		F: FnOnce(&mut Self) -> Result<T, Error>;

	/// Get all outputs from a wallet impl (probably with some sort
	/// of query param), read+write. Implementor should save
	/// any changes to its data and perform any locking needed
	fn with_wallet<T, F>(&mut self, f: F) -> Result<T, Error>
	where
		F: FnOnce(&mut Self) -> T;

	/// Add an output
	fn add_output(&mut self, out: OutputData);

	/// Delete an output
	fn delete_output(&mut self, id: &Identifier);

	/// Lock an output
	fn lock_output(&mut self, out: &OutputData);

	/// get a single output
	fn get_output(&self, key_id: &Identifier) -> Option<&OutputData>;

	/// Next child ID when we want to create a new output
	/// Should also increment index
	fn next_child(&mut self, root_key_id: Identifier) -> u32;

	/// Return current details
	fn details(&mut self) -> &mut WalletDetails;

	/// Select spendable coins from the wallet
	fn select_coins(
		&self,
		root_key_id: Identifier,
		amount: u64,
		current_height: u64,
		minimum_confirmations: u64,
		max_outputs: usize,
		select_all: bool,
	) -> Vec<OutputData>;

	/// Attempt to restore the contents of a wallet from seed
	fn restore(&mut self) -> Result<(), Error>;
}

/// Encapsulate all communication functions. No functions within libwallet
/// should care about communication details
pub trait WalletClient {
	/// Return the URL of the check node
	fn node_url(&self) -> &str;

	/// Call the wallet API to create a coinbase transaction
	fn create_coinbase(&self, dest: &str, block_fees: &BlockFees) -> Result<CbData, Error>;

	/// Send a transaction slate to another listening wallet and return result
	/// TODO: Probably need a slate wrapper type
	fn send_tx_slate(&self, dest: &str, slate: &Slate) -> Result<Slate, Error>;

	/// Posts a transaction to a grin node
	fn post_tx(&self, dest: &str, tx: &TxWrapper, fluff: bool) -> Result<(), Error>;

	/// retrieves the current tip from the specified grin node
	fn get_chain_height(&self, addr: &str) -> Result<u64, Error>;

	/// retrieve a list of outputs from the specified grin node
	/// need "by_height" and "by_id" variants
	fn get_outputs_from_node(
		&self,
		addr: &str,
		wallet_outputs: Vec<pedersen::Commitment>,
	) -> Result<HashMap<pedersen::Commitment, String>, Error>;

	/// Get any missing block hashes from node
	fn get_missing_block_hashes_from_node(
		&self,
		addr: &str,
		height: u64,
		wallet_outputs: Vec<pedersen::Commitment>,
	) -> Result<
		(
			HashMap<pedersen::Commitment, (u64, BlockIdentifier)>,
			HashMap<pedersen::Commitment, MerkleProofWrapper>,
		),
		Error,
	>;

	/// retrieve merkle proof for a commit from a node
	fn get_merkle_proof_for_commit(
		&self,
		addr: &str,
		commit: &str,
	) -> Result<MerkleProofWrapper, Error>;
}

/// Information about an output that's being tracked by the wallet. Must be
/// enough to reconstruct the commitment associated with the output when the
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
	/// Merkle proof
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

	/// Mark an output as spent
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
	/// Unconfirmed
	Unconfirmed,
	/// Unspent
	Unspent,
	/// Locked
	Locked,
	/// Spent
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

/// Wrapper for a merkle proof
#[derive(Debug, Clone, PartialEq, PartialOrd, Eq, Ord)]
pub struct MerkleProofWrapper(pub MerkleProof);

impl MerkleProofWrapper {
	/// Create
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

/// Block Identifier
#[derive(Debug, Clone, PartialEq, PartialOrd, Eq, Ord)]
pub struct BlockIdentifier(pub Hash);

impl BlockIdentifier {
	/// return hash
	pub fn hash(&self) -> Hash {
		self.0
	}

	/// convert to hex string
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

/// Fees in block to use for coinbase amount calculation
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BlockFees {
	/// fees
	pub fees: u64,
	/// height
	pub height: u64,
	/// key id
	pub key_id: Option<Identifier>,
}

impl BlockFees {
	/// return key id
	pub fn key_id(&self) -> Option<Identifier> {
		self.key_id.clone()
	}
}

/// Response to build a coinbase output.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CbData {
	/// Output
	pub output: String,
	/// Kernel
	pub kernel: String,
	/// Key Id
	pub key_id: String,
}

/// a contained wallet info struct, so automated tests can parse wallet info
/// can add more fields here over time as needed
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct WalletInfo {
	/// height from which info was taken
	pub last_confirmed_height: u64,
	/// total amount in the wallet
	pub total: u64,
	/// amount awaiting confirmation
	pub amount_awaiting_confirmation: u64,
	/// coinbases waiting for lock height
	pub amount_immature: u64,
	/// amount currently spendable
	pub amount_currently_spendable: u64,
	/// amount locked via previous transactions
	pub amount_locked: u64,
}

/// Separate data for a wallet, containing fields
/// that are needed but not necessarily represented
/// via simple rows of OutputData
/// If a wallet is restored from seed this is obvious
/// lost and re-populated as well as possible
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct WalletDetails {
	/// The last block height at which the wallet data
	/// was confirmed against a node
	pub last_confirmed_height: u64,
	/// The last child index used
	pub last_child_index: u32,
}

impl Default for WalletDetails {
	fn default() -> WalletDetails {
		WalletDetails {
			last_confirmed_height: 0,
			last_child_index: 0,
		}
	}
}

/// Dummy wrapper for the hex-encoded serialized transaction.
#[derive(Serialize, Deserialize)]
pub struct TxWrapper {
	/// hex representation of transaction
	pub tx_hex: String,
}

/// Send TX API Args
#[derive(Clone, Serialize, Deserialize)]
pub struct SendTXArgs {
	/// amount to send
	pub amount: u64,
	/// minimum confirmations
	pub minimum_confirmations: u64,
	/// destination url
	pub dest: String,
	/// Max number of outputs
	pub max_outputs: usize,
	/// whether to use all outputs (combine)
	pub selection_strategy_is_use_all: bool,
	/// dandelion control
	pub fluff: bool,
}
