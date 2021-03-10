// Copyright 2021 The Grin Developers
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

//! The transaction pool, keeping a view of currently valid transactions that
//! may be confirmed soon.

#![deny(non_upper_case_globals)]
#![deny(non_camel_case_types)]
#![deny(non_snake_case)]
#![deny(unused_mut)]

//extern crate blake2_rfc as blake2;
//extern crate grin_core as core;
//extern crate grin_keychain as keychain;
//extern crate grin_util as util;
#[allow(unused_imports)]
#[macro_use] // Needed for Serialize/Deserialize. The compiler complaining here is a bug.
extern crate serde_derive;
#[macro_use]
extern crate log;

mod pool;
pub mod transaction_pool;
pub mod types;

pub use crate::pool::Pool;
pub use crate::transaction_pool::TransactionPool;
pub use crate::types::{
	BlockChain, DandelionConfig, PoolAdapter, PoolConfig, PoolEntry, PoolError, TxSource,
};
