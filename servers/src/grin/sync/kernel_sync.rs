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

use chrono::prelude::{DateTime, Utc};
use chrono::Duration;
use std::sync::Arc;

use chain;
use chain::Tip;
use common::types::{SyncState, SyncStatus};
use core::core::hash::Hashed;
use core::core::BlockHeader;
use p2p;
use p2p::types::Capabilities;

/// Fast sync has 4 "states":
/// * syncing headers
/// * once all headers are sync'd, sync kernels
/// * once kernels are sync'd, requesting the txhashset state
/// * once we have the state, get blocks after that
///
/// The KernelSync struct implements and monitors the second step.
pub struct KernelSync {
	sync_state: Arc<SyncState>,
	peers: Arc<p2p::Peers>,
	chain: Arc<chain::Chain>,
	capabilities: p2p::Capabilities,

	/// Holds the timeout, num kernels received, and previous num kernels received
	/// at the time of the previous kernel sync.
	prev_kernel_sync: (DateTime<Utc>, u64, u64),
}

impl KernelSync {
	pub fn new(
		sync_state: Arc<SyncState>,
		peers: Arc<p2p::Peers>,
		chain: Arc<chain::Chain>,
		capabilities: p2p::Capabilities,
	) -> KernelSync {
		KernelSync {
			sync_state,
			peers,
			chain,
			capabilities,
			prev_kernel_sync: (Utc::now(), 0, 0),
		}
	}

	/// Check whether kernel sync should run and requests kernels from capable peers.
	pub fn check_run(&mut self) -> bool {
		let enable_kernel_sync = self
			.capabilities
			.contains(Capabilities::ENHANCED_TXHASHSET_HIST);

		if enable_kernel_sync {
			let header_head = match self.chain.header_head() {
				Ok(header_head) => header_head,
				Err(e) => {
					error!("kernel_sync: check_run err! {:?}", e);
					return false;
				}
			};

			let kernel_tip = match self.chain.get_kernel_root_validated_tip() {
				Ok(kernel_tip) => kernel_tip,
				Err(e) => {
					error!("kernel_sync: check_run err! {:?}", e);
					return false;
				}
			};

			if !self.kernel_sync_due(&header_head, &kernel_tip) {
				return false;
			}

			self.sync_state.update(SyncStatus::KernelSync {
				current_height: kernel_tip.height,
				highest_height: header_head.height,
			});

			// DAVID: If no capable peer exists, fall back to full txhashset download
			self.kernel_sync(kernel_tip.height + 1);

			return true;
		}
		false
	}

	fn kernel_sync_due(&mut self, header_head: &Tip, kernel_tip: &BlockHeader) -> bool {
		// Kernels are up to date on the current fork.
		if kernel_tip.height + 5 > header_head.height {
			return false;
		}

		let now = Utc::now();
		let (timeout, last_kernel_blocks_received, prev_kernel_blocks_received) = self.prev_kernel_sync;

		// received all necessary kernels, can ask for more
		let can_request_more =
			kernel_tip.height >= prev_kernel_blocks_received + (p2p::MAX_KERNEL_BLOCKS as u64);

		// no kernels processed and we're past timeout, need to ask for more
		let stalling = kernel_tip.height <= last_kernel_blocks_received && now > timeout;

		if can_request_more || stalling {
			self.prev_kernel_sync = (
				now + Duration::seconds(10),
				kernel_tip.height,
				kernel_tip.height,
			);
			true
		} else {
			// resetting the timeout as long as we progress
			if kernel_tip.height > last_kernel_blocks_received {
				self.prev_kernel_sync = (
					now + Duration::seconds(2),
					kernel_tip.height,
					prev_kernel_blocks_received,
				);
			}
			false
		}
	}

	fn kernel_sync(
		&mut self,
		first_block_height: u64,
	) -> Result<(), p2p::Error> {
		let opt_peer = self.peers.most_work_peers().into_iter().find(|peer| {
			peer.info
				.capabilities
				.contains(Capabilities::ENHANCED_TXHASHSET_HIST)
		});

		if let Some(peer) = opt_peer {
			debug!(
				"kernel_sync: asking {} for kernels starting at block {:?}",
				peer.info.addr, first_block_height
			);

			let _ = peer.send_kernel_request(first_block_height);
			return Ok(());
		}
		Err(p2p::Error::PeerException)
	}
}
