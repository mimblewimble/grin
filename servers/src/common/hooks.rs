// Copyright 2019 The Grin Developers
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

//! Adapters connecting new block, new transaction, and accepted transaction
//! events to consumers of those events.

use crate::chain::BlockStatus;
use crate::core::core;
use crate::core::core::hash::Hashed;
use std::net::SocketAddr;

#[allow(unused_variables)]
/// Trait to be implemented by Network Event Hooks
pub trait NetEvents {
	/// Triggers when a new transaction arrives
	fn on_transaction_received(&self, tx: &core::Transaction) {}

	/// Triggers when a new block arrives
	fn on_block_received(&self, block: &core::Block, addr: &SocketAddr) {}

	/// Triggers when a new block header arrives
	fn on_header_received(&self, bh: &core::BlockHeader, addr: &SocketAddr) {}
}

#[allow(unused_variables)]
/// Trait to be implemented by Chain Event Hooks
pub trait ChainEvents {
	/// Triggers when a new block is accepted by the chain (might be a Reorg or a Fork)
	fn on_block_accepted(&self, block: &core::Block, status: &BlockStatus) {}
}

/// Basic Logger
pub struct EventLogger;

impl NetEvents for EventLogger {
	fn on_transaction_received(&self, tx: &core::Transaction) {
		debug!(
			"Received tx {}, [in/out/kern: {}/{}/{}] going to process.",
			tx.hash(),
			tx.inputs().len(),
			tx.outputs().len(),
			tx.kernels().len(),
		);
	}

	fn on_block_received(&self, block: &core::Block, addr: &SocketAddr) {
		debug!(
			"Received block {} at {} from {} [in/out/kern: {}/{}/{}] going to process.",
			block.hash(),
			block.header.height,
			addr,
			block.inputs().len(),
			block.outputs().len(),
			block.kernels().len(),
		);
	}

	fn on_header_received(&self, header: &core::BlockHeader, addr: &SocketAddr) {
		debug!(
			"Received block header {} at {} from {}, going to process.",
			header.hash(),
			header.height,
			addr
		);
	}
}

impl ChainEvents for EventLogger {
	fn on_block_accepted(&self, block: &core::Block, status: &BlockStatus) {
		match status {
			BlockStatus::Reorg => {
				warn!(
					"block_accepted (REORG!): {:?} at {} (diff: {})",
					block.hash(),
					block.header.height,
					block.header.total_difficulty(),
				);
			}
			BlockStatus::Fork => {
				debug!(
					"block_accepted (fork?): {:?} at {} (diff: {})",
					block.hash(),
					block.header.height,
					block.header.total_difficulty(),
				);
			}
			BlockStatus::Next => {
				debug!(
					"block_accepted (head+): {:?} at {} (diff: {})",
					block.hash(),
					block.header.height,
					block.header.total_difficulty(),
				);
			}
		}
	}
}
