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

use chrono::prelude::{DateTime, Utc};
use chrono::Duration;
use rand::seq::{IteratorRandom, SliceRandom};
use std::collections::HashSet;
use std::sync::Arc;

use crate::chain::{
	self, pihd_params, types::PIHDHeaderSegmentContainer, HeaderSyncMode, SyncState, SyncStatus,
};
use crate::common::types::Error;
use crate::core::core::hash::Hash;
use crate::core::core::SegmentIdentifier;
use crate::core::pow::Difficulty;
use crate::p2p::{
	self, types::PeerAddr, types::ReasonForBan, Capabilities, Peer, PIHD_HEADER_SEGMENT_HEIGHT,
};

struct LegacyHeaderRequest {
	peer_addr: PeerAddr,
	height: u64,
	requested_at: DateTime<Utc>,
}

struct PIHDPeerFailure {
	peer_addr: PeerAddr,
	last_failure: DateTime<Utc>,
	cooldown_until: DateTime<Utc>,
	failure_count: usize,
}

pub struct HeaderSync {
	sync_state: Arc<SyncState>,
	peers: Arc<p2p::Peers>,
	chain: Arc<chain::Chain>,
	prev_header_sync: (DateTime<Utc>, u64, u64),
	syncing_peer: Option<Arc<Peer>>,
	stalling_ts: Option<DateTime<Utc>>,
	pending_legacy: Option<LegacyHeaderRequest>,
	pihd_failure_count: usize,
	pihd_peer_failures: Vec<PIHDPeerFailure>,
	pihd_stalling_ts: Option<DateTime<Utc>>,
	pihd_disabled_until: Option<DateTime<Utc>>,
}

impl HeaderSync {
	pub fn new(
		sync_state: Arc<SyncState>,
		peers: Arc<p2p::Peers>,
		chain: Arc<chain::Chain>,
	) -> HeaderSync {
		HeaderSync {
			sync_state,
			peers,
			chain,
			prev_header_sync: (Utc::now(), 0, 0),
			syncing_peer: None,
			stalling_ts: None,
			pending_legacy: None,
			pihd_failure_count: 0,
			pihd_peer_failures: vec![],
			pihd_stalling_ts: None,
			pihd_disabled_until: None,
		}
	}

	pub fn check_run(&mut self, sync_head: chain::Tip) -> Result<bool, chain::Error> {
		// We only want to run header_sync for some sync states.
		let do_run = match self.sync_state.status() {
			SyncStatus::BodySync { .. }
			| SyncStatus::HeaderSync { .. }
			| SyncStatus::TxHashsetDone
			| SyncStatus::NoSync
			| SyncStatus::Initial
			| SyncStatus::AwaitingPeers(_) => true,
			_ => false,
		};

		if !do_run {
			return Ok(false);
		}

		self.cleanup_pending_requests(sync_head);

		if !self.header_sync_due(sync_head) {
			return Ok(false);
		}
		self.syncing_peer = None;

		let (pihd_peers, pihd_max_height, pihd_max_diff) = if self.pihd_enabled() {
			self.choose_pihd_peers(sync_head)
		} else {
			(vec![], 0, Difficulty::zero())
		};

		if pihd_peers.is_empty() {
			let sync_peer = self
				.pending_legacy
				.as_ref()
				.and_then(|req| self.peers.get_connected_peer(req.peer_addr))
				.filter(|peer| peer.info.total_difficulty() > sync_head.total_difficulty)
				.or_else(|| self.choose_sync_peer());
			if let Some(sync_peer) = sync_peer {
				let (peer_height, peer_diff) = {
					let info = sync_peer.info.live_info.read();
					(info.height, info.total_difficulty)
				};

				// Quick check - nothing to sync if we are caught up with the peer.
				if peer_diff <= sync_head.total_difficulty {
					if self.pihd_active() {
						info!(
							"sync: PIHD header sync completed at height {}, total difficulty {}",
							sync_head.height, sync_head.total_difficulty
						);
						self.sync_state.clear_pihd_header_segments();
					}
					return Ok(false);
				}

				if self.pihd_active() {
					info!(
							"sync: PIHD header sync aborted at height {}; falling back to legacy header sync",
							sync_head.height
						);
					self.sync_state.clear_pihd_header_segments();
				}

				self.sync_state.update(SyncStatus::HeaderSync {
					sync_head,
					sync_mode: HeaderSyncMode::Legacy,
					highest_height: peer_height,
					highest_diff: peer_diff,
				});
				if self.request_headers(sync_head, sync_peer.clone()) {
					self.syncing_peer = Some(sync_peer.clone());
				}
			} else if self.pihd_active() {
				info!(
					"sync: PIHD header sync aborted at height {}; no eligible PIHD peers",
					sync_head.height
				);
				self.sync_state.clear_pihd_header_segments();
			}
		} else {
			if !self.pihd_active() {
				info!(
					"sync: PIHD header sync started at height {} with {} eligible peer(s)",
					sync_head.height,
					pihd_peers.len()
				);
			}
			self.sync_state.update(SyncStatus::HeaderSync {
				sync_head,
				sync_mode: HeaderSyncMode::Pihd,
				highest_height: pihd_max_height,
				highest_diff: pihd_max_diff,
			});
			self.pihd_header_sync(sync_head, pihd_peers);
			self.syncing_peer = None;
		}
		Ok(true)
	}

	fn cleanup_pending_requests(&mut self, header_head: chain::Tip) {
		let now = Utc::now();
		let peers = self.peers.clone();
		if header_head.height > self.prev_header_sync.1 {
			self.pihd_failure_count = 0;
			self.pihd_stalling_ts = None;
		}

		// Returns conditions to retain pihd segments.
		let retain_pihd_segment_conditions =
			|req: &PIHDHeaderSegmentContainer| -> (bool, bool, bool) {
				let completed_height = p2p::pihd_header_segment_end_height(req.identifier)
					.unwrap_or(u64::MAX)
					.min(req.target_height);
				let connected = peers.get_connected_peer(PeerAddr(req.peer_addr)).is_some();
				let complete = header_head.height >= completed_height;
				let timeout = now
					> req.request_time
						+ Duration::seconds(pihd_params::PIHD_HEADER_REQUEST_TIMEOUT_SECS);
				(complete, connected, timeout)
			};

		let mut failed_peers = HashSet::new();
		self.sync_state.retain_pihd_header_segments(|req| {
			let (complete, connected, timeout) = retain_pihd_segment_conditions(req);
			if !complete && !req.responded {
				if connected && timeout {
					failed_peers.insert(PeerAddr(req.peer_addr));
				}
				if !connected {
					failed_peers.insert(PeerAddr(req.peer_addr));
				}
			}
			!complete && (req.responded || (connected && !timeout))
		});
		let rejected_peers = self.sync_state.take_rejected_pihd_peers();
		for peer_addr in rejected_peers {
			failed_peers.insert(PeerAddr(peer_addr));
		}
		if !failed_peers.is_empty() {
			self.pihd_failure_count += failed_peers.len();
			for peer_addr in failed_peers {
				let failure_count = self.note_pihd_peer_failure(peer_addr, now);
				if failure_count >= pihd_params::MAX_PEER_FAILURES_BEFORE_BLOCK {
					self.block_pihd_peer(peer_addr);
				}
			}
			if self.pihd_stalling_ts.is_none() {
				self.pihd_stalling_ts = Some(now);
			}
		}
		if self.pihd_failure_count > 0 {
			let pihd_stalled = self
				.pihd_stalling_ts
				.map(|stalling_ts| {
					now > stalling_ts + Duration::seconds(pihd_params::STALL_FALLBACK_SECS)
				})
				.unwrap_or(false);
			if self.pihd_failure_count >= pihd_params::MAX_TIMED_OUT_SEGMENTS && pihd_stalled {
				info!(
					"sync: disabling PIHD for {} seconds after {} failed header segment request(s) and {} seconds without header progress",
					pihd_params::DISABLE_SECS,
					self.pihd_failure_count,
					pihd_params::STALL_FALLBACK_SECS
				);
				if self.pihd_active() {
					info!(
							"sync: PIHD header sync aborted at height {}; failed {} header segment request(s), falling back to legacy header sync",
							header_head.height,
							self.pihd_failure_count
						);
				}
				self.sync_state.clear_pihd_header_segments();
				self.pihd_failure_count = 0;
				self.pihd_stalling_ts = None;
				self.pihd_disabled_until = Some(now + Duration::seconds(pihd_params::DISABLE_SECS));
			}
		}

		if let Some(req) = &self.pending_legacy {
			let connected = self.peers.get_connected_peer(req.peer_addr).is_some();
			let complete = header_head.height > req.height;
			let timed_out = now
				> req.requested_at
					+ Duration::seconds(pihd_params::LEGACY_HEADER_REQUEST_TIMEOUT_SECS);
			if complete || timed_out || !connected {
				self.pending_legacy = None;
			}
		}
	}

	fn note_pihd_peer_failure(&mut self, peer_addr: PeerAddr, now: DateTime<Utc>) -> usize {
		record_pihd_peer_failure(&mut self.pihd_peer_failures, peer_addr, now)
	}

	fn block_pihd_peer(&self, peer_addr: PeerAddr) {
		if self.peers.is_blocked(peer_addr) {
			debug!(
				"sync: peer {} already blocked after PIHD header segment failure",
				peer_addr
			);
			return;
		}

		let _ = self
			.peers
			.block_peer(peer_addr, "PIHD header segment failure");
		let is_outbound = self.peers.iter().outbound().by_addr(peer_addr).is_some();
		if is_outbound {
			debug!(
				"sync: disconnecting outbound peer {} after PIHD header segment failure",
				peer_addr
			);
			if let Err(e) = self
				.peers
				.disconnect_peer(peer_addr, "PIHD header segment failure")
			{
				debug!(
					"sync: failed to disconnect PIHD peer {} after header segment failure: {:?}",
					peer_addr, e
				);
			}
		}
	}

	fn pihd_peer_available(&self, peer_addr: PeerAddr, now: DateTime<Utc>) -> bool {
		!self
			.pihd_peer_failures
			.iter()
			.any(|failure| failure.peer_addr == peer_addr && failure.cooldown_until > now)
	}

	fn pihd_enabled(&mut self) -> bool {
		if let Some(disabled_until) = self.pihd_disabled_until {
			if Utc::now() < disabled_until {
				return false;
			}
			self.pihd_disabled_until = None;
		}
		true
	}

	fn pihd_active(&self) -> bool {
		if self
			.pihd_disabled_until
			.map(|disabled_until| Utc::now() < disabled_until)
			.unwrap_or(false)
		{
			return false;
		}

		matches!(
			self.sync_state.status(),
			SyncStatus::HeaderSync {
				sync_mode: HeaderSyncMode::Pihd,
				..
			}
		)
	}

	fn header_sync_due(&mut self, header_head: chain::Tip) -> bool {
		let now = Utc::now();
		let (timeout, latest_height, prev_height) = self.prev_header_sync;

		// received all necessary headers, can ask for more
		let all_headers_received =
			header_head.height >= prev_height + (p2p::MAX_BLOCK_HEADERS as u64) - 4;
		// no headers processed, and we're past timeout, need to ask for more
		let stalling = header_head.height <= latest_height && now > timeout;

		// always enable header sync on initial state transition from NoSync / Initial
		let force_sync = match self.sync_state.status() {
			SyncStatus::NoSync | SyncStatus::Initial | SyncStatus::AwaitingPeers(_) => true,
			_ => false,
		};

		if force_sync || all_headers_received || stalling {
			self.prev_header_sync = (
				now + Duration::seconds(10),
				header_head.height,
				header_head.height,
			);

			// save the stalling start time
			if stalling {
				if self.stalling_ts.is_none() {
					self.stalling_ts = Some(now);
				}
			} else {
				self.stalling_ts = None;
			}

			if all_headers_received {
				// reset the stalling start time if syncing goes well
				self.stalling_ts = None;
			} else if let Some(ref stalling_ts) = self.stalling_ts {
				if let Some(ref peer) = self.syncing_peer {
					match self.sync_state.status() {
						SyncStatus::HeaderSync { .. } | SyncStatus::BodySync { .. } => {
							// Ban this fraud peer which claims a higher work but can't send us the real headers
							if now > *stalling_ts + Duration::seconds(120)
								&& header_head.total_difficulty < peer.info.total_difficulty()
							{
								if let Err(e) = self
									.peers
									.ban_peer(peer.info.addr, ReasonForBan::FraudHeight)
								{
									error!("failed to ban peer {}: {:?}", peer.info.addr, e);
								}
								info!(
										"sync: ban a fraud peer: {}, claimed height: {}, total difficulty: {}",
										peer.info.addr,
										peer.info.height(),
										peer.info.total_difficulty(),
									);
							}
						}
						_ => (),
					}
					self.syncing_peer = None;
				}
			}
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

	fn choose_sync_peer(&self) -> Option<Arc<Peer>> {
		let peers_iter = || {
			self.peers
				.iter()
				.with_capabilities(Capabilities::HEADER_HIST)
				.connected()
		};

		// Filter peers further based on max difficulty.
		let max_diff = peers_iter().max_difficulty().unwrap_or(Difficulty::zero());
		let peers_iter = || peers_iter().with_difficulty(|x| x >= max_diff);

		// Choose a random "most work" peer, preferring outbound if at all possible.
		peers_iter().outbound().choose_random().or_else(|| {
			warn!("no suitable outbound peer for header sync, considering inbound");
			peers_iter().inbound().choose_random()
		})
	}

	fn choose_pihd_peers(&self, sync_head: chain::Tip) -> (Vec<Arc<Peer>>, u64, Difficulty) {
		let peers_iter = || {
			self.peers
				.iter()
				.with_capabilities(Capabilities::HEADER_HIST)
				.connected()
		};
		let candidates: Vec<_> = peers_iter()
			.with_capabilities(Capabilities::PIHD_HIST)
			.with_difficulty(|x| x > sync_head.total_difficulty)
			.with_filter(|p| p.info.height() > sync_head.height)
			.into_iter()
			.collect();
		let max_height = candidates
			.iter()
			.map(|p| p.info.height())
			.max()
			.unwrap_or(0);
		let max_diff = candidates
			.iter()
			.map(|p| p.info.total_difficulty())
			.max()
			.unwrap_or(Difficulty::zero());
		let mut rng = rand::thread_rng();
		let mut peers: Vec<_> = candidates
			.iter()
			.filter(|p| p.info.is_outbound())
			.cloned()
			.collect();
		peers.shuffle(&mut rng);
		let mut inbound: Vec<_> = candidates
			.into_iter()
			.filter(|p| p.info.is_inbound())
			.collect();
		inbound.shuffle(&mut rng);
		peers.extend(inbound);
		(peers, max_height, max_diff)
	}

	fn pihd_header_sync(&mut self, sync_head: chain::Tip, peers: Vec<Arc<Peer>>) {
		let now = Utc::now();
		let mut rng = rand::thread_rng();
		self.pihd_peer_failures
			.retain(|failure| retain_pihd_peer_failure(failure, now));
		let preferred_peers = peers
			.iter()
			.filter(|peer| {
				self.pihd_peer_available(peer.info.addr, now)
					&& !self.peers.is_blocked(peer.info.addr)
			})
			.cloned()
			.collect::<Vec<_>>();
		let available_peers = if preferred_peers.is_empty() {
			peers
				.iter()
				.filter(|peer| self.pihd_peer_available(peer.info.addr, now))
				.cloned()
				.collect::<Vec<_>>()
		} else {
			preferred_peers
		};
		let peers = if available_peers.is_empty() {
			peers
		} else {
			available_peers
		};
		if self.sync_state.pending_pihd_segments_count() >= pihd_params::MAX_IN_FLIGHT_SEGMENTS {
			return;
		}
		let mut sent = 0;
		let mut segment_idx = p2p::types::next_pihd_header_segment_idx(sync_head.height);
		while self.sync_state.pending_pihd_segments_count() < pihd_params::MAX_IN_FLIGHT_SEGMENTS
			&& sent < pihd_params::MAX_REQUESTS_PER_TICK
		{
			let identifier = SegmentIdentifier {
				height: PIHD_HEADER_SEGMENT_HEIGHT,
				idx: segment_idx,
			};
			let start_height = match p2p::pihd_header_segment_start_height(identifier) {
				Some(height) => height,
				None => return,
			};
			if self.sync_state.contains_pihd_header_segment(identifier) {
				segment_idx += 1;
				continue;
			}
			let can_request = |peer: &&Arc<Peer>, max_in_flight| {
				peer.info.height() >= start_height
					&& self
						.sync_state
						.pending_pihd_segments_count_from(peer.info.addr.0)
						< max_in_flight
			};
			let peer = peers
				.iter()
				.filter(|peer| peer.info.is_outbound())
				.filter(|peer| can_request(peer, pihd_params::MAX_IN_FLIGHT_SEGMENTS_PER_PEER))
				.choose(&mut rng)
				.cloned()
				.or_else(|| {
					peers
						.iter()
						.filter(|peer| peer.info.is_inbound())
						.filter(|peer| {
							can_request(peer, pihd_params::MAX_IN_FLIGHT_SEGMENTS_PER_PEER)
						})
						.choose(&mut rng)
						.cloned()
				})
				.or_else(|| {
					peers
						.iter()
						.filter(|peer| peer.info.is_outbound())
						.filter(|peer| can_request(peer, pihd_params::MAX_IN_FLIGHT_SEGMENTS))
						.choose(&mut rng)
						.cloned()
				})
				.or_else(|| {
					peers
						.iter()
						.filter(|peer| peer.info.is_inbound())
						.filter(|peer| can_request(peer, pihd_params::MAX_IN_FLIGHT_SEGMENTS))
						.choose(&mut rng)
						.cloned()
				});
			let peer = match peer {
				Some(peer) => peer,
				None => return,
			};
			debug!(
				"Ask header segment {:?} from {}",
				identifier, peer.info.addr
			);
			if peer.send_header_segment_request(identifier).is_ok() {
				let target_height = peer.info.height();
				self.sync_state.add_pihd_header_segment(
					identifier,
					peer.info.addr.0,
					target_height,
				);
				sent += 1;
			}
			segment_idx += 1;
		}
	}

	/// Request some block headers from a peer to advance us.
	fn request_headers(&mut self, sync_head: chain::Tip, peer: Arc<Peer>) -> bool {
		if let Some(req) = &self.pending_legacy {
			let pending_peer_addr = req.peer_addr;
			if pending_peer_addr == peer.info.addr {
				return self.peers.get_connected_peer(peer.info.addr).is_some();
			}
			if self.peers.get_connected_peer(pending_peer_addr).is_some() {
				return false;
			}
			self.pending_legacy = None;
		}
		if self.peers.get_connected_peer(peer.info.addr).is_none() {
			return false;
		}
		if let Ok(locator) = self.get_locator(sync_head) {
			debug!(
				"sync: request_headers: asking {} for headers, {:?}",
				peer.info.addr, locator,
			);

			if peer.send_header_request(locator).is_ok() {
				self.pending_legacy = Some(LegacyHeaderRequest {
					peer_addr: peer.info.addr,
					height: sync_head.height,
					requested_at: Utc::now(),
				});
				return true;
			}
		}
		false
	}

	/// Build a locator based on header_head.
	fn get_locator(&self, sync_head: chain::Tip) -> Result<Vec<Hash>, Error> {
		let heights = get_locator_heights(sync_head.height);
		let locator = self.chain.get_locator_hashes(sync_head, &heights)?;
		Ok(locator)
	}
}

fn retain_pihd_peer_failure(failure: &PIHDPeerFailure, now: DateTime<Utc>) -> bool {
	failure.last_failure + Duration::seconds(pihd_params::PEER_FAILURE_WINDOW_SECS) > now
		|| failure.cooldown_until > now
}

fn record_pihd_peer_failure(
	failures: &mut Vec<PIHDPeerFailure>,
	peer_addr: PeerAddr,
	now: DateTime<Utc>,
) -> usize {
	failures.retain(|failure| retain_pihd_peer_failure(failure, now));
	let cooldown_until = now + Duration::seconds(pihd_params::PEER_TIMEOUT_COOLDOWN_SECS);
	if let Some(failure) = failures
		.iter_mut()
		.find(|failure| failure.peer_addr == peer_addr)
	{
		failure.last_failure = now;
		failure.cooldown_until = cooldown_until;
		failure.failure_count = failure.failure_count.saturating_add(1);
		failure.failure_count
	} else {
		failures.push(PIHDPeerFailure {
			peer_addr,
			last_failure: now,
			cooldown_until,
			failure_count: 1,
		});
		1
	}
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

	#[test]
	fn test_pihd_segment_start_height() {
		assert_eq!(
			p2p::pihd_header_segment_capacity(),
			p2p::MAX_BLOCK_HEADERS as u64
		);
		assert_eq!(
			p2p::pihd_header_segment_start_height(SegmentIdentifier {
				height: PIHD_HEADER_SEGMENT_HEIGHT,
				idx: 0
			}),
			Some(1)
		);
		assert_eq!(
			p2p::pihd_header_segment_start_height(SegmentIdentifier {
				height: PIHD_HEADER_SEGMENT_HEIGHT,
				idx: 1
			}),
			Some(p2p::pihd_header_segment_capacity() + 1)
		);
		assert_eq!(
			p2p::pihd_header_segment_start_height(SegmentIdentifier {
				height: PIHD_HEADER_SEGMENT_HEIGHT,
				idx: u64::MAX
			}),
			None
		);
	}

	#[test]
	fn test_pihd_peer_failure_count() {
		let peer_addr = PeerAddr("127.0.0.1:13414".parse().unwrap());
		let now = Utc::now();
		let mut failures = vec![];

		assert_eq!(record_pihd_peer_failure(&mut failures, peer_addr, now), 1);
		assert_eq!(
			record_pihd_peer_failure(&mut failures, peer_addr, now + Duration::seconds(10)),
			2
		);
		assert_eq!(
			record_pihd_peer_failure(&mut failures, peer_addr, now + Duration::seconds(20)),
			3
		);
		assert_eq!(failures.len(), 1);
		assert_eq!(
			failures[0].cooldown_until,
			now + Duration::seconds(20 + pihd_params::PEER_TIMEOUT_COOLDOWN_SECS)
		);
	}

	#[test]
	fn test_pihd_peer_failure_expiry() {
		let peer_addr = PeerAddr("127.0.0.1:13414".parse().unwrap());
		let now = Utc::now();
		let old = now - Duration::seconds(pihd_params::PEER_FAILURE_WINDOW_SECS + 1);
		let mut failures = vec![PIHDPeerFailure {
			peer_addr,
			last_failure: old,
			cooldown_until: old + Duration::seconds(pihd_params::PEER_TIMEOUT_COOLDOWN_SECS),
			failure_count: 2,
		}];

		assert_eq!(record_pihd_peer_failure(&mut failures, peer_addr, now), 1);
		assert_eq!(failures.len(), 1);
		assert_eq!(failures[0].last_failure, now);
	}
}
