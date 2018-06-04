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

use libwallet::Error;
use libwallet::types::{OutputData, WalletBackend, WalletInfo};
use libwallet::updater;

/// Wrapper around internal API functions, containing a reference to
/// the wallet/keychain that they're acting upon
pub struct APIInternal<'a, W>
where
	W: 'a + WalletBackend,
{
	/// Wallet, contains its keychain (TODO: Split these up into 2 traits
	/// perhaps)
	pub wallet: &'a mut W,
}

impl<'a, W> APIInternal<'a, W>
where
	W: 'a + WalletBackend,
{
	/// Create new API instance
	pub fn new(wallet_in: &'a mut W) -> APIInternal<'a, W> {
		APIInternal { wallet: wallet_in }
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

	/// Attempt to update outputs in wallet, return whether it was successful
	fn update_outputs(&mut self) -> bool {
		match updater::refresh_outputs(self.wallet) {
			Ok(_) => true,
			Err(_) => false,
		}
	}
}
