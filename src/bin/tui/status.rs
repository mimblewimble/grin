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

//! Basic status view definition

use cursive::direction::Orientation;
use cursive::traits::Identifiable;
use cursive::view::View;
use cursive::views::{BoxView, LinearLayout, TextView};
use cursive::Cursive;

use tui::constants::VIEW_BASIC_STATUS;
use tui::types::TUIStatusListener;

use servers::common::types::SyncStatus;
use servers::ServerStats;

pub struct TUIStatusView;

impl TUIStatusListener for TUIStatusView {
	/// Create basic status view
	fn create() -> Box<View> {
		let basic_status_view = BoxView::with_full_screen(
			LinearLayout::new(Orientation::Vertical)
				.child(
					LinearLayout::new(Orientation::Horizontal)
						.child(TextView::new("Current Status: "))
						.child(TextView::new("Starting").with_id("basic_current_status")),
				).child(
					LinearLayout::new(Orientation::Horizontal)
						.child(TextView::new("Connected Peers: "))
						.child(TextView::new("0").with_id("connected_peers")),
				).child(
					LinearLayout::new(Orientation::Horizontal)
						.child(TextView::new("Chain Height: "))
						.child(TextView::new("  ").with_id("chain_height")),
				).child(
					LinearLayout::new(Orientation::Horizontal)
						.child(TextView::new("Total Difficulty: "))
						.child(TextView::new("  ").with_id("basic_total_difficulty")),
				).child(
					LinearLayout::new(Orientation::Horizontal)
						.child(TextView::new("------------------------")),
				).child(
					LinearLayout::new(Orientation::Horizontal)
						.child(TextView::new("  ").with_id("basic_mining_config_status")),
				).child(
					LinearLayout::new(Orientation::Horizontal)
						.child(TextView::new("  ").with_id("basic_mining_status")),
				).child(
					LinearLayout::new(Orientation::Horizontal)
						.child(TextView::new("  ").with_id("basic_network_info")),
				), //.child(logo_view)
		);
		Box::new(basic_status_view.with_id(VIEW_BASIC_STATUS))
	}

	/// update
	fn update(c: &mut Cursive, stats: &ServerStats) {
		//find and update here as needed
		let basic_status = {
			if stats.awaiting_peers {
				"Waiting for peers".to_string()
			} else {
				match stats.sync_status {
					SyncStatus::Initial => "Initializing".to_string(),
					SyncStatus::NoSync => "Running".to_string(),
					SyncStatus::HeaderSync {
						current_height,
						highest_height,
					} => {
						let percent = if highest_height == 0 {
							0
						} else {
							current_height * 100 / highest_height
						};
						format!("Downloading headers: {}%, step 1/4", percent)
					}
					SyncStatus::TxHashsetDownload => {
						"Downloading chain state for fast sync, step 2/4".to_string()
					}
					SyncStatus::TxHashsetSetup => {
						"Preparing chain state for validation, step 3/4".to_string()
					}
					SyncStatus::TxHashsetValidation {
						kernels,
						kernel_total,
						rproofs,
						rproof_total,
					} => {
						// 10% of overall progress is attributed to kernel validation
						// 90% to range proofs (which are much longer)
						let mut percent = if kernel_total > 0 {
							kernels * 10 / kernel_total
						} else {
							0
						};
						percent += if rproof_total > 0 {
							rproofs * 90 / rproof_total
						} else {
							0
						};
						format!("Validating chain state: {}%, step 3/4", percent)
					}
					SyncStatus::TxHashsetSave => {
						"Finalizing chain state for fast sync, step 3/4".to_string()
					}
					SyncStatus::BodySync {
						current_height,
						highest_height,
					} => {
						let percent = if highest_height == 0 {
							0
						} else {
							current_height * 100 / highest_height
						};
						format!("Downloading blocks: {}%, step 4/4", percent)
					}
				}
			}
		};
		/*let basic_mining_config_status = {
			if stats.mining_stats.is_enabled {
				"Configured as mining node"
			} else {
				"Configured as validating node only (not mining)"
			}
		};
		let (basic_mining_status, basic_network_info) = {
			if stats.mining_stats.is_enabled {
				if stats.is_syncing {
					(
						"Mining Status: Paused while syncing".to_string(),
						" ".to_string(),
					)
				} else if stats.mining_stats.combined_gps == 0.0 {
					(
						"Mining Status: Starting miner and awaiting first solution...".to_string(),
						" ".to_string(),
					)
				} else {
					(
						format!(
							"Mining Status: Mining at height {} at {:.*} GPS",
							stats.mining_stats.block_height, 4, stats.mining_stats.combined_gps
						),
						format!(
							"Cuckoo {} - Network Difficulty {}",
							stats.mining_stats.cuckoo_size,
							stats.mining_stats.network_difficulty.to_string()
						),
					)
				}
			} else {
				(" ".to_string(), " ".to_string())
			}
		};*/
		c.call_on_id("basic_current_status", |t: &mut TextView| {
			t.set_content(basic_status);
		});
		c.call_on_id("connected_peers", |t: &mut TextView| {
			t.set_content(stats.peer_count.to_string());
		});
		c.call_on_id("chain_height", |t: &mut TextView| {
			t.set_content(stats.head.height.to_string());
		});
		c.call_on_id("basic_total_difficulty", |t: &mut TextView| {
			t.set_content(stats.head.total_difficulty.to_string());
		});
		/*c.call_on_id("basic_mining_config_status", |t: &mut TextView| {
			t.set_content(basic_mining_config_status);
		});
		c.call_on_id("basic_mining_status", |t: &mut TextView| {
			t.set_content(basic_mining_status);
		});
		c.call_on_id("basic_network_info", |t: &mut TextView| {
			t.set_content(basic_network_info);
		});*/
	}
}
