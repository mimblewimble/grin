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

//! The transaction pool, keeping a view of currently-valid transactions that
//! may be confirmed soon.

#![deny(non_upper_case_globals)]
#![deny(non_camel_case_types)]
#![deny(non_snake_case)]
#![deny(unused_mut)]
#![warn(missing_docs)]

pub mod graph;
mod types;
mod blockchain;
mod pool;

extern crate blake2_rfc as blake2;
extern crate grin_core as core;
extern crate grin_keychain as keychain;
extern crate grin_util as util;
extern crate rand;
extern crate serde;
#[macro_use]
extern crate serde_derive;
extern crate time;

pub use pool::TransactionPool;
pub use types::{BlockChain, PoolAdapter, PoolConfig, PoolError, TxSource};
