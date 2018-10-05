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
use p2p::{self, Peers};
use util::LOGGER;

pub fn run_sync(
	sync_state: Arc<SyncState>,
	awaiting_peers: Arc<AtomicBool>,
	peers: Arc<p2p::Peers>,
	chain: Arc<chain::Chain>,
	skip_sync_wait: bool,
	archive_mode: bool,
	stop: Arc<AtomicBool>,
) {
	let _ = thread::Builder::new()
		.name("sync".to_string())
		.spawn(move || {
			sync_loop(
				sync_state,
				awaiting_peers,
				peers,
				chain,
				skip_sync_wait,
				archive_mode,
				stop,
			)
		});
}

fn wait_for_min_peers(
	awaiting_peers: Arc<AtomicBool>,
	peers: Arc<p2p::Peers>,
	chain: Arc<chain::Chain>,
	skip_sync_wait: bool,
) {
	// Initial sleep to give us time to peer with some nodes.
	// Note: Even if we have "skip_sync_wait" we need to wait a
	// short period of time for tests to do the right thing.
	let wait_secs = if skip_sync_wait { 3 } else { 30 };

	let head = chain.head().unwrap();

	awaiting_peers.store(true, Ordering::Relaxed);
	let mut n = 0;
	const MIN_PEERS: usize = 3;
	loop {
		let wp = peers.more_work_peers();
		// exit loop when:
		// * we have more than MIN_PEERS more_work peers
		// * we are synced already, e.g. grin was quickly restarted
		// * timeout
		if wp.len() > MIN_PEERS
			|| (wp.len() == 0 && peers.enough_peers() && head.total_difficulty > Difficulty::zero())
			|| n > wait_secs
		{
			break;
		}
		thread::sleep(time::Duration::from_secs(1));
		n += 1;
	}
	awaiting_peers.store(false, Ordering::Relaxed);
}

/// Starts the syncing loop, just spawns two threads that loop forever
fn sync_loop(
	sync_state: Arc<SyncState>,
	awaiting_peers: Arc<AtomicBool>,
	peers: Arc<p2p::Peers>,
	chain: Arc<chain::Chain>,
	skip_sync_wait: bool,
	archive_mode: bool,
	stop: Arc<AtomicBool>,
) {
	// Wait for connections reach at least MIN_PEERS
	wait_for_min_peers(awaiting_peers, peers.clone(), chain.clone(), skip_sync_wait);

	// Our 3 main sync stages
	let mut header_sync = HeaderSync::new(sync_state.clone(), peers.clone(), chain.clone());
	let mut body_sync = BodySync::new(sync_state.clone(), peers.clone(), chain.clone());
	let mut state_sync = StateSync::new(
		sync_state.clone(),
		peers.clone(),
		chain.clone(),
		archive_mode,
	);

	// Highest height seen on the network, generally useful for a fast test on
	// whether some sync is needed
	let mut highest_height = 0;

	// Main syncing loop
	while !stop.load(Ordering::Relaxed) {
		thread::sleep(time::Duration::from_millis(10));

		// check whether syncing is generally needed, when we compare our state with others
		let (syncing, most_work_height) =
			needs_syncing(sync_state.as_ref(), peers.clone(), chain.clone());

		if most_work_height > 0 {
			// we can occasionally get a most work height of 0 if read locks fail
			highest_height = most_work_height;
		}

		// quick short-circuit if no syncing is needed
		if !syncing {
			sync_state.update(SyncStatus::NoSync);
			continue;
		}

		// if syncing is needed
		let head = chain.head().unwrap();
		let header_head = chain.header_head().unwrap();

		// run each sync stage, each of them deciding whether they're needed
		// except for body sync that only runs if state sync is off or done
		header_sync.check_run(&header_head, highest_height);
		if !state_sync.check_run(&header_head, &head, highest_height) {
			body_sync.check_run(&head, highest_height);
		}
	}
}

/// Whether we're currently syncing the chain or we're fully caught up and
/// just receiving blocks through gossip.
fn needs_syncing(
	sync_state: &SyncState,
	peers: Arc<Peers>,
	chain: Arc<chain::Chain>,
) -> (bool, u64) {
	let local_diff = chain.head().unwrap().total_difficulty;
	let peer = peers.most_work_peer();
	let is_syncing = sync_state.is_syncing();
	let mut most_work_height = 0;

	// if we're already syncing, we're caught up if no peer has a higher
	// difficulty than us
	if is_syncing {
		if let Some(peer) = peer {
			if let Ok(peer) = peer.try_read() {
				most_work_height = peer.info.height;

				if peer.info.total_difficulty <= local_diff {
					let ch = chain.head().unwrap();
					info!(
						LOGGER,
						"synchronized at {} @ {} [{}]",
						local_diff.to_num(),
						ch.height,
						ch.last_block_h
					);

					let _ = chain.reset_head();
					return (false, most_work_height);
				}
			}
		} else {
			warn!(LOGGER, "sync: no peers available, disabling sync");
			return (false, 0);
		}
	} else {
		if let Some(peer) = peer {
			if let Ok(peer) = peer.try_read() {
				most_work_height = peer.info.height;

				// sum the last 5 difficulties to give us the threshold
				let threshold = chain
					.difficulty_iter()
					.filter_map(|x| x.map(|(_, x)| x).ok())
					.take(5)
					.fold(Difficulty::zero(), |sum, val| sum + val);

				if peer.info.total_difficulty > local_diff.clone() + threshold.clone() {
					info!(
						LOGGER,
						"sync: total_difficulty {}, peer_difficulty {}, threshold {} (last 5 blocks), enabling sync",
						local_diff,
						peer.info.total_difficulty,
						threshold,
					);
					return (true, most_work_height);
				}
			}
		}
	}
	(is_syncing, most_work_height)
}
