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
use serde_json;

use failure::ResultExt;

use core::core::hash::Hash;
use core::ser;

use keychain::{Identifier, Keychain};

use libtx::slate::Slate;
use libwallet::error::{Error, ErrorKind};

use util::secp::pedersen;

/// Combined trait to allow dynamic wallet dispatch
pub trait WalletInst<K>: WalletBackend<K> + WalletClient + Send + Sync + 'static
where
	K: Keychain,
{
}
impl<T, K> WalletInst<K> for T
where
	T: WalletBackend<K> + WalletClient + Send + Sync + 'static,
	K: Keychain,
{
}

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

	/// Iterate over all output data stored by the backend
	fn iter<'a>(&'a self) -> Box<Iterator<Item = OutputData> + 'a>;

	/// Get output data by id
	fn get(&self, id: &Identifier) -> Result<OutputData, Error>;

	/// Create a new write batch to update or remove output data
	fn batch<'a>(&'a mut self) -> Result<Box<WalletOutputBatch + 'a>, Error>;

	/// Next child ID when we want to create a new output
	fn next_child<'a>(&mut self, root_key_id: Identifier) -> Result<u32, Error>;

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

/// Batch trait to update the output data backend atomically. Trying to use a
/// batch after commit MAY result in a panic. Due to this being a trait, the
/// commit method can't take ownership.
pub trait WalletOutputBatch {
	/// Add or update data about an output to the backend
	fn save(&mut self, out: OutputData) -> Result<(), Error>;

	/// Get wallet details
	fn details(&mut self) -> &mut WalletDetails;

	/// Gets output data by id
	fn get(&self, id: &Identifier) -> Result<OutputData, Error>;

	/// Iterate over all output data in batch
	fn iter<'b>(&'b self) -> Box<Iterator<Item = OutputData> + 'b>;

	/// Delete data about an output to the backend
	fn delete(&mut self, id: &Identifier) -> Result<(), Error>;

	/// Save an output as locked in the backend
	fn lock_output(&mut self, out: &mut OutputData) -> Result<(), Error>;

	/// Write the wallet data to backend file
	fn commit(&self) -> Result<(), Error>;
}

/// Encapsulate all communication functions. No functions within libwallet
/// should care about communication details
pub trait WalletClient {
	/// Return the URL of the check node
	fn node_url(&self) -> &str;

	/// Call the wallet API to create a coinbase transaction
	fn create_coinbase(&self, block_fees: &BlockFees) -> Result<CbData, Error>;

	/// Send a transaction slate to another listening wallet and return result
	/// TODO: Probably need a slate wrapper type
	fn send_tx_slate(&self, addr: &str, slate: &Slate) -> Result<Slate, Error>;

	/// Posts a transaction to a grin node
	fn post_tx(&self, tx: &TxWrapper, fluff: bool) -> Result<(), Error>;

	/// retrieves the current tip from the specified grin node
	fn get_chain_height(&self) -> Result<u64, Error>;

	/// retrieve a list of outputs from the specified grin node
	/// need "by_height" and "by_id" variants
	fn get_outputs_from_node(
		&self,
		wallet_outputs: Vec<pedersen::Commitment>,
	) -> Result<HashMap<pedersen::Commitment, String>, Error>;

	/// Get a list of outputs from the node by traversing the UTXO
	/// set in PMMR index order.
	/// Returns
	/// (last available output index, last insertion index retrieved,
	/// outputs(commit, proof, is_coinbase))
	fn get_outputs_by_pmmr_index(
		&self,
		start_height: u64,
		max_outputs: u64,
	) -> Result<
		(
			u64,
			u64,
			Vec<(pedersen::Commitment, pedersen::RangeProof, bool)>,
		),
		Error,
	>;
}

/// Information about an output that's being tracked by the wallet. Must be
/// enough to reconstruct the commitment associated with the ouput when the
/// root private key is known.

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
}

impl ser::Writeable for OutputData {
	fn write<W: ser::Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		writer.write_bytes(&serde_json::to_vec(self).map_err(|_| ser::Error::CorruptedData)?)
	}
}

impl ser::Readable for OutputData {
	fn read(reader: &mut ser::Reader) -> Result<OutputData, ser::Error> {
		let data = reader.read_vec()?;
		serde_json::from_slice(&data[..]).map_err(|_| ser::Error::CorruptedData)
	}
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
