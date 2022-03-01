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

use std::sync::Arc;
use std::thread;
use std::time;

use crate::chain::{self, SyncState, SyncStatus};
use crate::core::global;
use crate::core::pow::Difficulty;
use crate::grin::sync::body_sync::BodySync;
use crate::grin::sync::header_sync::HeaderSync;
use crate::grin::sync::state_sync::StateSync;
use crate::p2p;
use crate::util::StopState;

pub fn run_sync(
	sync_state: Arc<SyncState>,
	peers: Arc<p2p::Peers>,
	chain: Arc<chain::Chain>,
	stop_state: Arc<StopState>,
) -> std::io::Result<std::thread::JoinHandle<()>> {
	thread::Builder::new()
		.name("sync".to_string())
		.spawn(move || {
			let runner = SyncRunner::new(sync_state, peers, chain, stop_state);
			runner.sync_loop();
		})
}

pub struct SyncRunner {
	sync_state: Arc<SyncState>,
	peers: Arc<p2p::Peers>,
	chain: Arc<chain::Chain>,
	stop_state: Arc<StopState>,
}

impl SyncRunner {
	fn new(
		sync_state: Arc<SyncState>,
		peers: Arc<p2p::Peers>,
		chain: Arc<chain::Chain>,
		stop_state: Arc<StopState>,
	) -> SyncRunner {
		SyncRunner {
			sync_state,
			peers,
			chain,
			stop_state,
		}
	}

	fn wait_for_min_peers(&self) -> Result<(), chain::Error> {
		// Initial sleep to give us time to peer with some nodes.
		// Note: Even if we have skip peer wait we need to wait a
		// short period of time for tests to do the right thing.
		let wait_secs = if let SyncStatus::AwaitingPeers(true) = self.sync_state.status() {
			30
		} else {
			3
		};

		let head = self.chain.head()?;

		let mut n = 0;
		const MIN_PEERS: usize = 3;
		loop {
			if self.stop_state.is_stopped() {
				break;
			}
			// Count peers with at least our difficulty.
			let wp = self
				.peers
				.iter()
				.outbound()
				.with_difficulty(|x| x >= head.total_difficulty)
				.connected()
				.count();

			// exit loop when:
			// * we have more than MIN_PEERS more_or_same_work peers
			// * we are synced already, e.g. grin was quickly restarted
			// * timeout
			if wp > MIN_PEERS
				|| (wp == 0
					&& self.peers.enough_outbound_peers()
					&& head.total_difficulty > Difficulty::zero())
				|| n > wait_secs
			{
				if wp > 0 || !global::is_production_mode() {
					break;
				}
			}
			thread::sleep(time::Duration::from_secs(1));
			n += 1;
		}
		Ok(())
	}

	/// Starts the syncing loop, just spawns two threads that loop forever
	fn sync_loop(&self) {
		macro_rules! unwrap_or_restart_loop(
	($obj: expr) =>(
		match $obj {
			Ok(v) => v,
			Err(e) => {
				error!("unexpected error: {:?}", e);
				thread::sleep(time::Duration::from_secs(1));
				continue;
			},
		}
	));

		// Wait for connections reach at least MIN_PEERS
		if let Err(e) = self.wait_for_min_peers() {
			error!("wait_for_min_peers failed: {:?}", e);
		}

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
		loop {
			if self.stop_state.is_stopped() {
				break;
			}

			thread::sleep(time::Duration::from_millis(10));

			let currently_syncing = self.sync_state.is_syncing();

			// check whether syncing is generally needed, when we compare our state with others
			let (needs_syncing, most_work_height) = unwrap_or_restart_loop!(self.needs_syncing());
			if most_work_height > 0 {
				// we can occasionally get a most work height of 0 if read locks fail
				highest_height = most_work_height;
			}

			// quick short-circuit (and a decent sleep) if no syncing is needed
			if !needs_syncing {
				if currently_syncing {
					self.sync_state.update(SyncStatus::NoSync);

					// Initial transition out of a "syncing" state and into NoSync.
					// This triggers a chain compaction to keep out local node tidy.
					// Note: Chain compaction runs with an internal threshold
					// so can be safely run even if the node is restarted frequently.
					unwrap_or_restart_loop!(self.chain.compact());
				}

				// sleep for 10 secs but check stop signal every second
				for _ in 1..10 {
					thread::sleep(time::Duration::from_secs(1));
					if self.stop_state.is_stopped() {
						break;
					}
				}
				continue;
			}

			// if syncing is needed
			let head = unwrap_or_restart_loop!(self.chain.head());
			let tail = self.chain.tail().unwrap_or_else(|_| head.clone());
			let header_head = unwrap_or_restart_loop!(self.chain.header_head());

			// "sync_head" allows us to sync against a large fork on the header chain
			// we track this during an extended header sync
			let sync_status = self.sync_state.status();

			let sync_head = match sync_status {
				SyncStatus::HeaderSync { sync_head, .. } => sync_head,
				_ => header_head,
			};

			// run each sync stage, each of them deciding whether they're needed
			// except for state sync that only runs if body sync return true (means txhashset is needed)
			unwrap_or_restart_loop!(header_sync.check_run(sync_head));

			let mut check_state_sync = false;
			match self.sync_state.status() {
				SyncStatus::TxHashsetPibd { .. }
				| SyncStatus::TxHashsetDownload { .. }
				| SyncStatus::TxHashsetSetup { .. }
				| SyncStatus::TxHashsetRangeProofsValidation { .. }
				| SyncStatus::TxHashsetKernelsValidation { .. }
				| SyncStatus::TxHashsetSave
				| SyncStatus::TxHashsetDone => check_state_sync = true,
				_ => {
					// skip body sync if header chain is not synced.
					if sync_head.height < highest_height {
						continue;
					}

					let check_run =
						unwrap_or_restart_loop!(body_sync.check_run(&head, highest_height));
					if check_run {
						check_state_sync = true;
					}
				}
			}

			if check_state_sync {
				state_sync.check_run(
					&header_head,
					&head,
					&tail,
					highest_height,
					self.stop_state.clone(),
				);
			}
		}
	}

	/// Whether we're currently syncing the chain or we're fully caught up and
	/// just receiving blocks through gossip.
	fn needs_syncing(&self) -> Result<(bool, u64), chain::Error> {
		let local_diff = self.chain.head()?.total_difficulty;
		let mut is_syncing = self.sync_state.is_syncing();

		// Find a peer with greatest known difficulty.
		// Consider all peers, both inbound and outbound.
		// We will prioritize syncing against outbound peers exclusively when possible.
		// But we do support syncing against an inbound peer if greater work than any outbound peers.
		let max_diff = self
			.peers
			.iter()
			.connected()
			.max_difficulty()
			.unwrap_or(Difficulty::zero());
		let peer = self
			.peers
			.iter()
			.with_difficulty(|x| x >= max_diff)
			.connected()
			.choose_random();

		let peer_info = if let Some(p) = peer {
			p.info.clone()
		} else {
			warn!("sync: no peers available, disabling sync");
			return Ok((false, 0));
		};

		// if we're already syncing, we're caught up if no peer has a higher
		// difficulty than us
		if is_syncing {
			if peer_info.total_difficulty() <= local_diff {
				let ch = self.chain.head()?;
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
			let threshold = {
				let diff_iter = match self.chain.difficulty_iter() {
					Ok(v) => v,
					Err(e) => {
						error!("failed to get difficulty iterator: {:?}", e);
						// we handle 0 height in the caller
						return Ok((false, 0));
					}
				};
				diff_iter
					.map(|x| x.difficulty)
					.take(5)
					.fold(Difficulty::zero(), |sum, val| sum + val)
			};

			let peer_diff = peer_info.total_difficulty();
			if peer_diff > local_diff + threshold {
				info!(
					"sync: total_difficulty {}, peer_difficulty {}, threshold {} (last 5 blocks), enabling sync",
					local_diff,
					peer_diff,
					threshold,
				);
				is_syncing = true;
			}
		}
		Ok((is_syncing, peer_info.height()))
	}
}
