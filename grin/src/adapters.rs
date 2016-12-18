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

use std::sync::{Arc, Mutex};

use chain;
use core::core;
use p2p::NetAdapter;

pub struct NetToChainAdapter {
	chain_store: Arc<chain::ChainStore>,
}

impl NetAdapter for NetToChainAdapter {
	fn transaction_received(&self, tx: core::Transaction) {
		unimplemented!();
	}
	fn block_received(&self, b: core::Block) {
		// if let Err(e) = chain::process_block(&b, self.chain_store,
		// chain::pipe::NONE) {
		//   debug!("Block {} refused by chain: {}", b.hash(), e);
		// }
		unimplemented!();
	}
}

impl NetToChainAdapter {
	pub fn new(chain_store: Arc<chain::ChainStore>) -> NetToChainAdapter {
		NetToChainAdapter { chain_store: chain_store }
	}
}
