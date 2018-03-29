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

use cursive::Cursive;
use cursive::view::View;
use cursive::views::{BoxView, LinearLayout, TextView};
use cursive::direction::Orientation;
use cursive::traits::*;

use tui::constants::*;
use tui::types::*;

use grin::ServerStats;

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
				)
				.child(
					LinearLayout::new(Orientation::Horizontal)
						.child(TextView::new("Connected Peers: "))
						.child(TextView::new("0").with_id("connected_peers")),
				)
				.child(
					LinearLayout::new(Orientation::Horizontal)
						.child(TextView::new("Chain Height: "))
						.child(TextView::new("  ").with_id("chain_height")),
				)
				.child(
					LinearLayout::new(Orientation::Horizontal)
						.child(TextView::new("Total Difficulty: "))
						.child(TextView::new("  ").with_id("basic_total_difficulty")),
				)
				.child(
					LinearLayout::new(Orientation::Horizontal)
						.child(TextView::new("------------------------")),
				)
				.child(
					LinearLayout::new(Orientation::Horizontal)
						.child(TextView::new("  ").with_id("basic_mining_config_status")),
				)
				.child(
					LinearLayout::new(Orientation::Horizontal)
						.child(TextView::new("  ").with_id("basic_mining_status")),
				)
				.child(
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
			if stats.is_syncing {
				if stats.awaiting_peers {
					"Waiting for peers".to_string()
				} else {
					format!("Syncing - Latest header: {}", stats.header_head.height).to_string()
				}
			} else {
				"Running".to_string()
			}
		};
		let basic_mining_config_status = {
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
		};
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
		c.call_on_id("basic_mining_config_status", |t: &mut TextView| {
			t.set_content(basic_mining_config_status);
		});
		c.call_on_id("basic_mining_status", |t: &mut TextView| {
			t.set_content(basic_mining_status);
		});
		c.call_on_id("basic_network_info", |t: &mut TextView| {
			t.set_content(basic_network_info);
		});
	}
}
