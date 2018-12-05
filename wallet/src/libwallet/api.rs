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

//! Main interface into all wallet API functions.
//! Wallet APIs are split into two seperate blocks of functionality
//! called the 'Owner' and 'Foreign' APIs:
//! * The 'Owner' API is intended to expose methods that are to be
//! used by the wallet owner only. It is vital that this API is not
//! exposed to anyone other than the owner of the wallet (i.e. the
//! person with access to the seed and password.
//! * The 'Foreign' API contains methods that other wallets will
//! use to interact with the owner's wallet. This API can be exposed
//! to the outside world, with the consideration as to how that can
//! be done securely up to the implementor.
//!
//! Methods in both APIs are intended to be 'single use', that is to say each
//! method will 'open' the wallet (load the keychain with its master seed), perform
//! its operation, then 'close' the wallet (unloading references to the keychain and master
//! seed).

use std::marker::PhantomData;
use std::sync::Arc;
use util::Mutex;
use uuid::Uuid;

use core::core::hash::Hashed;
use core::core::Transaction;
use core::ser;
use keychain::{Identifier, Keychain};
use core::libtx::slate::Slate;
use libwallet::internal::{keys, tx, updater};
use libwallet::types::{
	AcctPathMapping, BlockFees, CbData, NodeClient, OutputData, TxLogEntry, TxWrapper,
	WalletBackend, WalletInfo,
};
use libwallet::{Error, ErrorKind};
use util;
use util::secp::pedersen;

/// Functions intended for use by the owner (e.g. master seed holder) of the wallet.
pub struct APIOwner<W: ?Sized, C, K>
where
	W: WalletBackend<C, K>,
	C: NodeClient,
	K: Keychain,
{
	/// A reference-counted mutex to an implementation of the
	/// [`WalletBackend`](../types/trait.WalletBackend.html) trait.
	pub wallet: Arc<Mutex<W>>,
	phantom: PhantomData<K>,
	phantom_c: PhantomData<C>,
}

impl<W: ?Sized, C, K> APIOwner<W, C, K>
where
	W: WalletBackend<C, K>,
	C: NodeClient,
	K: Keychain,
{
	/// Create a new API instance with the given wallet instance. All subsequent
	/// API calls will operate on this instance of the wallet.
	///
	/// Each method will call the [`WalletBackend`](../types/trait.WalletBackend.html)'s
	/// [`open_with_credentials`](../types/trait.WalletBackend.html#tymethod.open_with_credentials)
	/// (initialising a keychain with the master seed,) perform its operation, then close the keychain
	/// with a call to [`close`](../types/trait.WalletBackend.html#tymethod.close)
	///
	/// # Arguments
	/// * `wallet_in` - A reference-counted mutex containing an implementation of the
	/// [`WalletBackend`](../types/trait.WalletBackend.html) trait.
	///
	/// # Returns
	/// * An instance of the OwnerAPI holding a reference to the provided wallet
	///
	/// # Example
	/// ```
	/// # extern crate grin_wallet as wallet;
	/// # extern crate grin_keychain as keychain;
	/// # extern crate grin_util as util;
	///
	/// use std::sync::Arc;
	/// use util::Mutex;
	///
	/// use keychain::ExtKeychain;
	/// use wallet::libwallet::api::APIOwner;
	///
	/// // These contain sample implementations of each part needed for a wallet
	/// use wallet::{LMDBBackend, HTTPNodeClient, WalletBackend,  WalletConfig};
	///
	/// let mut wallet_config = WalletConfig::default();
	///
	/// // A NodeClient must first be created to handle communication between
	/// // the wallet and the node.
	///
	/// let node_client = HTTPNodeClient::new(&wallet_config.check_node_api_http_addr, None);
	/// let mut wallet:Arc<Mutex<WalletBackend<HTTPNodeClient, ExtKeychain>>> =
	///		Arc::new(Mutex::new(
	///			LMDBBackend::new(wallet_config.clone(), "", node_client).unwrap()
	///		));
	///
	/// let api_owner = APIOwner::new(wallet.clone());
	/// // .. perform wallet operations
	///
	/// ```

	pub fn new(wallet_in: Arc<Mutex<W>>) -> Self {
		APIOwner {
			wallet: wallet_in,
			phantom: PhantomData,
			phantom_c: PhantomData,
		}
	}

	/// Returns a list of accounts stored in the wallet (i.e. mappings between
	/// user-specified labels and BIP32 derivation paths.
	///
	/// # Returns
	/// * Result Containing:
	/// * A Vector of [`AcctPathMapping`](../types/struct.AcctPathMapping.html) data
	/// * or [`libwallet::Error`](../struct.Error.html) if an error is encountered.
	///
	/// # Remarks
	///
	/// * A wallet should always have the path with the label 'default' path defined,
	/// with path m/0/0
	/// * This method does not need to use the wallet seed or keychain.
	///
	/// # Example
	/// Set up as in [`new`](struct.APIOwner.html#method.new) method above.
	/// ```
	/// # extern crate grin_wallet as wallet;
	/// # extern crate grin_keychain as keychain;
	/// # extern crate grin_util as util;
	/// # use std::sync::Arc;
	/// # use util::Mutex;
	/// # use keychain::ExtKeychain;
	/// # use wallet::libwallet::api::APIOwner;
	/// # use wallet::{LMDBBackend, HTTPNodeClient, WalletBackend,  WalletConfig};
	/// # let mut wallet_config = WalletConfig::default();
	/// # let node_client = HTTPNodeClient::new(&wallet_config.check_node_api_http_addr, None);
	/// # let mut wallet:Arc<Mutex<WalletBackend<HTTPNodeClient, ExtKeychain>>> =
	/// # Arc::new(Mutex::new(
	/// # 	LMDBBackend::new(wallet_config.clone(), "", node_client).unwrap()
	/// # ));
	///
	/// let api_owner = APIOwner::new(wallet.clone());
	///
	/// let result = api_owner.accounts();
	///
	/// if let Ok(accts) = result {
	///		//...
	/// }
	/// ```

	pub fn accounts(&self) -> Result<Vec<AcctPathMapping>, Error> {
		let mut w = self.wallet.lock();
		keys::accounts(&mut *w)
	}

	/// Creates a new 'account', which is a mapping of a user-specified
	/// label to a BIP32 path
	///
	/// # Arguments
	/// * `label` - A human readable label to which to map the new BIP32 Path
	///
	/// # Returns
	/// * Result Containing:
	/// * A [Keychain Identifier](#) for the new path
	/// * or [`libwallet::Error`](../struct.Error.html) if an error is encountered.
	///
	/// # Remarks
	///
	/// * Wallets should be initialised with the 'default' path mapped to `m/0/0`
	/// * Each call to this function will increment the first element of the path
	/// so the first call will create an account at `m/1/0` and the second at
	/// `m/2/0` etc. . .
	/// * The account path is used throughout as the parent key for most key-derivation
	/// operations. See [`set_active_account`](struct.APIOwner.html#method.set_active_account) for
	/// further details.
	///
	/// * This function does not need to use the root wallet seed or keychain.
	///
	/// # Example
	/// Set up as in [`new`](struct.APIOwner.html#method.new) method above.
	/// ```
	/// # extern crate grin_wallet as wallet;
	/// # extern crate grin_keychain as keychain;
	/// # extern crate grin_util as util;
	/// # use std::sync::Arc;
	/// # use util::Mutex;
	/// # use keychain::ExtKeychain;
	/// # use wallet::libwallet::api::APIOwner;
	/// # use wallet::{LMDBBackend, HTTPNodeClient, WalletBackend,  WalletConfig};
	/// # let mut wallet_config = WalletConfig::default();
	/// # let node_client = HTTPNodeClient::new(&wallet_config.check_node_api_http_addr, None);
	/// # let mut wallet:Arc<Mutex<WalletBackend<HTTPNodeClient, ExtKeychain>>> =
	/// # Arc::new(Mutex::new(
	/// # 	LMDBBackend::new(wallet_config.clone(), "", node_client).unwrap()
	/// # ));
	///
	/// let api_owner = APIOwner::new(wallet.clone());
	///
	/// let result = api_owner.create_account_path("account1");
	///
	/// if let Ok(identifier) = result {
	///		//...
	/// }
	/// ```

	pub fn create_account_path(&self, label: &str) -> Result<Identifier, Error> {
		let mut w = self.wallet.lock();
		keys::new_acct_path(&mut *w, label)
	}

	/// Sets the wallet's currently active account. This sets the
	/// BIP32 parent path used for most key-derivation operations.
	///
	/// # Arguments
	/// * `label` - The human readable label for the account. Accounts can be retrieved via
	/// the [`account`](struct.APIOwner.html#method.accounts) method
	///
	/// # Returns
	/// * Result Containing:
	/// * `Ok(())` if the path was correctly set
	/// * or [`libwallet::Error`](../struct.Error.html) if an error is encountered.
	///
	/// # Remarks
	///
	/// * Wallet parent paths are 2 path elements long, e.g. `m/0/0` is the path
	/// labelled 'default'. Keys derived from this parent path are 3 elements long,
	/// e.g. the secret keys derived from the `m/0/0` path will be  at paths `m/0/0/0`,
	/// `m/0/0/1` etc...
	///
	/// * This function does not need to use the root wallet seed or keychain.
	///
	/// # Example
	/// Set up as in [`new`](struct.APIOwner.html#method.new) method above.
	/// ```
	/// # extern crate grin_wallet as wallet;
	/// # extern crate grin_keychain as keychain;
	/// # extern crate grin_util as util;
	/// # use std::sync::Arc;
	/// # use util::Mutex;
	/// # use keychain::ExtKeychain;
	/// # use wallet::libwallet::api::APIOwner;
	/// # use wallet::{LMDBBackend, HTTPNodeClient, WalletBackend,  WalletConfig};
	/// # let mut wallet_config = WalletConfig::default();
	/// # let node_client = HTTPNodeClient::new(&wallet_config.check_node_api_http_addr, None);
	/// # let mut wallet:Arc<Mutex<WalletBackend<HTTPNodeClient, ExtKeychain>>> =
	/// # Arc::new(Mutex::new(
	/// # 	LMDBBackend::new(wallet_config.clone(), "", node_client).unwrap()
	/// # ));
	///
	/// let api_owner = APIOwner::new(wallet.clone());
	///
	/// let result = api_owner.create_account_path("account1");
	///
	/// if let Ok(identifier) = result {
	///		// set the account active
	///		let result2 = api_owner.set_active_account("account1");
	/// }
	/// ```

	pub fn set_active_account(&self, label: &str) -> Result<(), Error> {
		let mut w = self.wallet.lock();
		w.set_parent_key_id_by_name(label)?;
		Ok(())
	}

	/// Returns a list of outputs from the active account in the wallet.
	///
	/// # Arguments
	/// * `include_spent` - If `true`, outputs that have been marked as 'spent'
	/// in the wallet will be returned. If `false`, spent outputs will omitted
	/// from the results.
	/// * `refresh_from_node` - If true, the wallet will attempt to contact
	/// a node (via the [`NodeClient`](../types/trait.NodeClient.html)
	/// provided during wallet instantiation). If `false`, the results will
	/// contain output information that may be out-of-date (from the last time
	/// the wallet's output set was refreshed against the node).
	/// * `tx_id` - If `Some(i)`, only return the outputs associated with
	/// the transaction log entry of id `i`.
	///
	/// # Returns
	/// * (`bool`, `Vec<OutputData, Commitment>`) - A tuple:
	/// * The first `bool` element indicates whether the data was successfully
	/// refreshed from the node (note this may be false even if the `refresh_from_node`
	/// argument was set to `true`.
	/// * The second element contains the result set, of which each element is
	/// a mapping between the wallet's internal [OutputData](../types/struct.OutputData.html)
	/// and the Output commitment as identified in the chain's UTXO set
	///
	/// # Example
	/// Set up as in [`new`](struct.APIOwner.html#method.new) method above.
	/// ```
	/// # extern crate grin_wallet as wallet;
	/// # extern crate grin_keychain as keychain;
	/// # extern crate grin_util as util;
	/// # use std::sync::Arc;
	/// # use util::Mutex;
	/// # use keychain::ExtKeychain;
	/// # use wallet::libwallet::api::APIOwner;
	/// # use wallet::{LMDBBackend, HTTPNodeClient, WalletBackend,  WalletConfig};
	/// # let mut wallet_config = WalletConfig::default();
	/// # let node_client = HTTPNodeClient::new(&wallet_config.check_node_api_http_addr, None);
	/// # let mut wallet:Arc<Mutex<WalletBackend<HTTPNodeClient, ExtKeychain>>> =
	/// # Arc::new(Mutex::new(
	/// # 	LMDBBackend::new(wallet_config.clone(), "", node_client).unwrap()
	/// # ));
	///
	/// let api_owner = APIOwner::new(wallet.clone());
	/// let show_spent = false;
	/// let update_from_node = true;
	/// let tx_id = None;
	///
	/// let result = api_owner.retrieve_outputs(show_spent, update_from_node, tx_id);
	///
	/// if let Ok((was_updated, output_mapping)) = result {
	///		//...
	/// }
	/// ```

	pub fn retrieve_outputs(
		&self,
		include_spent: bool,
		refresh_from_node: bool,
		tx_id: Option<u32>,
	) -> Result<(bool, Vec<(OutputData, pedersen::Commitment)>), Error> {
		let mut w = self.wallet.lock();
		w.open_with_credentials()?;
		let parent_key_id = w.parent_key_id();

		let mut validated = false;
		if refresh_from_node {
			validated = self.update_outputs(&mut w);
		}

		let res = Ok((
			validated,
			updater::retrieve_outputs(&mut *w, include_spent, tx_id, &parent_key_id)?,
		));

		w.close()?;
		res
	}

	/// Returns a list of [Transaction Log Entries](../types/struct.TxLogEntry.html)
	/// from the active account in the wallet.
	///
	/// # Arguments
	/// * `refresh_from_node` - If true, the wallet will attempt to contact
	/// a node (via the [`NodeClient`](../types/trait.NodeClient.html)
	/// provided during wallet instantiation). If `false`, the results will
	/// contain transaction information that may be out-of-date (from the last time
	/// the wallet's output set was refreshed against the node).
	/// * `tx_id` - If `Some(i)`, only return the transactions associated with
	/// the transaction log entry of id `i`.
	/// * `tx_slate_id` - If `Some(uuid)`, only return transactions associated with
	/// the given [`Slate`](../../libtx/slate/struct.Slate.html) uuid.
	///
	/// # Returns
	/// * (`bool`, `Vec<[TxLogEntry](../types/struct.TxLogEntry.html)>`) - A tuple:
	/// * The first `bool` element indicates whether the data was successfully
	/// refreshed from the node (note this may be false even if the `refresh_from_node`
	/// argument was set to `true`.
	/// * The second element contains the set of retrieved
	/// [TxLogEntries](../types/struct/TxLogEntry.html)
	///
	/// # Example
	/// Set up as in [`new`](struct.APIOwner.html#method.new) method above.
	/// ```
	/// # extern crate grin_wallet as wallet;
	/// # extern crate grin_keychain as keychain;
	/// # extern crate grin_util as util;
	/// # use std::sync::Arc;
	/// # use util::Mutex;
	/// # use keychain::ExtKeychain;
	/// # use wallet::libwallet::api::APIOwner;
	/// # use wallet::{LMDBBackend, HTTPNodeClient, WalletBackend,  WalletConfig};
	/// # let mut wallet_config = WalletConfig::default();
	/// # let node_client = HTTPNodeClient::new(&wallet_config.check_node_api_http_addr, None);
	/// # let mut wallet:Arc<Mutex<WalletBackend<HTTPNodeClient, ExtKeychain>>> =
	/// # Arc::new(Mutex::new(
	/// # 	LMDBBackend::new(wallet_config.clone(), "", node_client).unwrap()
	/// # ));
	///
	/// let api_owner = APIOwner::new(wallet.clone());
	/// let update_from_node = true;
	/// let tx_id = None;
	/// let tx_slate_id = None;
	///
	/// // Return all TxLogEntries
	/// let result = api_owner.retrieve_txs(update_from_node, tx_id, tx_slate_id);
	///
	/// if let Ok((was_updated, tx_log_entries)) = result {
	///		//...
	/// }
	/// ```

	pub fn retrieve_txs(
		&self,
		refresh_from_node: bool,
		tx_id: Option<u32>,
		tx_slate_id: Option<Uuid>,
	) -> Result<(bool, Vec<TxLogEntry>), Error> {
		let mut w = self.wallet.lock();
		w.open_with_credentials()?;
		let parent_key_id = w.parent_key_id();

		let mut validated = false;
		if refresh_from_node {
			validated = self.update_outputs(&mut w);
		}

		let res = Ok((
			validated,
			updater::retrieve_txs(&mut *w, tx_id, tx_slate_id, &parent_key_id)?,
		));

		w.close()?;
		res
	}

	/// Returns summary information from the active account in the wallet.
	///
	/// # Arguments
	/// * `refresh_from_node` - If true, the wallet will attempt to contact
	/// a node (via the [`NodeClient`](../types/trait.NodeClient.html)
	/// provided during wallet instantiation). If `false`, the results will
	/// contain transaction information that may be out-of-date (from the last time
	/// the wallet's output set was refreshed against the node).
	/// * `minimum_confirmations` - The minimum number of confirmations an output
	/// should have before it's included in the 'amount_currently_spendable' total
	///
	/// # Returns
	/// * (`bool`, [`WalletInfo`](../types/struct.WalletInfo.html)) - A tuple:
	/// * The first `bool` element indicates whether the data was successfully
	/// refreshed from the node (note this may be false even if the `refresh_from_node`
	/// argument was set to `true`.
	/// * The second element contains the Summary [`WalletInfo`](../types/struct.WalletInfo.html)
	///
	/// # Example
	/// Set up as in [`new`](struct.APIOwner.html#method.new) method above.
	/// ```
	/// # extern crate grin_wallet as wallet;
	/// # extern crate grin_keychain as keychain;
	/// # extern crate grin_util as util;
	/// # use std::sync::Arc;
	/// # use util::Mutex;
	/// # use keychain::ExtKeychain;
	/// # use wallet::libwallet::api::APIOwner;
	/// # use wallet::{LMDBBackend, HTTPNodeClient, WalletBackend,  WalletConfig};
	/// # let mut wallet_config = WalletConfig::default();
	/// # let node_client = HTTPNodeClient::new(&wallet_config.check_node_api_http_addr, None);
	/// # let mut wallet:Arc<Mutex<WalletBackend<HTTPNodeClient, ExtKeychain>>> =
	/// # Arc::new(Mutex::new(
	/// # 	LMDBBackend::new(wallet_config.clone(), "", node_client).unwrap()
	/// # ));
	///
	/// let mut api_owner = APIOwner::new(wallet.clone());
	/// let update_from_node = true;
	/// let minimum_confirmations=10;
	///
	/// // Return summary info for active account
	/// let result = api_owner.retrieve_summary_info(update_from_node, minimum_confirmations);
	///
	/// if let Ok((was_updated, summary_info)) = result {
	///		//...
	/// }
	/// ```

	pub fn retrieve_summary_info(
		&mut self,
		refresh_from_node: bool,
		minimum_confirmations: u64,
	) -> Result<(bool, WalletInfo), Error> {
		let mut w = self.wallet.lock();
		w.open_with_credentials()?;
		let parent_key_id = w.parent_key_id();

		let mut validated = false;
		if refresh_from_node {
			validated = self.update_outputs(&mut w);
		}

		let wallet_info = updater::retrieve_info(&mut *w, &parent_key_id, minimum_confirmations)?;
		let res = Ok((validated, wallet_info));

		w.close()?;
		res
	}

	/// Initiates a new transaction as the sender, creating a new
	/// [`Slate`](../../libtx/slate/struct.Slate.html) object containing
	/// the sender's inputs, change outputs, and public signature data. This slate can
	/// then be sent to the recipient to continue the transaction via the
	/// [Foreign API's `receive_tx`](struct.APIForeign.html#method.receive_tx) method.
	///
	/// When a transaction is created, the wallet must also lock inputs (and create unconfirmed
	/// outputs) corresponding to the transaction created in the slate, so that the wallet doesn't
	/// attempt to re-spend outputs that are already included in a transaction before the transaction
	/// is confirmed. This method also returns a function that will perform that locking, and it is
	/// up to the caller to decide the best time to call the lock function
	/// (via the [`tx_lock_outputs`](struct.APIOwner.html#method.tx_lock_outputs) method).
	/// If the exchange method is intended to be synchronous (such as via a direct http call,)
	/// then the lock call can wait until the response is confirmed. If it is asynchronous, (such
	/// as via file transfer,) the lock call should happen immediately (before the file is sent
	/// to the recipient).
	///
	/// # Arguments
	/// * `src_acct_name` - The human readable account name from which to draw outputs
	/// for the transaction, overriding whatever the active account is as set via the
	/// [`set_active_account`](struct.APIOwner.html#method.set_active_account) method.
	/// If None, the transaction will use the active account.
	/// * `amount` - The amount to send, in nanogrins. (`1 G = 1_000_000_000nG`)
	/// * `minimum_confirmations` - The minimum number of confirmations an output
	/// should have in order to be included in the transaction.
	/// * `max_outputs` - By default, the wallet selects as many inputs as possible in a
	/// transaction, to reduce the Output set and the fees. The wallet will attempt to spend
	/// include up to `max_outputs` in a transaction, however if this is not enough to cover
	/// the whole amount, the wallet will include more outputs. This parameter should be considered
	/// a soft limit.
	/// * `num_change_outputs` - The target number of change outputs to create in the transaction.
	/// The actual number created will be `num_change_outputs` + whatever remainder is needed.
	/// * `selection_strategy_is_use_all` - If `true`, attempt to use up as many outputs as
	/// possible to create the transaction, up the 'soft limit' of `max_outputs`. This helps
	/// to reduce the size of the UTXO set and the amount of data stored in the wallet, and
	/// minimizes fees. This will generally result in many inputs and a large change output(s),
	/// usually much larger than the amount being sent. If `false`, the transaction will include
	/// as many outputs as are needed to meet the amount, (and no more) starting with the smallest
	/// value outputs.
	/// * `message` - An optional participant message to include alongside the sender's public
	/// ParticipantData within the slate. This message will include a signature created with the
	/// sender's private keys, and will be publically verifiable. Note this message is for
	/// the convenience of the participants during the exchange; it is not included in the final
	/// transaction sent to the chain. Validation of this message is optional.
	///
	/// # Returns
	/// * a result containing:
	/// * ([`Slate`](../../libtx/slate/struct.Slate.html), lock_function) - A tuple:
	/// * The transaction Slate, which can be forwarded to the recieving party by any means.
	/// * A lock function, which should be called when the caller deems it appropriate to lock
	/// the transaction outputs (i.e. there is relative certaintly that the slate will be
	/// transmitted to the receiving party). Must be called before calling
	/// [`finalize_tx`](struct.APIOwner.html#method.finalize_tx).
	/// * or [`libwallet::Error`](../struct.Error.html) if an error is encountered.
	///
	/// # Remarks
	///
	/// * This method requires an active connection to a node, and will fail with error if a node
	/// cannot be contacted to refresh output statuses.
	/// * This method will store a partially completed transaction in the wallet's transaction log,
	/// which will be updated on the corresponding call to [`finalize_tx`](struct.APIOwner.html#method.finalize_tx).
	///
	/// # Example
	/// Set up as in [new](struct.APIOwner.html#method.new) method above.
	/// ```
	/// # extern crate grin_wallet as wallet;
	/// # extern crate grin_keychain as keychain;
	/// # extern crate grin_util as util;
	/// # use std::sync::Arc;
	/// # use util::Mutex;
	/// # use keychain::ExtKeychain;
	/// # use wallet::libwallet::api::APIOwner;
	/// # use wallet::{LMDBBackend, HTTPNodeClient, WalletBackend,  WalletConfig};
	/// # let mut wallet_config = WalletConfig::default();
	/// # let node_client = HTTPNodeClient::new(&wallet_config.check_node_api_http_addr, None);
	/// # let mut wallet:Arc<Mutex<WalletBackend<HTTPNodeClient, ExtKeychain>>> =
	/// # Arc::new(Mutex::new(
	/// # 	LMDBBackend::new(wallet_config.clone(), "", node_client).unwrap()
	/// # ));
	///
	/// let mut api_owner = APIOwner::new(wallet.clone());
	/// let amount = 2_000_000_000;
	///
	/// // Attempt to create a transaction using the 'default' account
	/// let result = api_owner.initiate_tx(
	///		None,
	///		amount,     // amount
	///		10,         // minimum confirmations
	///		500,        // max outputs
	///		1,          // num change outputs
	///		true,       // select all outputs
	///		Some("Have some Grins. Love, Yeastplume".to_owned()),
	///	);
	///
	/// if let Ok((slate, lock_fn)) = result {
	///		// Send slate somehow
	///		// ...
	///		// Lock our outputs if we're happy the slate was (or is being) sent
	///		api_owner.tx_lock_outputs(&slate, lock_fn);
	/// }
	/// ```

	pub fn initiate_tx(
		&mut self,
		src_acct_name: Option<&str>,
		amount: u64,
		minimum_confirmations: u64,
		max_outputs: usize,
		num_change_outputs: usize,
		selection_strategy_is_use_all: bool,
		message: Option<String>,
	) -> Result<(Slate, impl FnOnce(&mut W, &str) -> Result<(), Error>), Error> {
		let mut w = self.wallet.lock();
		w.open_with_credentials()?;
		let parent_key_id = match src_acct_name {
			Some(d) => {
				let pm = w.get_acct_path(d.to_owned())?;
				match pm {
					Some(p) => p.path,
					None => w.parent_key_id(),
				}
			}
			None => w.parent_key_id(),
		};

		let (slate, context, lock_fn) = tx::create_send_tx(
			&mut *w,
			amount,
			minimum_confirmations,
			max_outputs,
			num_change_outputs,
			selection_strategy_is_use_all,
			&parent_key_id,
			false,
			message,
		)?;

		// Save the aggsig context in our DB for when we
		// recieve the transaction back
		{
			let mut batch = w.batch()?;
			batch.save_private_context(slate.id.as_bytes(), &context)?;
			batch.commit()?;
		}

		w.close()?;
		Ok((slate, lock_fn))
	}

	/// Lock outputs associated with a given slate/transaction
	pub fn tx_lock_outputs(
		&mut self,
		slate: &Slate,
		lock_fn: impl FnOnce(&mut W, &str) -> Result<(), Error>,
	) -> Result<(), Error> {
		let mut w = self.wallet.lock();
		w.open_with_credentials()?;
		let tx_hex = util::to_hex(ser::ser_vec(&slate.tx).unwrap());
		lock_fn(&mut *w, &tx_hex)?;
		Ok(())
	}

	/// Sender finalization of the transaction. Takes the file returned by the
	/// sender as well as the private file generate on the first send step.
	/// Builds the complete transaction and sends it to a grin node for
	/// propagation.
	pub fn finalize_tx(&mut self, slate: &mut Slate) -> Result<(), Error> {
		let mut w = self.wallet.lock();
		let parent_key_id = w.parent_key_id();
		w.open_with_credentials()?;
		let context = w.get_private_context(slate.id.as_bytes())?;
		tx::complete_tx(&mut *w, slate, &context)?;
		tx::update_tx_hex(&mut *w, &parent_key_id, slate)?;
		{
			let mut batch = w.batch()?;
			batch.delete_private_context(slate.id.as_bytes())?;
			batch.commit()?;
		}

		w.close()?;
		Ok(())
	}

	/// Roll back a transaction and all associated outputs with a given
	/// transaction id This means delete all change outputs, (or recipient
	/// output if you're recipient), and unlock all locked outputs associated
	/// with the transaction used when a transaction is created but never
	/// posted
	pub fn cancel_tx(
		&mut self,
		tx_id: Option<u32>,
		tx_slate_id: Option<Uuid>,
	) -> Result<(), Error> {
		let mut w = self.wallet.lock();
		w.open_with_credentials()?;
		let parent_key_id = w.parent_key_id();
		if !self.update_outputs(&mut w) {
			return Err(ErrorKind::TransactionCancellationError(
				"Can't contact running Grin node. Not Cancelling.",
			))?;
		}
		tx::cancel_tx(&mut *w, &parent_key_id, tx_id, tx_slate_id)?;
		w.close()?;
		Ok(())
	}

	/// Posts a transaction to the chain
	pub fn post_tx(&self, tx: &Transaction, fluff: bool) -> Result<(), Error> {
		let tx_hex = util::to_hex(ser::ser_vec(tx).unwrap());
		let client = {
			let mut w = self.wallet.lock();
			w.w2n_client().clone()
		};
		let res = client.post_tx(&TxWrapper { tx_hex: tx_hex }, fluff);
		if let Err(e) = res {
			error!("api: post_tx: failed with error: {}", e);
			Err(e)
		} else {
			debug!(
				"api: post_tx: successfully posted tx: {}, fluff? {}",
				tx.hash(),
				fluff
			);
			Ok(())
		}
	}

	/// Verifies all messages in the slate match their public keys
	pub fn verify_slate_messages(&mut self, slate: &Slate) -> Result<(), Error> {
		let mut w = self.wallet.lock();
		slate.verify_messages(w.keychain().secp())?;
		Ok(())
	}

	/// Attempt to restore contents of wallet
	pub fn restore(&mut self) -> Result<(), Error> {
		let mut w = self.wallet.lock();
		w.open_with_credentials()?;
		let res = w.restore();
		w.close()?;
		res
	}

	/// Retrieve current height from node
	pub fn node_height(&mut self) -> Result<(u64, bool), Error> {
		let res = {
			let mut w = self.wallet.lock();
			w.open_with_credentials()?;
			w.w2n_client().get_chain_height()
		};
		match res {
			Ok(height) => Ok((height, true)),
			Err(_) => {
				let outputs = self.retrieve_outputs(true, false, None)?;
				let height = match outputs.1.iter().map(|(out, _)| out.height).max() {
					Some(height) => height,
					None => 0,
				};
				Ok((height, false))
			}
		}
	}

	/// Attempt to update outputs in wallet, return whether it was successful
	fn update_outputs(&self, w: &mut W) -> bool {
		let parent_key_id = w.parent_key_id();
		match updater::refresh_outputs(&mut *w, &parent_key_id) {
			Ok(_) => true,
			Err(_) => false,
		}
	}
}

/// Wrapper around external API functions, intended to communicate
/// with other parties
pub struct APIForeign<W: ?Sized, C, K>
where
	W: WalletBackend<C, K>,
	C: NodeClient,
	K: Keychain,
{
	/// Wallet, contains its keychain (TODO: Split these up into 2 traits
	/// perhaps)
	pub wallet: Arc<Mutex<W>>,
	phantom: PhantomData<K>,
	phantom_c: PhantomData<C>,
}

impl<'a, W: ?Sized, C, K> APIForeign<W, C, K>
where
	W: WalletBackend<C, K>,
	C: NodeClient,
	K: Keychain,
{
	/// Create new API instance
	pub fn new(wallet_in: Arc<Mutex<W>>) -> Box<Self> {
		Box::new(APIForeign {
			wallet: wallet_in,
			phantom: PhantomData,
			phantom_c: PhantomData,
		})
	}

	/// Build a new (potential) coinbase transaction in the wallet
	pub fn build_coinbase(&mut self, block_fees: &BlockFees) -> Result<CbData, Error> {
		let mut w = self.wallet.lock();
		w.open_with_credentials()?;
		let res = updater::build_coinbase(&mut *w, block_fees);
		w.close()?;
		res
	}

	/// Receive a transaction from a sender
	pub fn receive_tx(
		&mut self,
		slate: &mut Slate,
		dest_acct_name: Option<&str>,
		message: Option<String>,
	) -> Result<(), Error> {
		let mut w = self.wallet.lock();
		w.open_with_credentials()?;
		let parent_key_id = match dest_acct_name {
			Some(d) => {
				let pm = w.get_acct_path(d.to_owned())?;
				match pm {
					Some(p) => p.path,
					None => w.parent_key_id(),
				}
			}
			None => w.parent_key_id(),
		};
		let res = tx::receive_tx(&mut *w, slate, &parent_key_id, false, message);
		w.close()?;

		if let Err(e) = res {
			error!("api: receive_tx: failed with error: {}", e);
			Err(e)
		} else {
			debug!(
				"api: receive_tx: successfully received tx: {}",
				slate.tx.hash()
			);
			Ok(())
		}
	}
}
