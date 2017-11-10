// Copyright 2017 The Grin Developers
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
extern crate rand;
extern crate serde;
#[macro_use]
extern crate serde_derive;
extern crate serde_json;
#[macro_use]
extern crate slog;
#[macro_use]
extern crate prettytable;
extern crate term;

extern crate bodyparser;
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
extern crate grin_util as util;

mod checker;
mod handlers;
mod outputs;
mod info;
mod receiver;
mod sender;
mod types;
pub mod client;
pub mod server;

pub use outputs::show_outputs;
pub use info::show_info;
pub use receiver::{receive_json_tx, receive_json_tx_str, WalletReceiver};
pub use sender::{issue_burn_tx, issue_send_tx};
pub use types::{BlockFees, CbData, Error, WalletConfig, WalletReceiveRequest, WalletSeed};
