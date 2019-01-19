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

#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate log;

use grin_api as api;
use grin_chain as chain;
use grin_core as core;
use grin_keychain as keychain;
use grin_p2p as p2p;
use grin_pool as pool;
use grin_store as store;
use grin_util as util;
use grin_wallet as wallet;

pub mod common;
mod grin;
mod mining;
mod webwallet;

pub use crate::common::stats::{DiffBlock, PeerStats, ServerStats, StratumStats, WorkerStats};
pub use crate::common::types::{ServerConfig, StratumServerConfig};
pub use crate::grin::server::Server;
pub use crate::webwallet::server::start_webwallet_server;
