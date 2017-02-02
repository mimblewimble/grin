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

use chain::{self, ChainAdapter};
use core::core;
use p2p::{NetAdapter, Server};
use util::OneTime;

/// Implementation of the NetAdapter for the blockchain. Gets notified when new
/// blocks and transactions are received and forwards to the chain and pool
/// implementations.
pub struct NetToChainAdapter {
	/// the reference copy of the current chain state
	chain_head: Arc<Mutex<chain::Tip>>,
	chain_store: Arc<chain::ChainStore>,
	chain_adapter: Arc<ChainToNetAdapter>,
}

impl NetAdapter for NetToChainAdapter {
	fn height(&self) -> u64 {
		self.chain_head.lock().unwrap().height
	}
	fn transaction_received(&self, tx: core::Transaction) {
		unimplemented!();
	}
	fn block_received(&self, b: core::Block) {
		// TODO delegate to a separate thread to avoid holding up the caller
		debug!("Received block {} from network, going to process.",
		       b.hash());
		// pushing the new block through the chain pipeline
		let store = self.chain_store.clone();
		let chain_adapter = self.chain_adapter.clone();
		let res = chain::process_block(&b, store, chain_adapter, chain::NONE);

		// log errors and update the shared head reference on success
		if let Err(e) = res {
			debug!("Block {} refused by chain: {:?}", b.hash(), e);
		} else if let Ok(Some(tip)) = res {
			let chain_head = self.chain_head.clone();
			let mut head = chain_head.lock().unwrap();
			*head = tip;
		}
	}
}

impl NetToChainAdapter {
	pub fn new(chain_head: Arc<Mutex<chain::Tip>>,
	           chain_store: Arc<chain::ChainStore>,
	           chain_adapter: Arc<ChainToNetAdapter>)
	           -> NetToChainAdapter {
		NetToChainAdapter {
			chain_head: chain_head,
			chain_store: chain_store,
			chain_adapter: chain_adapter,
		}
	}
}

/// Implementation of the ChainAdapter for the network. Gets notified when the
/// blockchain accepted a new block and forwards it to the network for
/// broadcast.
pub struct ChainToNetAdapter {
	p2p: OneTime<Arc<Server>>,
}

impl ChainAdapter for ChainToNetAdapter {
	fn block_accepted(&self, b: &core::Block) {
		self.p2p.borrow().broadcast_block(b);
	}
}

impl ChainToNetAdapter {
	pub fn new() -> ChainToNetAdapter {
		ChainToNetAdapter { p2p: OneTime::new() }
	}
	pub fn init(&self, p2p: Arc<Server>) {
		self.p2p.init(p2p);
	}
}
