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

	timeout: DateTime<Utc>,
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
			timeout: Utc::now(),
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

			let header = match self.chain.get_block_header(&header_head.last_block_h) {
				Ok(header) => header,
				Err(e) => {
					error!("kernel_sync: check_run err! {:?}", e);
					return false;
				}
			};

			let num_kernels = self.chain.get_num_kernels();

			if !self.kernel_sync_due(num_kernels) {
				return false;
			}

			self.sync_state.update(SyncStatus::KernelSync {
				current_index: num_kernels,
				highest_index: header.kernel_mmr_size,
			});

			let _ = self.kernel_sync(0);

			return true;
		}
		false
	}

	fn kernel_sync_due(&mut self, num_kernels: u64) -> bool {
		// During first iteration, this is all or nothing. All kernels are downloaded at once.
		if num_kernels > 0 {
			return false;
		}

		let now = Utc::now();

		// no kernels processed and we're past timeout, need to ask for more
		if now > self.timeout {
			self.timeout = now + Duration::minutes(3);
			true
		} else {
			false
		}
	}

	fn kernel_sync(&mut self, first_block_height: u64) -> Result<(), p2p::Error> {
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
