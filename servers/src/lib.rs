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

//! Main crate putting together all the other crates that compose Grin into a
//! binary.

#![deny(non_upper_case_globals)]
#![deny(non_camel_case_types)]
#![deny(non_snake_case)]
#![deny(unused_mut)]
#![warn(missing_docs)]

extern crate bufstream;
extern crate futures;
extern crate http;
extern crate hyper;
extern crate hyper_staticfile;
extern crate itertools;
extern crate jsonrpc_core;
extern crate lmdb_zero as lmdb;
extern crate rand;
extern crate serde;
#[macro_use]
extern crate serde_derive;
extern crate serde_json;
#[macro_use]
extern crate slog;
extern crate chrono;

extern crate grin_api as api;
extern crate grin_chain as chain;
extern crate grin_core as core;
extern crate grin_keychain as keychain;
extern crate grin_p2p as p2p;
extern crate grin_pool as pool;
extern crate grin_store as store;
extern crate grin_util as util;
extern crate grin_wallet as wallet;

pub mod common;
mod grin;
mod mining;
mod webwallet;

pub use common::stats::{DiffBlock, PeerStats, ServerStats, StratumStats, WorkerStats};
pub use common::types::{ServerConfig, StratumServerConfig};
pub use grin::server::Server;
pub use webwallet::server::start_webwallet_server;
