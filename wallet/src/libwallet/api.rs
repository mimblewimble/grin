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

//! Wrappers around library functions, intended to split functions
//! into external and internal APIs (i.e. functions for the local wallet
//! vs. functions to interact with someone else)
//! Still experimental, not sure this is the best way to do this

use std::fs::File;
use std::io::Write;
use std::marker::PhantomData;
use std::sync::Arc;
use util::Mutex;
use uuid::Uuid;

use serde_json as json;

use core::core::hash::Hashed;
use core::core::Transaction;
use core::ser;
use keychain::{Identifier, Keychain};
use libtx::slate::Slate;
use libwallet::internal::{keys, tx, updater};
use libwallet::types::{
	AcctPathMapping, BlockFees, CbData, NodeClient, OutputData, TxLogEntry, TxWrapper,
	WalletBackend, WalletInfo,
};
use libwallet::{Error, ErrorKind};
use util;
use util::secp::pedersen;

/// Wrapper around internal API functions, containing a reference to
/// the wallet/keychain that they're acting upon
pub struct APIOwner<W: ?Sized, C, K>
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

impl<W: ?Sized, C, K> APIOwner<W, C, K>
where
	W: WalletBackend<C, K>,
	C: NodeClient,
	K: Keychain,
{
	/// Create new API instance
	pub fn new(wallet_in: Arc<Mutex<W>>) -> Self {
		APIOwner {
			wallet: wallet_in,
			phantom: PhantomData,
			phantom_c: PhantomData,
		}
	}

	/// Attempt to update and retrieve outputs
	/// Return (whether the outputs were validated against a node, OutputData)
	/// if tx_id is some then only retrieve outputs for associated transaction
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

	/// Attempt to update outputs and retrieve transactions
	/// Return (whether the outputs were validated against a node, OutputData)
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

	/// Retrieve summary info for wallet
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

	/// Return list of existing account -> Path mappings
	pub fn accounts(&mut self) -> Result<Vec<AcctPathMapping>, Error> {
		let mut w = self.wallet.lock();
		keys::accounts(&mut *w)
	}

	/// Create a new account path
	pub fn new_account_path(&mut self, label: &str) -> Result<Identifier, Error> {
		let mut w = self.wallet.lock();
		keys::new_acct_path(&mut *w, label)
	}

	/// Creates a new partial transaction for the given amount
	pub fn initiate_tx(
		&mut self,
		src_acct_name: Option<&str>,
		amount: u64,
		minimum_confirmations: u64,
		max_outputs: usize,
		num_change_outputs: usize,
		selection_strategy_is_use_all: bool,
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

	/// Issue a burn TX
	pub fn issue_burn_tx(
		&mut self,
		amount: u64,
		minimum_confirmations: u64,
		max_outputs: usize,
	) -> Result<(), Error> {
		let mut w = self.wallet.lock();
		w.open_with_credentials()?;
		let parent_key_id = w.parent_key_id();
		let tx_burn = tx::issue_burn_tx(
			&mut *w,
			amount,
			minimum_confirmations,
			max_outputs,
			&parent_key_id,
		)?;
		let tx_hex = util::to_hex(ser::ser_vec(&tx_burn).unwrap());
		w.w2n_client()
			.post_tx(&TxWrapper { tx_hex: tx_hex }, false)?;
		w.close()?;
		Ok(())
	}

	/// Posts a transaction to the chain
	pub fn post_tx(&self, slate: &Slate, fluff: bool) -> Result<(), Error> {
		let tx_hex = util::to_hex(ser::ser_vec(&slate.tx).unwrap());
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
				slate.tx.hash(),
				fluff
			);
			Ok(())
		}
	}

	/// Writes stored transaction data to a given file
	pub fn dump_stored_tx(
		&self,
		tx_id: u32,
		write_to_disk: bool,
		dest: &str,
	) -> Result<Transaction, Error> {
		let (confirmed, tx_hex) = {
			let mut w = self.wallet.lock();
			w.open_with_credentials()?;
			let parent_key_id = w.parent_key_id();
			let res = tx::retrieve_tx_hex(&mut *w, &parent_key_id, tx_id)?;
			w.close()?;
			res
		};
		if confirmed {
			warn!(
				"api: dump_stored_tx: transaction at {} is already confirmed.",
				tx_id
			);
		}
		if tx_hex.is_none() {
			error!(
				"api: dump_stored_tx: completed transaction at {} does not exist.",
				tx_id
			);
			return Err(ErrorKind::TransactionBuildingNotCompleted(tx_id))?;
		}
		let tx_bin = util::from_hex(tx_hex.unwrap()).unwrap();
		let tx = ser::deserialize::<Transaction>(&mut &tx_bin[..])?;
		if write_to_disk {
			let mut tx_file = File::create(dest)?;
			tx_file.write_all(json::to_string(&tx).unwrap().as_bytes())?;
			tx_file.sync_all()?;
		}
		Ok(tx)
	}

	/// (Re)Posts a transaction that's already been stored to the chain
	pub fn post_stored_tx(&self, tx_id: u32, fluff: bool) -> Result<(), Error> {
		let client;
		let (confirmed, tx_hex) = {
			let mut w = self.wallet.lock();
			w.open_with_credentials()?;
			let parent_key_id = w.parent_key_id();
			client = w.w2n_client().clone();
			let res = tx::retrieve_tx_hex(&mut *w, &parent_key_id, tx_id)?;
			w.close()?;
			res
		};
		if confirmed {
			error!(
				"api: repost_tx: transaction at {} is confirmed. NOT resending.",
				tx_id
			);
			return Err(ErrorKind::TransactionAlreadyConfirmed)?;
		}
		if tx_hex.is_none() {
			error!(
				"api: repost_tx: completed transaction at {} does not exist.",
				tx_id
			);
			return Err(ErrorKind::TransactionBuildingNotCompleted(tx_id))?;
		}
		client.post_tx(
			&TxWrapper {
				tx_hex: tx_hex.unwrap(),
			},
			fluff,
		)?;
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
		let res = tx::receive_tx(&mut *w, slate, &parent_key_id, false);
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
