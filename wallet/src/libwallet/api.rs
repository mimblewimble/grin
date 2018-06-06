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

use libtx::slate::Slate;
use libwallet::Error;
use libwallet::internal::{tx, updater};
use libwallet::types::{BlockFees, CbData, OutputData, WalletBackend, WalletInfo};

/// Wrapper around internal API functions, containing a reference to
/// the wallet/keychain that they're acting upon
pub struct APIOwner<'a, W>
where
	W: 'a + WalletBackend,
{
	/// Wallet, contains its keychain (TODO: Split these up into 2 traits
	/// perhaps)
	pub wallet: &'a mut W,
}

impl<'a, W> APIOwner<'a, W>
where
	W: 'a + WalletBackend,
{
	/// Create new API instance
	pub fn new(wallet_in: &'a mut W) -> APIOwner<'a, W> {
		APIOwner { wallet: wallet_in }
	}

	/// Attempt to update and retrieve outputs
	/// Return (whether the outputs were validated against a node, OutputData)
	pub fn retrieve_outputs(
		&mut self,
		include_spent: bool,
	) -> Result<(bool, Vec<OutputData>), Error> {
		let validated = self.update_outputs();
		Ok((
			validated,
			updater::retrieve_outputs(self.wallet, include_spent)?,
		))
	}

	/// Retrieve summary info for wallet
	pub fn retrieve_summary_info(&mut self) -> Result<(bool, WalletInfo), Error> {
		let validated = self.update_outputs();
		Ok((validated, updater::retrieve_info(self.wallet)?))
	}

	/// Issues a send transaction and sends to recipient
	/// (TODO: Split into separate functions, create tx, send, complete tx)
	pub fn issue_send_tx(
		&mut self,
		amount: u64,
		minimum_confirmations: u64,
		dest: &str,
		max_outputs: usize,
		selection_strategy_is_use_all: bool,
		fluff: bool,
	) -> Result<(), Error> {
		tx::issue_send_tx(
			self.wallet,
			amount,
			minimum_confirmations,
			dest,
			max_outputs,
			selection_strategy_is_use_all,
			fluff,
		)
	}

	/// Issue a burn TX
	pub fn issue_burn_tx(
		&mut self,
		amount: u64,
		minimum_confirmations: u64,
		max_outputs: usize,
	) -> Result<(), Error> {
		tx::issue_burn_tx(self.wallet, amount, minimum_confirmations, max_outputs)
	}

	/// Attempt to restore contents of wallet
	pub fn restore(&mut self) -> Result<(), Error> {
		self.wallet.restore()
	}

	/// Retrieve current height from node
	pub fn node_height(&mut self) -> Result<(u64, bool), Error> {
		match updater::get_tip_from_node(self.wallet.node_url()) {
			Ok(tip) => Ok((tip.height, true)),
			Err(_) => {
				let outputs = self.retrieve_outputs(true)?;
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
pub struct APIForeign<'a, W>
where
	W: 'a + WalletBackend,
{
	/// Wallet, contains its keychain (TODO: Split these up into 2 traits
	/// perhaps)
	pub wallet: &'a mut W,
}

impl<'a, W> APIForeign<'a, W>
where
	W: 'a + WalletBackend,
{
	/// Create new API instance
	pub fn new(wallet_in: &'a mut W) -> APIForeign<'a, W> {
		APIForeign { wallet: wallet_in }
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
