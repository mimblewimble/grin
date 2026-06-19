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
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use crate::chain::{self, pibd_params, SyncState, SyncStatus};
use crate::core::core::{hash::Hashed, pmmr::segment::SegmentType};
use crate::core::global;
use crate::core::pow::Difficulty;
use crate::p2p::{self, Capabilities, Peer, PeerAddr};
use crate::util::StopState;

const PIBD_PROGRESS_CHECK_SECS: u64 = 10;

/// Fast sync has 3 "states":
/// * syncing headers
/// * once all headers are sync'd, requesting the txhashset state
/// * once we have the state, get blocks after that
///
/// The StateSync struct implements and monitors the middle step.
pub struct StateSync {
	sync_state: Arc<SyncState>,
	peers: Arc<p2p::Peers>,
	chain: Arc<chain::Chain>,

	prev_state_sync: Option<DateTime<Utc>>,
	state_sync_peer: Option<Arc<Peer>>,

	pibd_aborted: bool,
	earliest_zero_pibd_peer_time: Option<DateTime<Utc>>,
	last_pibd_progress_check: Option<Instant>,
}

impl StateSync {
	pub fn new(
		sync_state: Arc<SyncState>,
		peers: Arc<p2p::Peers>,
		chain: Arc<chain::Chain>,
	) -> StateSync {
		StateSync {
			sync_state,
			peers,
			chain,
			prev_state_sync: None,
			state_sync_peer: None,
			pibd_aborted: false,
			earliest_zero_pibd_peer_time: None,
			last_pibd_progress_check: None,
		}
	}

	/// Record earliest time at which we had no suitable
	/// peers for continuing PIBD
	pub fn set_earliest_zero_pibd_peer_time(&mut self, t: Option<DateTime<Utc>>) {
		self.earliest_zero_pibd_peer_time = t;
	}

	/// Flag to abort PIBD process within StateSync, intentionally separate from `sync_state`,
	/// which can be reset between calls
	pub fn set_pibd_aborted(&mut self) {
		self.pibd_aborted = true;
	}

	/// Check whether state sync should run and triggers a state download when
	/// it's time (we have all headers). Returns true as long as state sync
	/// needs monitoring, false when it's either done or turned off.
	pub fn check_run(
		&mut self,
		header_head: &chain::Tip,
		_head: &chain::Tip,
		_tail: &chain::Tip,
		highest_height: u64,
		stop_state: Arc<StopState>,
	) -> bool {
		let mut sync_need_restart = false;

		// check sync error
		if let Some(sync_error) = self.sync_state.sync_error() {
			error!("state_sync: error = {}. restart fast sync", sync_error);
			sync_need_restart = true;
		}

		// Determine whether we're going to try using PIBD or whether we've already given up
		// on it
		let using_pibd = !matches!(
			self.sync_state.status(),
			SyncStatus::TxHashsetPibd { aborted: true, .. },
		) && !self.pibd_aborted;

		// Check whether we've errored and should restart pibd
		if using_pibd {
			if let SyncStatus::TxHashsetPibd { errored: true, .. } = self.sync_state.status() {
				let archive_header = self.chain.txhashset_archive_header_header_only().unwrap();
				error!("PIBD Reported Failure - Restarting Sync");
				// reset desegmenter state
				let desegmenter = self.chain.desegmenter(&archive_header).unwrap();

				if let Some(d) = desegmenter.write().as_mut() {
					d.reset();
				};
				if let Err(e) = self.chain.reset_pibd_head() {
					error!("pibd_sync restart: reset pibd_head error = {}", e);
				}
				if let Err(e) = self.chain.reset_chain_head_to_genesis() {
					error!("pibd_sync restart: chain reset to genesis error = {}", e);
				}
				if let Err(e) = self.chain.reset_prune_lists() {
					error!("pibd_sync restart: reset prune lists error = {}", e);
				}
				self.sync_state
					.update_pibd_progress(false, false, 0, 1, &archive_header);
				sync_need_restart = true;
			}
		}

		// check peer connection status of this sync
		if !using_pibd {
			if let Some(ref peer) = self.state_sync_peer {
				if let SyncStatus::TxHashsetDownload { .. } = self.sync_state.status() {
					if !peer.is_connected() {
						sync_need_restart = true;
						info!(
							"state_sync: peer connection lost: {:?}. restart",
							peer.info.addr,
						);
					}
				}
			}
		}

		// if txhashset downloaded and validated successfully, we switch to BodySync state,
		// and we need call state_sync_reset() to make it ready for next possible state sync.
		let done = self.sync_state.update_if(
			SyncStatus::BodySync {
				current_height: 0,
				highest_height: 0,
			},
			|s| match s {
				SyncStatus::TxHashsetDone => true,
				_ => false,
			},
		);

		if sync_need_restart || done {
			self.state_sync_reset();
			self.sync_state.clear_sync_error();
		}

		if done {
			return false;
		}

		// run fast sync if applicable, normally only run one-time, except restart in error
		if sync_need_restart || header_head.height == highest_height {
			if using_pibd {
				if sync_need_restart {
					return true;
				}
				let (launch, _download_timeout) = self.state_sync_due();
				let archive_header = { self.chain.txhashset_archive_header_header_only().unwrap() };
				if launch {
					info!(
						"state_sync: PIBD started for archive header {} at height {}",
						archive_header.hash(),
						archive_header.height
					);
					self.sync_state
						.update_pibd_progress(false, false, 0, 1, &archive_header);
					self.last_pibd_progress_check = Some(Instant::now());
				}
				// Continue our PIBD process (which returns true if all segments are in)
				if self.continue_pibd() {
					let desegmenter = self.chain.desegmenter(&archive_header).unwrap();
					// All segments in, validate
					if let Some(d) = desegmenter.write().as_mut() {
						if let Ok(true) = d.check_progress(self.sync_state.clone()) {
							info!(
								"state_sync: PIBD segments downloaded for archive header {} at height {}; validating final txhashset",
								archive_header.hash(),
								archive_header.height
							);
							if let Err(e) = d.check_update_leaf_set_state() {
								error!("error updating PIBD leaf set: {}", e);
								self.sync_state.update_pibd_progress(
									false,
									true,
									0,
									1,
									&archive_header,
								);
								return false;
							}
							if let Err(e) = d.validate_complete_state(
								self.sync_state.clone(),
								stop_state.clone(),
							) {
								error!("error validating PIBD state: {}", e);
								self.sync_state.update_pibd_progress(
									false,
									true,
									0,
									1,
									&archive_header,
								);
								return false;
							}
							info!(
								"state_sync: PIBD completed for archive header {} at height {}",
								archive_header.hash(),
								archive_header.height
							);
							return true;
						}
					};
				}
			} else {
				let (go, download_timeout) = self.state_sync_due();

				if let SyncStatus::TxHashsetDownload { .. } = self.sync_state.status() {
					if download_timeout {
						error!("state_sync: TxHashsetDownload status timeout in 10 minutes!");
						self.sync_state
							.set_sync_error(chain::Error::SyncError(format!(
								"{:?}",
								p2p::Error::Timeout
							)));
					}
				}

				if go {
					self.state_sync_peer = None;
					match self.request_state(&header_head) {
						Ok(peer) => {
							self.state_sync_peer = Some(peer);
						}
						Err(e) => self
							.sync_state
							.set_sync_error(chain::Error::SyncError(format!("{:?}", e))),
					}

					self.sync_state
						.update(SyncStatus::TxHashsetDownload(Default::default()));
				}
			}
		}
		true
	}

	/// Continue the PIBD process, returning true if the desegmenter is reporting
	/// that the process is done
	fn continue_pibd(&mut self) -> bool {
		// Check the state of our chain to figure out what we should be requesting next
		let archive_header = self.chain.txhashset_archive_header_header_only().unwrap();
		let desegmenter = self.chain.desegmenter(&archive_header).unwrap();

		// Remove stale requests, if we haven't received the segment in time re-request
		let stale_segments = self
			.sync_state
			.remove_stale_pibd_requests(pibd_params::SEGMENT_REQUEST_TIMEOUT_SECS);
		if !stale_segments.is_empty() {
			let stale_peers: HashSet<_> = stale_segments
				.iter()
				.filter_map(|(_, peer_addr)| peer_addr.map(PeerAddr))
				.collect();
			for peer_addr in stale_peers {
				// TODO: Consider retry-only exclusion first, and block after repeated PIBD timeouts.
				let _ = self.peers.block_peer(peer_addr, "PIBD segment timeout");
				let is_outbound = self.peers.iter().outbound().by_addr(peer_addr).is_some();
				if is_outbound {
					debug!(
						"state_sync: disconnecting outbound peer {} after PIBD timeout",
						peer_addr
					);
					if let Err(e) = self
						.peers
						.disconnect_peer(peer_addr, "PIBD segment timeout")
					{
						debug!(
							"state_sync: failed to disconnect timed-out peer {}: {:?}",
							peer_addr, e
						);
					}
				}
			}
		}

		let progress_check_due = self
			.last_pibd_progress_check
			.map(|last| last.elapsed().as_secs() >= PIBD_PROGRESS_CHECK_SECS)
			.unwrap_or(true);
		let mut progress_check_done = false;

		// Apply segments... TODO: figure out how this should be called, might
		// need to be a separate thread.
		if let Some(mut de) = desegmenter.try_write() {
			if let Some(d) = de.as_mut() {
				let res = d.apply_next_segments();
				if let Err(e) = res {
					error!("error applying segment: {}", e);
					self.sync_state
						.update_pibd_progress(false, true, 0, 1, &archive_header);
					return false;
				}
				self.sync_state
					.update_pibd_leaf_progress(d.applied_leaf_count(), &archive_header);
				if progress_check_due {
					self.last_pibd_progress_check = Some(Instant::now());
					progress_check_done = true;
					match d.check_progress(self.sync_state.clone()) {
						Ok(true) => return true,
						Ok(false) => (),
						Err(e) => error!("state_sync: PIBD check_progress error: {}", e),
					}
				}
			}
		}

		let pending_segment_count = self.sync_state.pending_pibd_segment_count();
		if progress_check_due && !progress_check_done {
			if let Some(mut de) = desegmenter.try_write() {
				if let Some(d) = de.as_mut() {
					self.last_pibd_progress_check = Some(Instant::now());
					match d.check_progress(self.sync_state.clone()) {
						Ok(true) => return true,
						Ok(false) => (),
						Err(e) => error!("state_sync: PIBD check_progress error: {}", e),
					}
				}
			} else {
				trace!("state_sync: PIBD check_progress skipped, desegmenter busy");
			}
		}

		let request_budget =
			pibd_params::SEGMENT_REQUEST_COUNT.saturating_sub(pending_segment_count);

		let mut next_segment_ids = vec![];
		if request_budget > 0 {
			if let Some(mut de) = desegmenter.try_write() {
				if let Some(d) = de.as_mut() {
					// Figure out the next segments we need, looking past currently
					// pending requests so we can keep the request window full.
					next_segment_ids = d.next_desired_segments(
						pibd_params::SEGMENT_REQUEST_COUNT + pending_segment_count,
					);
					if !next_segment_ids.is_empty() {
						trace!(
							"state_sync: requesting {} PIBD segment candidate(s)",
							next_segment_ids.len()
						);
					} else {
						trace!("state_sync: no PIBD segments requested this loop");
					}
				}
			} else {
				trace!("state_sync: PIBD request scheduling skipped, desegmenter busy");
			}
		}

		// For each segment, pick a desirable peer and send message
		// (Provided we're not waiting for a response for this message from someone else)
		let mut sent_requests = 0;
		let mut request_candidates = vec![];

		let mut bitmap_candidates = vec![];
		let mut output_candidates = vec![];
		let mut rangeproof_candidates = vec![];
		let mut kernel_candidates = vec![];

		for seg_id in next_segment_ids.into_iter() {
			let excluded_peer = stale_segments
				.iter()
				.find(|(stale_id, _)| stale_id == &seg_id)
				.and_then(|(_, addr)| *addr);
			let candidate = (seg_id, excluded_peer);
			match candidate.0.segment_type {
				SegmentType::Bitmap => bitmap_candidates.push(candidate),
				SegmentType::Output => output_candidates.push(candidate),
				SegmentType::RangeProof => rangeproof_candidates.push(candidate),
				SegmentType::Kernel => kernel_candidates.push(candidate),
			}
		}

		bitmap_candidates.reverse();
		output_candidates.reverse();
		rangeproof_candidates.reverse();
		kernel_candidates.reverse();
		loop {
			let len_before = request_candidates.len();
			if let Some(candidate) = bitmap_candidates.pop() {
				request_candidates.push(candidate);
			}
			if let Some(candidate) = kernel_candidates.pop() {
				request_candidates.push(candidate);
			}
			if let Some(candidate) = output_candidates.pop() {
				request_candidates.push(candidate);
			}
			if let Some(candidate) = rangeproof_candidates.pop() {
				request_candidates.push(candidate);
			}
			if request_candidates.len() == len_before {
				break;
			}
		}

		let peers = self.peers.clone();
		let sync_state = self.sync_state.clone();
		for (seg_id, excluded_peer) in request_candidates.iter() {
			if sent_requests >= request_budget {
				continue;
			}
			if self.sync_state.contains_pibd_segment(seg_id) {
				trace!("Request list contains, continuing: {:?}", seg_id);
				continue;
			}

			// First, get max difficulty or greater peers
			let peers_iter = || peers.iter().connected();
			let max_diff = peers_iter().max_difficulty().unwrap_or(Difficulty::zero());
			let peers_iter_max = || peers_iter().with_difficulty(|x| x >= max_diff);

			// Then, further filter by PIBD capabilities v1
			let peers_iter_pibd = || {
				peers_iter_max()
					.with_capabilities(Capabilities::PIBD_HIST_1)
					.connected()
			};
			let height_slack = pibd_params::SYNC_PEER_HEIGHT_SLACK_BLOCKS;
			let max_pibd_height = peers_iter_pibd()
				.into_iter()
				.map(|p| p.info.height())
				.max()
				.unwrap_or(0);
			let available_pibd_peers = || {
				peers_iter_pibd().with_filter(|p| {
					p.info.height().saturating_add(height_slack) >= max_pibd_height
				})
			};

			// If there are no suitable PIBD-Enabled peers, AND there hasn't been one for a minute,
			// abort PIBD and fall back to txhashset download
			// Waiting a minute helps ensures that the cancellation isn't simply due to a single non-PIBD enabled
			// peer having the max difficulty
			if available_pibd_peers().count() == 0 {
				if let None = self.earliest_zero_pibd_peer_time {
					self.set_earliest_zero_pibd_peer_time(Some(Utc::now()));
				}
				if self.earliest_zero_pibd_peer_time.unwrap()
					+ Duration::seconds(pibd_params::TXHASHSET_ZIP_FALLBACK_TIME_SECS)
					< Utc::now()
				{
					info!(
						"state_sync: PIBD aborted for archive header {} at height {}; no PIBD-enabled max-difficulty peers for {} seconds, falling back to TxHashset.zip download",
						archive_header.hash(),
						archive_header.height,
						pibd_params::TXHASHSET_ZIP_FALLBACK_TIME_SECS
					);
					self.sync_state
						.update_pibd_progress(true, true, 0, 1, &archive_header);
					self.sync_state
						.set_sync_error(chain::Error::AbortingPIBDError);
					self.set_pibd_aborted();
					return false;
				}
			} else {
				self.set_earliest_zero_pibd_peer_time(None)
			}

			// Choose a random "most work" peer, excluding peer from stale/retry segment
			// and preferring outbound if at all possible.
			let peer = available_pibd_peers()
				.outbound()
				.with_filter(|p| {
					!peers.is_blocked(p.info.addr)
						&& !sync_state.rejected_pibd_segment_from(
							seg_id,
							p.info.addr.0,
							pibd_params::SEGMENT_REQUEST_TIMEOUT_SECS,
						)
				})
				.exclude(*excluded_peer)
				.choose_random()
				.or_else(|| {
					available_pibd_peers()
						.inbound()
						.with_filter(|p| {
							!peers.is_blocked(p.info.addr)
								&& !sync_state.rejected_pibd_segment_from(
									seg_id,
									p.info.addr.0,
									pibd_params::SEGMENT_REQUEST_TIMEOUT_SECS,
								)
						})
						.exclude(*excluded_peer)
						.choose_random()
						.or_else(|| {
							// If all otherwise eligible peers are blocked, keep sync moving.
							available_pibd_peers()
								.exclude(*excluded_peer)
								.choose_random()
						})
				});
			if let Some(p) = peer {
				// add to list of segments that are being tracked
				self.sync_state.add_pibd_segment(seg_id, p.info.addr.0);
				let res = match seg_id.segment_type {
					SegmentType::Bitmap => p.send_bitmap_segment_request(
						archive_header.hash(),
						seg_id.identifier.clone(),
					),
					SegmentType::Output => p.send_output_segment_request(
						archive_header.hash(),
						seg_id.identifier.clone(),
					),
					SegmentType::RangeProof => p.send_rangeproof_segment_request(
						archive_header.hash(),
						seg_id.identifier.clone(),
					),
					SegmentType::Kernel => p.send_kernel_segment_request(
						archive_header.hash(),
						seg_id.identifier.clone(),
					),
				};
				if let Err(e) = res {
					info!(
						"Error sending request to peer at {}, reason: {:?}",
						p.info.addr, e
					);
					self.sync_state.remove_pibd_segment(seg_id);
				} else if let Some(prev_peer) = excluded_peer {
					if p.info.addr.0 != *prev_peer {
						info!(
							"state_sync: retrying segment {:?} with new peer {} (previously {})",
							seg_id, p.info.addr, prev_peer
						);
					} else {
						debug!(
							"state_sync: requested segment {:?} from peer {}",
							seg_id, p.info.addr
						);
					}
				} else {
					debug!(
						"state_sync: requested segment {:?} from peer {}",
						seg_id, p.info.addr
					);
				}
				sent_requests += 1;
			}
		}
		false
	}

	fn request_state(&self, header_head: &chain::Tip) -> Result<Arc<Peer>, p2p::Error> {
		let threshold = global::state_sync_threshold() as u64;
		let archive_interval = global::txhashset_archive_interval();
		let mut txhashset_height = header_head.height.saturating_sub(threshold);
		txhashset_height = txhashset_height.saturating_sub(txhashset_height % archive_interval);

		let peers_iter = || {
			self.peers
				.iter()
				.with_capabilities(Capabilities::TXHASHSET_HIST)
				.connected()
		};

		// Filter peers further based on max difficulty.
		let max_diff = peers_iter().max_difficulty().unwrap_or(Difficulty::zero());
		let peers_iter = || peers_iter().with_difficulty(|x| x >= max_diff);

		// Choose a random "most work" peer, preferring outbound if at all possible.
		let peer = peers_iter().outbound().choose_random().or_else(|| {
			warn!("no suitable outbound peer for state sync, considering inbound");
			peers_iter().inbound().choose_random()
		});

		if let Some(peer) = peer {
			// ask for txhashset at state_sync_threshold
			let mut txhashset_head = self
				.chain
				.get_block_header(&header_head.prev_block_h)
				.map_err(|e| {
					error!(
						"chain error during getting a block header {}: {:?}",
						&header_head.prev_block_h, e
					);
					p2p::Error::Internal
				})?;
			while txhashset_head.height > txhashset_height {
				txhashset_head = self
					.chain
					.get_previous_header(&txhashset_head)
					.map_err(|e| {
						error!(
							"chain error during getting a previous block header {}: {:?}",
							txhashset_head.hash(),
							e
						);
						p2p::Error::Internal
					})?;
			}
			let bhash = txhashset_head.hash();
			debug!(
				"state_sync: before txhashset request, header head: {} / {}, txhashset_head: {} / {}",
				header_head.height,
				header_head.last_block_h,
				txhashset_head.height,
				bhash
			);
			if let Err(e) = peer.send_txhashset_request(txhashset_head.height, bhash) {
				error!("state_sync: send_txhashset_request err! {:?}", e);
				return Err(e);
			}
			return Ok(peer);
		}
		Err(p2p::Error::PeerException)
	}

	// For now this is a one-time thing (it can be slow) at initial startup.
	fn state_sync_due(&mut self) -> (bool, bool) {
		let now = Utc::now();
		let mut download_timeout = false;

		match self.prev_state_sync {
			None => {
				self.prev_state_sync = Some(now);
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

	fn state_sync_reset(&mut self) {
		let _ = self.peers.unblock_peers();
		self.prev_state_sync = None;
		self.state_sync_peer = None;
		self.last_pibd_progress_check = None;
	}
}
