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
extern crate serde_json;
#[macro_use]
extern crate slog;
extern crate term;
extern crate urlencoded;
extern crate uuid;

extern crate bodyparser;
extern crate failure;
#[macro_use]
extern crate failure_derive;
extern crate futures;
extern crate hyper;
extern crate iron;
#[macro_use]
extern crate router;
extern crate tokio_core;
extern crate tokio_retry;

extern crate grin_api as api;
extern crate grin_core as core;
extern crate grin_keychain as keychain;
extern crate grin_store as store;
extern crate grin_util as util;

mod client;
pub mod display;
mod error;
pub mod file_wallet;
pub mod libtx;
pub mod libwallet;
pub mod lmdb_wallet;
mod types;

pub use client::{create_coinbase, HTTPWalletClient};
pub use error::{Error, ErrorKind};
pub use file_wallet::FileWallet;
pub use libwallet::controller;
pub use libwallet::types::{
	BlockFees, CbData, WalletBackend, WalletClient, WalletInfo, WalletInst,
};
pub use lmdb_wallet::{wallet_db_exists, LMDBBackend};
pub use types::{WalletConfig, WalletSeed};
