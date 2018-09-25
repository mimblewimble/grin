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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time;
use std::{cmp, thread};

use chain;
use common::types::{Error, SyncState, SyncStatus};
use core::core::hash::{Hash, Hashed};
use core::global;
use core::pow::Difficulty;
use grin::sync;
use p2p::{self, Peer, Peers};
use util::LOGGER;

pub struct Syncer {}

impl Syncer {
	pub fn new() -> Syncer {
		Syncer {}
	}

	pub fn run_sync(
		&self,
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
				sync::run_sync(
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
pub fn run_sync(
	sync_state: Arc<SyncState>,
	awaiting_peers: Arc<AtomicBool>,
	peers: Arc<p2p::Peers>,
	chain: Arc<chain::Chain>,
	skip_sync_wait: bool,
	archive_mode: bool,
	stop: Arc<AtomicBool>,
) {
	let mut si = SyncInfo::new();

	// Wait for connections reach at least MIN_PEERS
	wait_for_min_peers(awaiting_peers, peers.clone(), chain.clone(), skip_sync_wait);

	// fast sync has 3 "states":
	// * syncing headers
	// * once all headers are sync'd, requesting the txhashset state
	// * once we have the state, get blocks after that
	//
	// full sync gets rid of the middle step and just starts from
	// the genesis state

	let mut history_locators: Vec<(u64, Hash)> = vec![];
	let mut body_sync_info = BodySyncInfo {
		sync_start_ts: Utc::now(),
		body_sync_hashes: vec![],
		prev_body_received: None,
		prev_tip: chain.head().unwrap(),
		prev_orphans_len: 0,
	};

	// Main syncing loop
	while !stop.load(Ordering::Relaxed) {
		thread::sleep(time::Duration::from_millis(10));

		// check whether syncing is generally needed, when we compare our state with others
		let (syncing, most_work_height) =
			needs_syncing(sync_state.as_ref(), peers.clone(), chain.clone());

		if most_work_height > 0 {
			// we can occasionally get a most work height of 0 if read locks fail
			si.highest_height = most_work_height;
		}

		// if no syncing is needed
		if !syncing {
			sync_state.update(SyncStatus::NoSync);
			continue;
		}

		// if syncing is needed
		let head = chain.head().unwrap();
		let header_head = chain.get_header_head().unwrap();

		// run the header sync in every 10s at least
		if si.header_sync_due(sync_state.as_ref(), &header_head) {
			do_header_sync(
				sync_state.as_ref(),
				header_head.clone(),
				peers.clone(),
				chain.clone(),
				&si,
				&mut history_locators,
			);
		}

		// if fast_sync is enabled and needed
		let need_fast_sync = !archive_mode
			&& si.highest_height.saturating_sub(head.height) > global::cut_through_horizon() as u64;
		if need_fast_sync {
			do_fast_sync(
				sync_state.as_ref(),
				header_head,
				peers.clone(),
				chain.clone(),
				&mut si,
			);

			continue;
		}

		// if fast_sync disabled or not needed, run the body_sync every 5s
		if si.body_sync_due(&head, chain.clone(), &mut body_sync_info) {
			body_sync(peers.clone(), chain.clone(), &mut body_sync_info);

			sync_state.update(SyncStatus::BodySync {
				current_height: head.height,
				highest_height: si.highest_height,
			});
		}
	}
}

fn do_header_sync(
	sync_state: &SyncState,
	header_head: chain::Tip,
	peers: Arc<p2p::Peers>,
	chain: Arc<chain::Chain>,
	si: &SyncInfo,
	history_locators: &mut Vec<(u64, Hash)>,
) {
	let status = sync_state.status();

	let update_sync_state = match status {
		SyncStatus::TxHashsetDownload => false,
		SyncStatus::NoSync | SyncStatus::Initial => {
			// Reset sync_head to header_head on transition to HeaderSync,
			// but ONLY on initial transition to HeaderSync state.
			let sync_head = chain.get_sync_head().unwrap();
			debug!(
				LOGGER,
				"sync: initial transition to HeaderSync. sync_head: {} at {}, reset to: {} at {}",
				sync_head.hash(),
				sync_head.height,
				header_head.hash(),
				header_head.height,
			);
			chain.init_sync_head(&header_head).unwrap();
			history_locators.clear();
			true
		}
		_ => true,
	};

	if update_sync_state {
		sync_state.update(SyncStatus::HeaderSync {
			current_height: header_head.height,
			highest_height: si.highest_height,
		});
	}

	header_sync(peers, chain, history_locators);
}

fn do_fast_sync(
	sync_state: &SyncState,
	header_head: chain::Tip,
	peers: Arc<p2p::Peers>,
	chain: Arc<chain::Chain>,
	si: &mut SyncInfo,
) {
	let mut sync_need_restart = false;

	// check sync error
	{
		let clone = sync_state.sync_error();
		if let Some(ref sync_error) = *clone.read().unwrap() {
			error!(
				LOGGER,
				"fast_sync: error = {:?}. restart fast sync", sync_error
			);
			sync_need_restart = true;
		}
		drop(clone);
	}

	// check peer connection status of this sync
	if let Some(ref peer) = si.fast_sync_peer {
		if let Ok(p) = peer.try_read() {
			if !p.is_connected() && SyncStatus::TxHashsetDownload == sync_state.status() {
				sync_need_restart = true;
				info!(
					LOGGER,
					"fast_sync: peer connection lost: {:?}. restart", p.info.addr,
				);
			}
		}
	}

	if sync_need_restart {
		si.fast_sync_reset();
		sync_state.clear_sync_error();
	}

	// run fast sync if applicable, normally only run one-time, except restart in error
	if header_head.height == si.highest_height {
		let (go, download_timeout) = si.fast_sync_due();

		if go {
			si.fast_sync_peer = None;
			match fast_sync(peers, chain, &header_head) {
				Ok(peer) => {
					si.fast_sync_peer = Some(peer);
				}
				Err(e) => sync_state.set_sync_error(Error::P2P(e)),
			}
			sync_state.update(SyncStatus::TxHashsetDownload);
		}

		if download_timeout && SyncStatus::TxHashsetDownload == sync_state.status() {
			error!(
				LOGGER,
				"fast_sync: TxHashsetDownload status timeout in 10 minutes!"
			);
			sync_state.set_sync_error(Error::P2P(p2p::Error::Timeout));
		}
	}
}

struct BodySyncInfo {
	sync_start_ts: DateTime<Utc>,
	body_sync_hashes: Vec<Hash>,
	prev_body_received: Option<DateTime<Utc>>,
	prev_tip: chain::Tip,
	prev_orphans_len: usize,
}

impl BodySyncInfo {
	fn reset(&mut self) {
		self.body_sync_hashes.clear();
		self.prev_body_received = None;
	}

	fn reset_start(&mut self, chain: Arc<chain::Chain>) {
		self.prev_tip = chain.head().unwrap();
		self.prev_orphans_len = chain.orphans_len() + chain.orphans_evicted_len();
		self.sync_start_ts = Utc::now();
	}

	fn body_no_more(&mut self, chain: Arc<chain::Chain>) -> bool {
		let tip = chain.head().unwrap();

		match self.prev_body_received {
			Some(prev_ts) => {
				if tip.last_block_h == self.prev_tip.last_block_h
					&& chain.orphans_len() + chain.orphans_evicted_len() == self.prev_orphans_len
					&& Utc::now() - prev_ts > Duration::milliseconds(200)
				{
					let hashes_not_get = self
						.body_sync_hashes
						.iter()
						.filter(|x| !chain.get_block(*x).is_ok() && !chain.is_orphan(*x))
						.collect::<Vec<_>>();
					debug!(
						LOGGER,
						"body_sync: {}/{} blocks received, and no more in 200ms",
						self.body_sync_hashes.len() - hashes_not_get.len(),
						self.body_sync_hashes.len(),
					);
					return true;
				}
			}
			None => {
				if Utc::now() - self.sync_start_ts > Duration::seconds(5) {
					debug!(
						LOGGER,
						"body_sync: 0/{} blocks received in 5s",
						self.body_sync_hashes.len(),
					);
					return true;
				}
			}
		}

		if tip.last_block_h != self.prev_tip.last_block_h
			|| chain.orphans_len() + chain.orphans_evicted_len() != self.prev_orphans_len
		{
			self.prev_tip = tip;
			self.prev_body_received = Some(Utc::now());
			self.prev_orphans_len = chain.orphans_len() + chain.orphans_evicted_len();
		}

		return false;
	}
}

fn body_sync(peers: Arc<Peers>, chain: Arc<chain::Chain>, body_sync_info: &mut BodySyncInfo) {
	let horizon = global::cut_through_horizon() as u64;
	let body_head: chain::Tip = chain.head().unwrap();
	let header_head: chain::Tip = chain.get_header_head().unwrap();
	let sync_head: chain::Tip = chain.get_sync_head().unwrap();

	body_sync_info.reset();

	debug!(
		LOGGER,
		"body_sync: body_head - {}, {}, header_head - {}, {}, sync_head - {}, {}",
		body_head.last_block_h,
		body_head.height,
		header_head.last_block_h,
		header_head.height,
		sync_head.last_block_h,
		sync_head.height,
	);

	let mut hashes = vec![];
	let mut oldest_height = 0;

	if header_head.total_difficulty > body_head.total_difficulty {
		let mut current = chain.get_block_header(&header_head.last_block_h);
		while let Ok(header) = current {
			// break out of the while loop when we find a header common
			// between the header chain and the current body chain
			if let Ok(_) = chain.is_on_current_chain(&header) {
				break;
			}

			hashes.push(header.hash());
			oldest_height = header.height;
			current = chain.get_block_header(&header.previous);
		}
	}
	hashes.reverse();

	// if we have 5 peers to sync from then ask for 50 blocks total (peer_count *
	// 10) max will be 80 if all 8 peers are advertising more work
	// also if the chain is already saturated with orphans, throttle
	let peer_count = peers.more_work_peers().len();
	let block_count = cmp::min(
		cmp::min(100, peer_count * 10),
		chain::MAX_ORPHAN_SIZE.saturating_sub(chain.orphans_len()) + 1,
	);

	let hashes_to_get = hashes
		.iter()
		.filter(|x| {
			// only ask for blocks that we have not yet processed
			// either successfully stored or in our orphan list
			!chain.get_block(x).is_ok() && !chain.is_orphan(x)
		}).take(block_count)
		.collect::<Vec<_>>();

	if hashes_to_get.len() > 0 {
		debug!(
			LOGGER,
			"block_sync: {}/{} requesting blocks {:?} from {} peers",
			body_head.height,
			header_head.height,
			hashes_to_get,
			peer_count,
		);

		for hash in hashes_to_get.clone() {
			// only archival  peers can be expected to have blocks older than horizon
			let peer = if oldest_height < header_head.height.saturating_sub(horizon) {
				peers.more_work_archival_peer()
			} else {
				peers.more_work_peer()
			};
			if let Some(peer) = peer {
				if let Ok(peer) = peer.try_read() {
					if let Err(e) = peer.send_block_request(*hash) {
						debug!(LOGGER, "Skipped request to {}: {:?}", peer.info.addr, e);
					} else {
						body_sync_info.body_sync_hashes.push(hash.clone());
					}
				}
			}
		}
	}

	body_sync_info.reset_start(chain);
}

fn header_sync(
	peers: Arc<Peers>,
	chain: Arc<chain::Chain>,
	history_locators: &mut Vec<(u64, Hash)>,
) {
	if let Ok(header_head) = chain.get_header_head() {
		let difficulty = header_head.total_difficulty;

		if let Some(peer) = peers.most_work_peer() {
			if let Ok(p) = peer.try_read() {
				let peer_difficulty = p.info.total_difficulty.clone();
				if peer_difficulty > difficulty {
					request_headers(&p, chain.clone(), history_locators);
				}
			}
		}
	}
}

fn fast_sync(
	peers: Arc<Peers>,
	chain: Arc<chain::Chain>,
	header_head: &chain::Tip,
) -> Result<Arc<RwLock<Peer>>, p2p::Error> {
	let horizon = global::cut_through_horizon() as u64;

	if let Some(peer) = peers.most_work_peer() {
		if let Ok(p) = peer.try_read() {
			// ask for txhashset at 90% of horizon, this still leaves time for download
			// and validation to happen and stay within horizon
			let mut txhashset_head = chain.get_block_header(&header_head.prev_block_h).unwrap();
			for _ in 0..(horizon - horizon / 10) {
				txhashset_head = chain.get_block_header(&txhashset_head.previous).unwrap();
			}
			let bhash = txhashset_head.hash();
			debug!(
				LOGGER,
				"fast_sync: before txhashset request, header head: {} / {}, txhashset_head: {} / {}",
				header_head.height,
				header_head.last_block_h,
				txhashset_head.height,
				bhash
			);
			if let Err(e) = p.send_txhashset_request(txhashset_head.height, bhash) {
				error!(LOGGER, "fast_sync: send_txhashset_request err! {:?}", e);
				return Err(e);
			}
			return Ok(peer.clone());
		}
	}
	Err(p2p::Error::PeerException)
}

/// Request some block headers from a peer to advance us.
fn request_headers(peer: &Peer, chain: Arc<chain::Chain>, history_locators: &mut Vec<(u64, Hash)>) {
	if let Ok(locator) = get_locator(chain, history_locators) {
		debug!(
			LOGGER,
			"sync: request_headers: asking {} for headers, {:?}", peer.info.addr, locator,
		);

		let _ = peer.send_header_request(locator);
	}
}

/// Whether we're currently syncing the chain or we're fully caught up and
/// just receiving blocks through gossip.
fn needs_syncing(
	sync_state: &SyncState,
	peers: Arc<Peers>,
	chain: Arc<chain::Chain>,
) -> (bool, u64) {
	let local_diff = chain.total_difficulty();
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

/// We build a locator based on sync_head.
/// Even if sync_head is significantly out of date we will "reset" it once we
/// start getting headers back from a peer.
///
fn get_locator(
	chain: Arc<chain::Chain>,
	history_locators: &mut Vec<(u64, Hash)>,
) -> Result<Vec<Hash>, Error> {
	let mut this_height = 0;

	let tip = chain.get_sync_head()?;
	let heights = get_locator_heights(tip.height);
	let mut new_heights: Vec<u64> = vec![];

	// for security, clear history_locators[] in any case of header chain rollback,
	// the easiest way is to check whether the sync head and the header head are identical.
	if history_locators.len() > 0 && tip.hash() != chain.get_header_head()?.hash() {
		history_locators.clear();
	}

	debug!(LOGGER, "sync: locator heights : {:?}", heights);

	let mut locator: Vec<Hash> = vec![];
	let mut current = chain.get_block_header(&tip.last_block_h);
	while let Ok(header) = current {
		if heights.contains(&header.height) {
			locator.push(header.hash());
			new_heights.push(header.height);
			if history_locators.len() > 0
				&& tip.height - header.height + 1 >= p2p::MAX_BLOCK_HEADERS as u64 - 1
			{
				this_height = header.height;
				break;
			}
		}
		current = chain.get_block_header(&header.previous);
	}

	// update history locators
	{
		let mut tmp: Vec<(u64, Hash)> = vec![];
		*&mut tmp = new_heights
			.clone()
			.into_iter()
			.zip(locator.clone().into_iter())
			.collect();
		tmp.reverse();
		if history_locators.len() > 0 && tmp[0].0 == 0 {
			tmp = tmp[1..].to_vec();
		}
		history_locators.append(&mut tmp);
	}

	// reuse remaining part of locator from history
	if this_height > 0 {
		let this_height_index = heights.iter().position(|&r| r == this_height).unwrap();
		let next_height = heights[this_height_index + 1];

		let reuse_index = history_locators
			.iter()
			.position(|&r| r.0 >= next_height)
			.unwrap();
		let mut tmp = history_locators[..reuse_index + 1].to_vec();
		tmp.reverse();
		for (height, hash) in &mut tmp {
			if *height == 0 {
				break;
			}

			// check the locator to make sure the gap >= 2^n, where n = index of heights Vec
			if this_height >= *height + 2u64.pow(locator.len() as u32) {
				locator.push(hash.clone());
				this_height = *height;
				new_heights.push(this_height);
			}
			if locator.len() >= (p2p::MAX_LOCATORS as usize) - 1 {
				break;
			}
		}

		// push height 0 if it's not there
		if new_heights[new_heights.len() - 1] != 0 {
			locator.push(history_locators[history_locators.len() - 1].1.clone());
			new_heights.push(0);
		}
	}

	debug!(LOGGER, "sync: locator heights': {:?}", new_heights);

	// shrink history_locators properly
	if heights.len() > 1 {
		let shrink_height = heights[heights.len() - 2];
		let mut shrunk_size = 0;
		let shrink_index = history_locators
			.iter()
			.position(|&r| r.0 > shrink_height)
			.unwrap();
		if shrink_index > 100 {
			// shrink but avoid trivial shrinking
			let mut shrunk = history_locators[shrink_index..].to_vec();
			shrunk_size = shrink_index;
			history_locators.clear();
			history_locators.push((0, locator[locator.len() - 1]));
			history_locators.append(&mut shrunk);
		}
		debug!(
			LOGGER,
			"sync: history locators: len={}, shrunk={}",
			history_locators.len(),
			shrunk_size
		);
	}

	debug!(LOGGER, "sync: locator: {:?}", locator);

	Ok(locator)
}

// current height back to 0 decreasing in powers of 2
fn get_locator_heights(height: u64) -> Vec<u64> {
	let mut current = height;
	let mut heights = vec![];
	while current > 0 {
		heights.push(current);
		if heights.len() >= (p2p::MAX_LOCATORS as usize) - 1 {
			break;
		}
		let next = 2u64.pow(heights.len() as u32);
		current = if current > next { current - next } else { 0 }
	}
	heights.push(0);
	heights
}

// Utility struct to group what information the main sync loop has to track
struct SyncInfo {
	prev_body_sync: (DateTime<Utc>, u64),
	prev_header_sync: (DateTime<Utc>, u64, u64),
	prev_fast_sync: Option<DateTime<Utc>>,
	fast_sync_peer: Option<Arc<RwLock<Peer>>>,
	highest_height: u64,
}

impl SyncInfo {
	fn new() -> SyncInfo {
		let now = Utc::now();
		SyncInfo {
			prev_body_sync: (now.clone(), 0),
			prev_header_sync: (now.clone(), 0, 0),
			prev_fast_sync: None,
			fast_sync_peer: None,
			highest_height: 0,
		}
	}

	fn header_sync_due(&mut self, sync_state: &SyncState, header_head: &chain::Tip) -> bool {
		let now = Utc::now();
		let (timeout, latest_height, prev_height) = self.prev_header_sync;

		// received all necessary headers, can ask for more
		let all_headers_received =
			header_head.height >= prev_height + (p2p::MAX_BLOCK_HEADERS as u64) - 4;
		// no headers processed and we're past timeout, need to ask for more
		let stalling = header_head.height == latest_height && now > timeout;

		// always enable header sync on initial state transition from NoSync / Initial
		let force_sync = match sync_state.status() {
			SyncStatus::NoSync | SyncStatus::Initial => true,
			_ => false,
		};

		if force_sync || all_headers_received || stalling {
			self.prev_header_sync = (
				now + Duration::seconds(10),
				header_head.height,
				header_head.height,
			);
			true
		} else {
			// resetting the timeout as long as we progress
			if header_head.height > latest_height {
				self.prev_header_sync =
					(now + Duration::seconds(2), header_head.height, prev_height);
			}
			false
		}
	}

	fn body_sync_due(
		&mut self,
		head: &chain::Tip,
		chain: Arc<chain::Chain>,
		body_sync_info: &mut BodySyncInfo,
	) -> bool {
		let now = Utc::now();
		let (prev_ts, prev_height) = self.prev_body_sync;

		if head.height >= prev_height + 96
			|| now - prev_ts > Duration::seconds(5)
			|| body_sync_info.body_no_more(chain)
		{
			self.prev_body_sync = (now, head.height);
			return true;
		}
		false
	}

	// For now this is a one-time thing (it can be slow) at initial startup.
	fn fast_sync_due(&mut self) -> (bool, bool) {
		let now = Utc::now();
		let mut download_timeout = false;

		match self.prev_fast_sync {
			None => {
				self.prev_fast_sync = Some(now);
				(true, download_timeout)
			}
			Some(prev) => {
				if now - prev > Duration::minutes(10) {
					download_timeout = true;
				}
				(false, download_timeout)
			}
		}
	}

	fn fast_sync_reset(&mut self) {
		self.prev_fast_sync = None;
		self.fast_sync_peer = None;
	}
}

#[cfg(test)]
mod test {
	use super::*;

	#[test]
	fn test_get_locator_heights() {
		assert_eq!(get_locator_heights(0), vec![0]);
		assert_eq!(get_locator_heights(1), vec![1, 0]);
		assert_eq!(get_locator_heights(2), vec![2, 0]);
		assert_eq!(get_locator_heights(3), vec![3, 1, 0]);
		assert_eq!(get_locator_heights(10), vec![10, 8, 4, 0]);
		assert_eq!(get_locator_heights(100), vec![100, 98, 94, 86, 70, 38, 0]);
		assert_eq!(
			get_locator_heights(1000),
			vec![1000, 998, 994, 986, 970, 938, 874, 746, 490, 0]
		);
		// check the locator is still a manageable length, even for large numbers of
		// headers
		assert_eq!(
			get_locator_heights(10000),
			vec![10000, 9998, 9994, 9986, 9970, 9938, 9874, 9746, 9490, 8978, 7954, 5906, 1810, 0,]
		);
	}
}
