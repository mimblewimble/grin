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

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time;

use chain;
use common::types::{SyncState, SyncStatus};
use core::pow::Difficulty;
use grin::sync::body_sync::BodySync;
use grin::sync::header_sync::HeaderSync;
use grin::sync::state_sync::StateSync;
use p2p;

pub fn run_sync(
	sync_state: Arc<SyncState>,
	peers: Arc<p2p::Peers>,
	chain: Arc<chain::Chain>,
	stop: Arc<AtomicBool>,
) {
	let _ = thread::Builder::new()
		.name("sync".to_string())
		.spawn(move || {
			let runner = SyncRunner::new(sync_state, peers, chain, stop);
			runner.sync_loop();
		});
}

pub struct SyncRunner {
	sync_state: Arc<SyncState>,
	peers: Arc<p2p::Peers>,
	chain: Arc<chain::Chain>,
	stop: Arc<AtomicBool>,
}

impl SyncRunner {
	fn new(
		sync_state: Arc<SyncState>,
		peers: Arc<p2p::Peers>,
		chain: Arc<chain::Chain>,
		stop: Arc<AtomicBool>,
	) -> SyncRunner {
		SyncRunner {
			sync_state,
			peers,
			chain,
			stop,
		}
	}

	fn wait_for_min_peers(&self) {
		// Initial sleep to give us time to peer with some nodes.
		// Note: Even if we have skip peer wait we need to wait a
		// short period of time for tests to do the right thing.
		let wait_secs = if let SyncStatus::AwaitingPeers(true) = self.sync_state.status() {
			30
		} else {
			3
		};

		let head = self.chain.head().unwrap();

		let mut n = 0;
		const MIN_PEERS: usize = 3;
		loop {
			let wp = self.peers.more_work_peers();
			// exit loop when:
			// * we have more than MIN_PEERS more_work peers
			// * we are synced already, e.g. grin was quickly restarted
			// * timeout
			if wp.len() > MIN_PEERS
				|| (wp.len() == 0
					&& self.peers.enough_peers()
					&& head.total_difficulty > Difficulty::zero())
				|| n > wait_secs
			{
				break;
			}
			thread::sleep(time::Duration::from_secs(1));
			n += 1;
		}
	}

	/// Starts the syncing loop, just spawns two threads that loop forever
	fn sync_loop(&self) {
		// Wait for connections reach at least MIN_PEERS
		self.wait_for_min_peers();

		// Our 3 main sync stages
		let mut header_sync = HeaderSync::new(
			self.sync_state.clone(),
			self.peers.clone(),
			self.chain.clone(),
		);
		let mut body_sync = BodySync::new(
			self.sync_state.clone(),
			self.peers.clone(),
			self.chain.clone(),
		);
		let mut state_sync = StateSync::new(
			self.sync_state.clone(),
			self.peers.clone(),
			self.chain.clone(),
		);

		// Highest height seen on the network, generally useful for a fast test on
		// whether some sync is needed
		let mut highest_height = 0;

		// Main syncing loop
		while !self.stop.load(Ordering::Relaxed) {
			thread::sleep(time::Duration::from_millis(10));

			// check whether syncing is generally needed, when we compare our state with others
			let (syncing, most_work_height) = self.needs_syncing();

			if most_work_height > 0 {
				// we can occasionally get a most work height of 0 if read locks fail
				highest_height = most_work_height;
			}

			// quick short-circuit (and a decent sleep) if no syncing is needed
			if !syncing {
				self.sync_state.update(SyncStatus::NoSync);
				thread::sleep(time::Duration::from_secs(10));
				continue;
			}

			// if syncing is needed
			let head = self.chain.head().unwrap();
			let tail = self.chain.tail().unwrap_or_else(|_| head.clone());
			let header_head = self.chain.header_head().unwrap();

			// run each sync stage, each of them deciding whether they're needed
			// except for state sync that only runs if body sync return true (means txhashset is needed)
			header_sync.check_run(&header_head, highest_height);

			let mut check_state_sync = false;
			match self.sync_state.status() {
				SyncStatus::TxHashsetDownload { .. }
				| SyncStatus::TxHashsetSetup
				| SyncStatus::TxHashsetValidation { .. }
				| SyncStatus::TxHashsetSave
				| SyncStatus::TxHashsetDone => check_state_sync = true,
				_ => {
					// skip body sync if header chain is not synced.
					if header_head.height < highest_height {
						continue;
					}

					if body_sync.check_run(&head, highest_height) {
						check_state_sync = true;
					}
				}
			}

			if check_state_sync {
				state_sync.check_run(&header_head, &head, &tail, highest_height);
			}
		}
	}

	/// Whether we're currently syncing the chain or we're fully caught up and
	/// just receiving blocks through gossip.
	fn needs_syncing(&self) -> (bool, u64) {
		let local_diff = self.chain.head().unwrap().total_difficulty;
		let mut is_syncing = self.sync_state.is_syncing();
		let peer = self.peers.most_work_peer();

		let peer_info = if let Some(p) = peer {
			p.info.clone()
		} else {
			warn!("sync: no peers available, disabling sync");
			return (false, 0);
		};

		// if we're already syncing, we're caught up if no peer has a higher
		// difficulty than us
		if is_syncing {
			if peer_info.total_difficulty() <= local_diff {
				let ch = self.chain.head().unwrap();
				info!(
					"synchronized at {} @ {} [{}]",
					local_diff.to_num(),
					ch.height,
					ch.last_block_h
				);
				is_syncing = false;
			}
		} else {
			// sum the last 5 difficulties to give us the threshold
			let threshold = self
				.chain
				.difficulty_iter()
				.map(|x| x.difficulty)
				.take(5)
				.fold(Difficulty::zero(), |sum, val| sum + val);

			let peer_diff = peer_info.total_difficulty();
			if peer_diff > local_diff.clone() + threshold.clone() {
				info!(
					"sync: total_difficulty {}, peer_difficulty {}, threshold {} (last 5 blocks), enabling sync",
					local_diff,
					peer_diff,
					threshold,
				);
				is_syncing = true;
			}
		}
		(is_syncing, peer_info.height())
	}
}
