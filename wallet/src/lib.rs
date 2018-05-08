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

#[macro_use]
extern crate lazy_static;

extern crate grin_api as api;
extern crate grin_core as core;
extern crate grin_keychain as keychain;
extern crate grin_util as util;

mod checker;
mod handlers;
mod outputs;
mod info;
mod receiver;
mod sender;
mod types;
mod restore;
pub mod client;
pub mod server;
pub mod libwallet;

pub use outputs::show_outputs;
pub use info::{retrieve_info, show_info};
pub use receiver::WalletReceiver;
pub use sender::{issue_burn_tx, issue_send_tx};
pub use types::{BlockFees, CbData, Error, ErrorKind, WalletConfig, WalletInfo,
                WalletReceiveRequest, WalletSeed};
pub use restore::restore;
