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

use std::marker::PhantomData;
use std::sync::{Arc, Mutex};

use core::ser;
use keychain::Keychain;
use libtx::slate::Slate;
use libwallet::internal::{tx, updater};
use libwallet::types::{
	BlockFees, CbData, OutputData, TxLogEntry, TxWrapper, WalletBackend, WalletClient, WalletInfo,
};
use libwallet::{Error, ErrorKind};
use util::{self, LOGGER};

/// Wrapper around internal API functions, containing a reference to
/// the wallet/keychain that they're acting upon
pub struct APIOwner<W: ?Sized, C, K>
where
	W: WalletBackend<C, K>,
	C: WalletClient,
	K: Keychain,
{
	/// Wallet, contains its keychain (TODO: Split these up into 2 traits
	/// perhaps)
	pub wallet: Arc<Mutex<Box<W>>>,
	phantom: PhantomData<K>,
	phantom_c: PhantomData<C>,
}

impl<W: ?Sized, C, K> APIOwner<W, C, K>
where
	W: WalletBackend<C, K>,
	C: WalletClient,
	K: Keychain,
{
	/// Create new API instance
	pub fn new(wallet_in: Arc<Mutex<Box<W>>>) -> Self {
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
	) -> Result<(bool, Vec<OutputData>), Error> {
		let mut w = self.wallet.lock().unwrap();
		w.open_with_credentials()?;

		let mut validated = false;
		if refresh_from_node {
			validated = self.update_outputs(&mut w);
		}

		let res = Ok((
			validated,
			updater::retrieve_outputs(&mut **w, include_spent, tx_id)?,
		));

		w.close()?;
		res
	}

	/// Attempt to update outputs and retrieve tranasactions
	/// Return (whether the outputs were validated against a node, OutputData)
	pub fn retrieve_txs(
		&self,
		refresh_from_node: bool,
		tx_id: Option<u32>,
	) -> Result<(bool, Vec<TxLogEntry>), Error> {
		let mut w = self.wallet.lock().unwrap();
		w.open_with_credentials()?;

		let mut validated = false;
		if refresh_from_node {
			validated = self.update_outputs(&mut w);
		}

		let res = Ok((validated, updater::retrieve_txs(&mut **w, tx_id)?));

		w.close()?;
		res
	}

	/// Retrieve summary info for wallet
	pub fn retrieve_summary_info(
		&mut self,
		refresh_from_node: bool,
	) -> Result<(bool, WalletInfo), Error> {
		let mut w = self.wallet.lock().unwrap();
		w.open_with_credentials()?;

		let mut validated = false;
		if refresh_from_node {
			validated = self.update_outputs(&mut w);
		}

		let wallet_info = updater::retrieve_info(&mut **w)?;
		let res = Ok((validated, wallet_info));

		w.close()?;
		res
	}

	/// Issues a send transaction and sends to recipient
	pub fn issue_send_tx(
		&mut self,
		amount: u64,
		minimum_confirmations: u64,
		dest: &str,
		max_outputs: usize,
		selection_strategy_is_use_all: bool,
	) -> Result<Slate, Error> {
		let mut w = self.wallet.lock().unwrap();
		w.open_with_credentials()?;

		let client;
		let mut slate_out: Slate;
		let lock_fn_out;

		client = w.client().clone();
		let (slate, context, lock_fn) = tx::create_send_tx(
			&mut **w,
			amount,
			minimum_confirmations,
			max_outputs,
			selection_strategy_is_use_all,
		)?;

		lock_fn_out = lock_fn;
		slate_out = match client.send_tx_slate(dest, &slate) {
			Ok(s) => s,
			Err(e) => {
				error!(
				LOGGER,
				"Communication with receiver failed on SenderInitiation send. Aborting transaction {:?}",
				e,
			);
				return Err(e)?;
			}
		};

		tx::complete_tx(&mut **w, &mut slate_out, &context)?;

		// lock our inputs
		lock_fn_out(&mut **w)?;
		w.close()?;
		Ok(slate_out)
	}

	/// Roll back a transaction and all associated outputs with a given
	/// transaction id This means delete all change outputs, (or recipient
	/// output if you're recipient), and unlock all locked outputs associated
	/// with the transaction used when a transaction is created but never
	/// posted
	pub fn cancel_tx(&mut self, tx_id: u32) -> Result<(), Error> {
		let mut w = self.wallet.lock().unwrap();
		w.open_with_credentials()?;
		if !self.update_outputs(&mut w) {
			return Err(ErrorKind::TransactionCancellationError(
				"Can't contact running Grin node. Not Cancelling.",
			))?;
		}
		tx::cancel_tx(&mut **w, tx_id)?;
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
		let mut w = self.wallet.lock().unwrap();
		w.open_with_credentials()?;
		let tx_burn = tx::issue_burn_tx(&mut **w, amount, minimum_confirmations, max_outputs)?;
		let tx_hex = util::to_hex(ser::ser_vec(&tx_burn).unwrap());
		w.client().post_tx(&TxWrapper { tx_hex: tx_hex }, false)?;
		w.close()?;
		Ok(())
	}

	/// Posts a transaction to the chain
	pub fn post_tx(&self, slate: &Slate, fluff: bool) -> Result<(), Error> {
		let tx_hex = util::to_hex(ser::ser_vec(&slate.tx).unwrap());
		let client = {
			let mut w = self.wallet.lock().unwrap();
			w.client().clone()
		};
		client.post_tx(&TxWrapper { tx_hex: tx_hex }, fluff)?;
		Ok(())
	}

	/// Attempt to restore contents of wallet
	pub fn restore(&mut self) -> Result<(), Error> {
		let mut w = self.wallet.lock().unwrap();
		w.open_with_credentials()?;
		let res = w.restore();
		w.close()?;
		res
	}

	/// Retrieve current height from node
	pub fn node_height(&mut self) -> Result<(u64, bool), Error> {
		let res = {
			let mut w = self.wallet.lock().unwrap();
			w.open_with_credentials()?;
			w.client().get_chain_height()
		};
		match res {
			Ok(height) => {
				let mut w = self.wallet.lock().unwrap();
				w.close()?;
				Ok((height, true))
			}
			Err(_) => {
				let outputs = self.retrieve_outputs(true, false, None)?;
				let height = match outputs.1.iter().map(|out| out.height).max() {
					Some(height) => height,
					None => 0,
				};
				let mut w = self.wallet.lock().unwrap();
				w.close()?;
				Ok((height, false))
			}
		}
	}

	/// Attempt to update outputs in wallet, return whether it was successful
	fn update_outputs(&self, w: &mut W) -> bool {
		match updater::refresh_outputs(&mut *w) {
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
	C: WalletClient,
	K: Keychain,
{
	/// Wallet, contains its keychain (TODO: Split these up into 2 traits
	/// perhaps)
	pub wallet: Arc<Mutex<Box<W>>>,
	phantom: PhantomData<K>,
	phantom_c: PhantomData<C>,
}

impl<'a, W: ?Sized, C, K> APIForeign<W, C, K>
where
	W: WalletBackend<C, K>,
	C: WalletClient,
	K: Keychain,
{
	/// Create new API instance
	pub fn new(wallet_in: Arc<Mutex<Box<W>>>) -> Box<Self> {
		Box::new(APIForeign {
			wallet: wallet_in,
			phantom: PhantomData,
			phantom_c: PhantomData,
		})
	}

	/// Build a new (potential) coinbase transaction in the wallet
	pub fn build_coinbase(&mut self, block_fees: &BlockFees) -> Result<CbData, Error> {
		let mut w = self.wallet.lock().unwrap();
		w.open_with_credentials()?;
		let res = updater::build_coinbase(&mut **w, block_fees);
		w.close()?;
		res
	}

	/// Receive a transaction from a sender
	pub fn receive_tx(&mut self, slate: &mut Slate) -> Result<(), Error> {
		let mut w = self.wallet.lock().unwrap();
		w.open_with_credentials()?;
		let res = tx::receive_tx(&mut **w, slate);
		w.close()?;
		res
	}
}
