// Copyright 2016 The Grin Developers
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

extern crate byteorder;
extern crate crypto;
#[macro_use]
extern crate log;
extern crate rand;
extern crate serde;
#[macro_use]
extern crate serde_derive;
extern crate serde_json;

extern crate grin_api as api;
extern crate grin_core as core;
extern crate grin_util as util;
extern crate secp256k1zkp as secp;

mod checker;
mod extkey;
mod receiver;
mod sender;
mod types;

pub use extkey::ExtendedKey;
pub use receiver::{WalletReceiver, receive_json_tx};
pub use sender::issue_send_tx;
