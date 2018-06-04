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

use libwallet::types::WalletBackend;

/// Wrapper around internal API functions, containing a reference to
/// the wallet/keychain that they're acting upon
pub struct APIInternal<'a, W>
where
	W: 'a + WalletBackend,
{
	/// Wallet, contains its keychain (TODO: Split these up into 2 traits
	/// perhaps)
	pub wallet: &'a W,
}

impl<'a, W> APIInternal<'a, W>
where
	W: 'a + WalletBackend,
{
	/// Create new API instance
	pub fn new(wallet_in: &'a mut W) -> APIInternal<'a, W> {
		APIInternal { wallet: wallet_in }
	}
}
