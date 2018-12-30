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

use crate::core::core::hash::Hash;
use crate::core::core::Transaction;
use crate::core::libtx::aggsig;
use crate::core::ser;
use crate::keychain::{Identifier, Keychain};
use crate::libwallet::error::{Error, ErrorKind};
use crate::util::secp::key::{PublicKey, SecretKey};
use crate::util::secp::{self, pedersen, Secp256k1};
use chrono::prelude::*;
use failure::ResultExt;
use serde;
use serde_json;
use std::collections::HashMap;
use std::fmt;
use uuid::Uuid;

/// Combined trait to allow dynamic wallet dispatch
pub trait WalletInst<C, K>: WalletBackend<C, K> + Send + Sync + 'static
where
	C: NodeClient,
	K: Keychain,
{
}
impl<T, C, K> WalletInst<C, K> for T
where
	T: WalletBackend<C, K> + Send + Sync + 'static,
	C: NodeClient,
	K: Keychain,
{
}

/// TODO:
/// Wallets should implement this backend for their storage. All functions
/// here expect that the wallet instance has instantiated itself or stored
/// whatever credentials it needs
pub trait WalletBackend<C, K>
where
	C: NodeClient,
	K: Keychain,
{
	/// Initialize with whatever stored credentials we have
	fn open_with_credentials(&mut self) -> Result<(), Error>;

	/// Close wallet and remove any stored credentials (TBD)
	fn close(&mut self) -> Result<(), Error>;

	/// Return the keychain being used
	fn keychain(&mut self) -> &mut K;

	/// Return the client being used to communicate with the node
	fn w2n_client(&mut self) -> &mut C;

	/// Set parent key id by stored account name
	fn set_parent_key_id_by_name(&mut self, label: &str) -> Result<(), Error>;

	/// The BIP32 path of the parent path to use for all output-related
	/// functions, (essentially 'accounts' within a wallet.
	fn set_parent_key_id(&mut self, _: Identifier);

	/// return the parent path
	fn parent_key_id(&mut self) -> Identifier;

	/// Iterate over all output data stored by the backend
	fn iter<'a>(&'a self) -> Box<dyn Iterator<Item = OutputData> + 'a>;

	/// Get output data by id
	fn get(&self, id: &Identifier) -> Result<OutputData, Error>;

	/// Get associated output commitment by id.
	fn get_commitment(&mut self, id: &Identifier) -> Result<pedersen::Commitment, Error>;

	/// Get an (Optional) tx log entry by uuid
	fn get_tx_log_entry(&self, uuid: &Uuid) -> Result<Option<TxLogEntry>, Error>;

	/// Retrieves the private context associated with a given slate id
	fn get_private_context(&mut self, slate_id: &[u8]) -> Result<Context, Error>;

	/// Iterate over all output data stored by the backend
	fn tx_log_iter<'a>(&'a self) -> Box<dyn Iterator<Item = TxLogEntry> + 'a>;

	/// Iterate over all stored account paths
	fn acct_path_iter<'a>(&'a self) -> Box<dyn Iterator<Item = AcctPathMapping> + 'a>;

	/// Gets an account path for a given label
	fn get_acct_path(&self, label: String) -> Result<Option<AcctPathMapping>, Error>;

	/// Stores a transaction
	fn store_tx(&self, uuid: &str, tx: &Transaction) -> Result<(), Error>;

	/// Retrieves a stored transaction from a TxLogEntry
	fn get_stored_tx(&self, entry: &TxLogEntry) -> Result<Option<Transaction>, Error>;

	/// Create a new write batch to update or remove output data
	fn batch<'a>(&'a mut self) -> Result<Box<dyn WalletOutputBatch<K> + 'a>, Error>;

	/// Next child ID when we want to create a new output, based on current parent
	fn next_child<'a>(&mut self) -> Result<Identifier, Error>;

	/// last verified height of outputs directly descending from the given parent key
	fn last_confirmed_height<'a>(&mut self) -> Result<u64, Error>;

	/// Attempt to restore the contents of a wallet from seed
	fn restore(&mut self) -> Result<(), Error>;

	/// Attempt to check and fix wallet state
	fn check_repair(&mut self) -> Result<(), Error>;
}

/// Batch trait to update the output data backend atomically. Trying to use a
/// batch after commit MAY result in a panic. Due to this being a trait, the
/// commit method can't take ownership.
/// TODO: Should these be split into separate batch objects, for outputs,
/// tx_log entries and meta/details?
pub trait WalletOutputBatch<K>
where
	K: Keychain,
{
	/// Return the keychain being used
	fn keychain(&mut self) -> &mut K;

	/// Add or update data about an output to the backend
	fn save(&mut self, out: OutputData) -> Result<(), Error>;

	/// Gets output data by id
	fn get(&self, id: &Identifier) -> Result<OutputData, Error>;

	/// Iterate over all output data stored by the backend
	fn iter(&self) -> Box<dyn Iterator<Item = OutputData>>;

	/// Delete data about an output from the backend
	fn delete(&mut self, id: &Identifier) -> Result<(), Error>;

	/// Save last stored child index of a given parent
	fn save_child_index(&mut self, parent_key_id: &Identifier, child_n: u32) -> Result<(), Error>;

	/// Save last confirmed height of outputs for a given parent
	fn save_last_confirmed_height(
		&mut self,
		parent_key_id: &Identifier,
		height: u64,
	) -> Result<(), Error>;

	/// get next tx log entry for the parent
	fn next_tx_log_id(&mut self, parent_key_id: &Identifier) -> Result<u32, Error>;

	/// Iterate over tx log data stored by the backend
	fn tx_log_iter(&self) -> Box<dyn Iterator<Item = TxLogEntry>>;

	/// save a tx log entry
	fn save_tx_log_entry(&mut self, t: TxLogEntry, parent_id: &Identifier) -> Result<(), Error>;

	/// save an account label -> path mapping
	fn save_acct_path(&mut self, mapping: AcctPathMapping) -> Result<(), Error>;

	/// Iterate over account names stored in backend
	fn acct_path_iter(&self) -> Box<dyn Iterator<Item = AcctPathMapping>>;

	/// Save an output as locked in the backend
	fn lock_output(&mut self, out: &mut OutputData) -> Result<(), Error>;

	/// Saves the private context associated with a slate id
	fn save_private_context(&mut self, slate_id: &[u8], ctx: &Context) -> Result<(), Error>;

	/// Delete the private context associated with the slate id
	fn delete_private_context(&mut self, slate_id: &[u8]) -> Result<(), Error>;

	/// Write the wallet data to backend file
	fn commit(&self) -> Result<(), Error>;
}

/// Encapsulate all wallet-node communication functions. No functions within libwallet
/// should care about communication details
pub trait NodeClient: Sync + Send + Clone {
	/// Return the URL of the check node
	fn node_url(&self) -> &str;

	/// Set the node URL
	fn set_node_url(&mut self, node_url: &str);

	/// Return the node api secret
	fn node_api_secret(&self) -> Option<String>;

	/// Change the API secret
	fn set_node_api_secret(&mut self, node_api_secret: Option<String>);

	/// Posts a transaction to a grin node
	fn post_tx(&self, tx: &TxWrapper, fluff: bool) -> Result<(), Error>;

	/// retrieves the current tip from the specified grin node
	fn get_chain_height(&self) -> Result<u64, Error>;

	/// retrieve a list of outputs from the specified grin node
	/// need "by_height" and "by_id" variants
	fn get_outputs_from_node(
		&self,
		wallet_outputs: Vec<pedersen::Commitment>,
	) -> Result<HashMap<pedersen::Commitment, (String, u64)>, Error>;

	/// Get a list of outputs from the node by traversing the UTXO
	/// set in PMMR index order.
	/// Returns
	/// (last available output index, last insertion index retrieved,
	/// outputs(commit, proof, is_coinbase, height))
	fn get_outputs_by_pmmr_index(
		&self,
		start_height: u64,
		max_outputs: u64,
	) -> Result<
		(
			u64,
			u64,
			Vec<(pedersen::Commitment, pedersen::RangeProof, bool, u64)>,
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
	/// Optional corresponding internal entry in tx entry log
	pub tx_log_entry: Option<u32>,
}

impl ser::Writeable for OutputData {
	fn write<W: ser::Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		writer.write_bytes(&serde_json::to_vec(self).map_err(|_| ser::Error::CorruptedData)?)
	}
}

impl ser::Readable for OutputData {
	fn read(reader: &mut dyn ser::Reader) -> Result<OutputData, ser::Error> {
		let data = reader.read_bytes_len_prefix()?;
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
		if self.height > current_height {
			return 0;
		}
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
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match *self {
			OutputStatus::Unconfirmed => write!(f, "Unconfirmed"),
			OutputStatus::Unspent => write!(f, "Unspent"),
			OutputStatus::Locked => write!(f, "Locked"),
			OutputStatus::Spent => write!(f, "Spent"),
		}
	}
}

#[derive(Serialize, Deserialize, Clone, Debug)]
/// Holds the context for a single aggsig transaction
pub struct Context {
	/// Secret key (of which public is shared)
	pub sec_key: SecretKey,
	/// Secret nonce (of which public is shared)
	/// (basically a SecretKey)
	pub sec_nonce: SecretKey,
	/// store my outputs between invocations
	pub output_ids: Vec<Identifier>,
	/// store my inputs
	pub input_ids: Vec<Identifier>,
	/// store the calculated fee
	pub fee: u64,
}

impl Context {
	/// Create a new context with defaults
	pub fn new(secp: &secp::Secp256k1, sec_key: SecretKey) -> Context {
		Context {
			sec_key: sec_key,
			sec_nonce: aggsig::create_secnonce(secp).unwrap(),
			input_ids: vec![],
			output_ids: vec![],
			fee: 0,
		}
	}
}

impl Context {
	/// Tracks an output contributing to my excess value (if it needs to
	/// be kept between invocations
	pub fn add_output(&mut self, output_id: &Identifier) {
		self.output_ids.push(output_id.clone());
	}

	/// Returns all stored outputs
	pub fn get_outputs(&self) -> Vec<Identifier> {
		self.output_ids.clone()
	}

	/// Tracks IDs of my inputs into the transaction
	/// be kept between invocations
	pub fn add_input(&mut self, input_id: &Identifier) {
		self.input_ids.push(input_id.clone());
	}

	/// Returns all stored input identifiers
	pub fn get_inputs(&self) -> Vec<Identifier> {
		self.input_ids.clone()
	}

	/// Returns private key, private nonce
	pub fn get_private_keys(&self) -> (SecretKey, SecretKey) {
		(self.sec_key.clone(), self.sec_nonce.clone())
	}

	/// Returns public key, public nonce
	pub fn get_public_keys(&self, secp: &Secp256k1) -> (PublicKey, PublicKey) {
		(
			PublicKey::from_secret_key(secp, &self.sec_key).unwrap(),
			PublicKey::from_secret_key(secp, &self.sec_nonce).unwrap(),
		)
	}
}

impl ser::Writeable for Context {
	fn write<W: ser::Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		writer.write_bytes(&serde_json::to_vec(self).map_err(|_| ser::Error::CorruptedData)?)
	}
}

impl ser::Readable for Context {
	fn read(reader: &mut dyn ser::Reader) -> Result<Context, ser::Error> {
		let data = reader.read_bytes_len_prefix()?;
		serde_json::from_slice(&data[..]).map_err(|_| ser::Error::CorruptedData)
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
		let hash =
			Hash::from_hex(hex).context(ErrorKind::GenericError("Invalid hex".to_owned()))?;
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

	fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
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
#[derive(Serialize, Eq, PartialEq, Deserialize, Debug, Clone)]
pub struct WalletInfo {
	/// height from which info was taken
	pub last_confirmed_height: u64,
	/// Minimum number of confirmations for an output to be treated as "spendable".
	pub minimum_confirmations: u64,
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

/// Types of transactions that can be contained within a TXLog entry
#[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
pub enum TxLogEntryType {
	/// A coinbase transaction becomes confirmed
	ConfirmedCoinbase,
	/// Outputs created when a transaction is received
	TxReceived,
	/// Inputs locked + change outputs when a transaction is created
	TxSent,
	/// Received transaction that was rolled back by user
	TxReceivedCancelled,
	/// Sent transaction that was rolled back by user
	TxSentCancelled,
}

impl fmt::Display for TxLogEntryType {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match *self {
			TxLogEntryType::ConfirmedCoinbase => write!(f, "Confirmed \nCoinbase"),
			TxLogEntryType::TxReceived => write!(f, "Received Tx"),
			TxLogEntryType::TxSent => write!(f, "Sent Tx"),
			TxLogEntryType::TxReceivedCancelled => write!(f, "Received Tx\n- Cancelled"),
			TxLogEntryType::TxSentCancelled => write!(f, "Sent Tx\n- Cancelled"),
		}
	}
}

/// Optional transaction information, recorded when an event happens
/// to add or remove funds from a wallet. One Transaction log entry
/// maps to one or many outputs
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TxLogEntry {
	/// BIP32 account path used for creating this tx
	pub parent_key_id: Identifier,
	/// Local id for this transaction (distinct from a slate transaction id)
	pub id: u32,
	/// Slate transaction this entry is associated with, if any
	pub tx_slate_id: Option<Uuid>,
	/// Transaction type (as above)
	pub tx_type: TxLogEntryType,
	/// Time this tx entry was created
	/// #[serde(with = "tx_date_format")]
	pub creation_ts: DateTime<Utc>,
	/// Time this tx was confirmed (by this wallet)
	/// #[serde(default, with = "opt_tx_date_format")]
	pub confirmation_ts: Option<DateTime<Utc>>,
	/// Whether the inputs+outputs involved in this transaction have been
	/// confirmed (In all cases either all outputs involved in a tx should be
	/// confirmed, or none should be; otherwise there's a deeper problem)
	pub confirmed: bool,
	/// number of inputs involved in TX
	pub num_inputs: usize,
	/// number of outputs involved in TX
	pub num_outputs: usize,
	/// Amount credited via this transaction
	pub amount_credited: u64,
	/// Amount debited via this transaction
	pub amount_debited: u64,
	/// Fee
	pub fee: Option<u64>,
	/// Location of the store transaction, (reference or resending)
	pub stored_tx: Option<String>,
}

impl ser::Writeable for TxLogEntry {
	fn write<W: ser::Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		writer.write_bytes(&serde_json::to_vec(self).map_err(|_| ser::Error::CorruptedData)?)
	}
}

impl ser::Readable for TxLogEntry {
	fn read(reader: &mut dyn ser::Reader) -> Result<TxLogEntry, ser::Error> {
		let data = reader.read_bytes_len_prefix()?;
		serde_json::from_slice(&data[..]).map_err(|_| ser::Error::CorruptedData)
	}
}

impl TxLogEntry {
	/// Return a new blank with TS initialised with next entry
	pub fn new(parent_key_id: Identifier, t: TxLogEntryType, id: u32) -> Self {
		TxLogEntry {
			parent_key_id: parent_key_id,
			tx_type: t,
			id: id,
			tx_slate_id: None,
			creation_ts: Utc::now(),
			confirmation_ts: None,
			confirmed: false,
			amount_credited: 0,
			amount_debited: 0,
			num_inputs: 0,
			num_outputs: 0,
			fee: None,
			stored_tx: None,
		}
	}

	/// Given a vec of TX log entries, return credited + debited sums
	pub fn sum_confirmed(txs: &Vec<TxLogEntry>) -> (u64, u64) {
		txs.iter().fold((0, 0), |acc, tx| match tx.confirmed {
			true => (acc.0 + tx.amount_credited, acc.1 + tx.amount_debited),
			false => acc,
		})
	}

	/// Update confirmation TS with now
	pub fn update_confirmation_ts(&mut self) {
		self.confirmation_ts = Some(Utc::now());
	}
}

/// Map of named accounts to BIP32 paths
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AcctPathMapping {
	/// label used by user
	pub label: String,
	/// Corresponding parent BIP32 derivation path
	pub path: Identifier,
}

impl ser::Writeable for AcctPathMapping {
	fn write<W: ser::Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		writer.write_bytes(&serde_json::to_vec(self).map_err(|_| ser::Error::CorruptedData)?)
	}
}

impl ser::Readable for AcctPathMapping {
	fn read(reader: &mut dyn ser::Reader) -> Result<AcctPathMapping, ser::Error> {
		let data = reader.read_bytes_len_prefix()?;
		serde_json::from_slice(&data[..]).map_err(|_| ser::Error::CorruptedData)
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
	/// payment method
	pub method: String,
	/// destination url
	pub dest: String,
	/// Max number of outputs
	pub max_outputs: usize,
	/// Number of change outputs to generate
	pub num_change_outputs: usize,
	/// whether to use all outputs (combine)
	pub selection_strategy_is_use_all: bool,
	/// Optional message, that will be signed
	pub message: Option<String>,
}
