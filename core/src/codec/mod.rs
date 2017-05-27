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

//! Codecs for Blocks and Transactions

use tokio_io::codec::Encoder;

pub mod block;
pub mod tx;

pub trait HashEncode: Sized + Clone {
	type HashEncoder: Encoder<Item = Self> + Default;
}

#[cfg(test)]
mod block_test;
#[cfg(test)]
mod tx_test;

pub use self::block::{BlockCodec, BlockHasher };
pub use self::tx::TxCodec;
