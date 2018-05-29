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

//! Networking code to connect to other peers and exchange block, transactions,
//! etc.

#![deny(non_upper_case_globals)]
#![deny(non_camel_case_types)]
#![deny(non_snake_case)]
#![deny(unused_mut)]

#[macro_use]
extern crate bitflags;
extern crate bytes;
#[macro_use]
extern crate enum_primitive;

#[macro_use]
extern crate grin_core as core;
extern crate grin_store;
extern crate grin_util as util;
extern crate num;
extern crate rand;
extern crate serde;
#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate slog;
extern crate time;

mod conn;
pub mod handshake;
pub mod msg;
mod peer;
mod peers;
mod protocol;
mod serv;
mod store;
pub mod types;

pub use peer::Peer;
pub use peers::Peers;
pub use serv::{DummyAdapter, Server};
pub use store::{PeerData, State};
pub use types::{Capabilities, ChainAdapter, DandelionConfig, Direction, Error, P2PConfig,
                PeerInfo, ReasonForBan, TxHashSetRead, MAX_BLOCK_HEADERS, MAX_LOCATORS,
                MAX_PEER_ADDRS};
