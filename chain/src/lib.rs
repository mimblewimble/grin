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

//! The block chain itself, validates and accepts new blocks, handles reorgs.

#![deny(non_upper_case_globals)]
#![deny(non_camel_case_types)]
#![deny(non_snake_case)]
#![deny(unused_mut)]
#![warn(missing_docs)]

#[macro_use]
extern crate bitflags;
extern crate byteorder;
extern crate croaring;
extern crate lmdb_zero as lmdb;
extern crate lru_cache;
extern crate serde;
#[macro_use]
extern crate serde_derive;
#[macro_use]
extern crate slog;
extern crate failure;
extern crate chrono;
#[macro_use]
extern crate failure_derive;

extern crate grin_core as core;
extern crate grin_keychain as keychain;
extern crate grin_store;
extern crate grin_util as util;

mod chain;
mod error;
pub mod pipe;
pub mod store;
pub mod txhashset;
pub mod types;

// Re-export the base interface

pub use chain::{Chain, MAX_ORPHAN_SIZE};
pub use error::{Error, ErrorKind};
pub use store::ChainStore;
pub use types::{ChainAdapter, Options, Tip, TxHashsetWriteStatus};
