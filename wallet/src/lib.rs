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

//! Library module for the main wallet functionalities provided by Grin.

use blake2_rfc as blake2;

#[macro_use]
extern crate prettytable;

use serde;
#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate log;

use failure;
use grin_api as api;
use grin_core as core;
use grin_keychain as keychain;
use grin_store as store;
use grin_util as util;
use term;

mod adapters;
pub mod command;
pub mod command_args;
pub mod controller;
pub mod display;
mod error;
pub mod libwallet;
pub mod lmdb_wallet;
mod node_clients;
mod types;

pub use crate::adapters::{
	FileWalletCommAdapter, HTTPWalletCommAdapter, KeybaseWalletCommAdapter, NullWalletCommAdapter,
	WalletCommAdapter,
};
pub use crate::error::{Error, ErrorKind};
pub use crate::libwallet::types::{
	BlockFees, CbData, NodeClient, WalletBackend, WalletInfo, WalletInst,
};
pub use crate::lmdb_wallet::{wallet_db_exists, LMDBBackend};
pub use crate::node_clients::{create_coinbase, HTTPNodeClient};
pub use crate::types::{EncryptedWalletSeed, WalletConfig, WalletSeed, SEED_FILE};

use crate::util::Mutex;
use std::sync::Arc;

/// Helper to create an instance of the LMDB wallet
pub fn instantiate_wallet(
	wallet_config: WalletConfig,
	passphrase: &str,
	account: &str,
	node_api_secret: Option<String>,
) -> Result<Arc<Mutex<dyn WalletInst<HTTPNodeClient, keychain::ExtKeychain>>>, Error> {
	// First test decryption, so we can abort early if we have the wrong password
	let _ = WalletSeed::from_file(&wallet_config, passphrase)?;
	let client_n = HTTPNodeClient::new(&wallet_config.check_node_api_http_addr, node_api_secret);
	let mut db_wallet = LMDBBackend::new(wallet_config.clone(), passphrase, client_n)?;
	db_wallet.set_parent_key_id_by_name(account)?;
	info!("Using LMDB Backend for wallet");
	Ok(Arc::new(Mutex::new(db_wallet)))
}
