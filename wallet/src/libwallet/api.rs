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

use core::ser;
use keychain::Keychain;
use libtx::slate::Slate;
use libwallet::Error;
use libwallet::internal::{tx, updater};
use libwallet::types::{BlockFees, CbData, OutputData, TxWrapper, WalletBackend, WalletClient,
                       WalletInfo};
use util::{self, LOGGER};

/// Wrapper around internal API functions, containing a reference to
/// the wallet/keychain that they're acting upon
pub struct APIOwner<'a, W, K>
where
	W: 'a + WalletBackend<K> + WalletClient,
	K: Keychain,
{
	/// Wallet, contains its keychain (TODO: Split these up into 2 traits
	/// perhaps)
	pub wallet: &'a mut W,
	phantom: PhantomData<K>,
}

impl<'a, W, K> APIOwner<'a, W, K>
where
	W: 'a + WalletBackend<K> + WalletClient,
	K: Keychain,
{
	/// Create new API instance
	pub fn new(wallet_in: &'a mut W) -> APIOwner<'a, W, K> {
		APIOwner {
			wallet: wallet_in,
			phantom: PhantomData,
		}
	}

	/// Attempt to update and retrieve outputs
	/// Return (whether the outputs were validated against a node, OutputData)
	pub fn retrieve_outputs(
		&mut self,
		include_spent: bool,
		refresh_from_node: bool,
	) -> Result<(bool, Vec<OutputData>), Error> {
		let mut validated = false;
		if refresh_from_node {
			validated = self.update_outputs();
		}
		Ok((
			validated,
			updater::retrieve_outputs(self.wallet, include_spent)?,
		))
	}

	/// Retrieve summary info for wallet
	pub fn retrieve_summary_info(
		&mut self,
		refresh_from_node: bool,
	) -> Result<(bool, WalletInfo), Error> {
		let mut validated = false;
		if refresh_from_node {
			validated = self.update_outputs();
		}
		let wallet_info = updater::retrieve_info(self.wallet)?;
		Ok((validated, wallet_info))
	}

	/// Issues a send transaction and sends to recipient
	pub fn issue_send_tx(
		&mut self,
		amount: u64,
		minimum_confirmations: u64,
		dest: &str,
		max_outputs: usize,
		selection_strategy_is_use_all: bool,
		fluff: bool,
	) -> Result<(), Error> {
		let (slate, context, lock_fn) = tx::create_send_tx(
			self.wallet,
			amount,
			minimum_confirmations,
			max_outputs,
			selection_strategy_is_use_all,
		)?;

		let mut slate = match self.wallet.send_tx_slate(dest, &slate) {
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

		tx::complete_tx(self.wallet, &mut slate, &context)?;

		// All good here, so let's post it
		let tx_hex = util::to_hex(ser::ser_vec(&slate.tx).unwrap());
		self.wallet.post_tx(&TxWrapper { tx_hex: tx_hex }, fluff)?;

		// All good here, lock our inputs
		lock_fn(self.wallet)?;
		Ok(())
	}

	/// Issue a burn TX
	pub fn issue_burn_tx(
		&mut self,
		amount: u64,
		minimum_confirmations: u64,
		max_outputs: usize,
	) -> Result<(), Error> {
		let tx_burn = tx::issue_burn_tx(self.wallet, amount, minimum_confirmations, max_outputs)?;
		let tx_hex = util::to_hex(ser::ser_vec(&tx_burn).unwrap());
		self.wallet.post_tx(&TxWrapper { tx_hex: tx_hex }, false)?;
		Ok(())
	}

	/// Attempt to restore contents of wallet
	pub fn restore(&mut self) -> Result<(), Error> {
		self.wallet.restore()
	}

	/// Retrieve current height from node
	pub fn node_height(&mut self) -> Result<(u64, bool), Error> {
		match self.wallet.get_chain_height() {
			Ok(height) => Ok((height, true)),
			Err(_) => {
				let outputs = self.retrieve_outputs(true, false)?;
				let height = match outputs.1.iter().map(|out| out.height).max() {
					Some(height) => height,
					None => 0,
				};
				Ok((height, false))
			}
		}
	}

	/// Attempt to update outputs in wallet, return whether it was successful
	fn update_outputs(&mut self) -> bool {
		match updater::refresh_outputs(self.wallet) {
			Ok(_) => true,
			Err(_) => false,
		}
	}
}

/// Wrapper around external API functions, intended to communicate
/// with other parties
pub struct APIForeign<'a, W, K>
where
	W: 'a + WalletBackend<K> + WalletClient,
	K: Keychain,
{
	/// Wallet, contains its keychain (TODO: Split these up into 2 traits
	/// perhaps)
	pub wallet: &'a mut W,
	phantom: PhantomData<K>,
}

impl<'a, W, K> APIForeign<'a, W, K>
where
	W: 'a + WalletBackend<K> + WalletClient,
	K: Keychain,
{
	/// Create new API instance
	pub fn new(wallet_in: &'a mut W) -> APIForeign<'a, W, K> {
		APIForeign {
			wallet: wallet_in,
			phantom: PhantomData,
		}
	}

	/// Build a new (potential) coinbase transaction in the wallet
	pub fn build_coinbase(&mut self, block_fees: &BlockFees) -> Result<CbData, Error> {
		updater::build_coinbase(self.wallet, block_fees)
	}

	/// Receive a transaction from a sender
	pub fn receive_tx(&mut self, slate: &mut Slate) -> Result<(), Error> {
		tx::receive_tx(self.wallet, slate)
	}
}
