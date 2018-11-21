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
			let head_header = match self.chain.head_header() {
				Ok(header) => header,
				Err(e) => {
					error!("kernel_sync: check_run err! {:?}", e);
					return false;
				}
			};

			let num_kernels_received = self.chain.get_num_kernels();
			if !self.kernel_sync_due(&head_header, num_kernels_received) {
				return false;
			}

			self.sync_state.update(SyncStatus::KernelSync {
				kernels_received: num_kernels_received,
				total_kernels: head_header.kernel_mmr_size,
			});

			// DAVID: If no capable peer exists, fall back to full txhashset download
			self.kernel_sync(&head_header, num_kernels_received);

			return true;
		}
		false
	}

	fn kernel_sync_due(&mut self, head_header: &BlockHeader, num_kernels_received: u64) -> bool {
		let kernels_5_blocks_back = if head_header.height < 5 {
			0 as u64
		} else {
			match self.chain.get_header_by_height(head_header.height - 5) {
				Ok(header) => header.kernel_mmr_size,
				Err(_) => 0 as u64,
			}
		};

		// Kernels are up to date on the current fork.
		if num_kernels_received > kernels_5_blocks_back {
			return false;
		}

		let now = Utc::now();
		let (timeout, last_kernels_received, prev_kernels_received) = self.prev_kernel_sync;

		// received all necessary kernels, can ask for more
		let can_request_more =
			num_kernels_received >= prev_kernels_received + (p2p::MAX_KERNELS as u64) - 4;

		// no kernels processed and we're past timeout, need to ask for more
		let stalling = num_kernels_received <= last_kernels_received && now > timeout;

		if can_request_more || stalling {
			self.prev_kernel_sync = (
				now + Duration::seconds(10),
				num_kernels_received,
				num_kernels_received,
			);
			true
		} else {
			// resetting the timeout as long as we progress
			if num_kernels_received > last_kernels_received {
				self.prev_kernel_sync = (
					now + Duration::seconds(2),
					num_kernels_received,
					prev_kernels_received,
				);
			}
			false
		}
	}

	fn kernel_sync(
		&mut self,
		head_header: &BlockHeader,
		next_kernel_index: u64,
	) -> Result<(), p2p::Error> {
		let opt_peer = self.peers.most_work_peers().into_iter().find(|peer| {
			peer.info
				.capabilities
				.contains(Capabilities::ENHANCED_TXHASHSET_HIST)
		});

		if let Some(peer) = opt_peer {
			debug!(
				"kernel_sync: asking {} for kernels at {:?}",
				peer.info.addr, next_kernel_index
			);

			let _ =
				peer.send_kernel_request(head_header.hash(), head_header.height, next_kernel_index);
			return Ok(());
		}
		Err(p2p::Error::PeerException)
	}
}
