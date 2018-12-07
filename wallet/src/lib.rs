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

extern crate blake2_rfc as blake2;
extern crate byteorder;
#[macro_use]
extern crate prettytable;
extern crate rand;
extern crate serde;
#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate serde_json;
#[macro_use]
extern crate log;
extern crate chrono;
extern crate term;
extern crate url;
extern crate uuid;

extern crate failure;
#[macro_use]
extern crate failure_derive;
extern crate futures;
extern crate hyper;
extern crate ring;
extern crate tokio;
extern crate tokio_core;
extern crate tokio_retry;

extern crate grin_api as api;
extern crate grin_chain as chain;
extern crate grin_core as core;
extern crate grin_keychain as keychain;
extern crate grin_store as store;
extern crate grin_util as util;

mod adapters;
pub mod command;
pub mod controller;
pub mod display;
mod error;
pub mod libwallet;
pub mod lmdb_wallet;
mod node_clients;
pub mod test_framework;
mod types;

pub use adapters::{
	FileWalletCommAdapter, HTTPWalletCommAdapter, KeybaseWalletCommAdapter, NullWalletCommAdapter,
	WalletCommAdapter,
};
pub use error::{Error, ErrorKind};
pub use libwallet::types::{BlockFees, CbData, NodeClient, WalletBackend, WalletInfo, WalletInst};
pub use lmdb_wallet::{wallet_db_exists, LMDBBackend};
pub use node_clients::{create_coinbase, HTTPNodeClient};
pub use types::{EncryptedWalletSeed, WalletConfig, WalletSeed, SEED_FILE};

use std::sync::Arc;
use util::Mutex;

/// Helper to create an instance of the LMDB wallet
pub fn instantiate_wallet(
	wallet_config: WalletConfig,
	node_client: impl NodeClient + 'static,
	passphrase: &str,
	account: &str,
) -> Result<Arc<Mutex<WalletInst<impl NodeClient, keychain::ExtKeychain>>>, Error> {
	// First test decryption, so we can abort early if we have the wrong password
	let _ = WalletSeed::from_file(&wallet_config, passphrase)?;
	let mut db_wallet = LMDBBackend::new(wallet_config.clone(), passphrase, node_client)?;
	db_wallet.set_parent_key_id_by_name(account)?;
	info!("Using LMDB Backend for wallet");
	Ok(Arc::new(Mutex::new(db_wallet)))
}
