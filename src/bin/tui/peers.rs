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

//! TUI peer display

use std::cmp::Ordering;

use servers::{PeerStats, ServerStats};

use chrono::prelude::*;
use tui::humansize::{file_size_opts::CONVENTIONAL, FileSize};

use cursive::direction::Orientation;
use cursive::traits::{Boxable, Identifiable};
use cursive::view::View;
use cursive::views::{BoxView, Dialog, LinearLayout, TextView};
use cursive::Cursive;

use tui::constants::{TABLE_PEER_STATUS, VIEW_PEER_SYNC};
use tui::table::{TableView, TableViewItem};
use tui::types::TUIStatusListener;

#[derive(Copy, Clone, PartialEq, Eq, Hash)]
enum PeerColumn {
	Address,
	State,
	UsedBandwidth,
	TotalDifficulty,
	Direction,
	Version,
}

impl PeerColumn {
	fn _as_str(&self) -> &str {
		match *self {
			PeerColumn::Address => "Address",
			PeerColumn::State => "State",
			PeerColumn::UsedBandwidth => "Used bandwidth",
			PeerColumn::Version => "Version",
			PeerColumn::TotalDifficulty => "Total Difficulty",
			PeerColumn::Direction => "Direction",
		}
	}
}

impl TableViewItem<PeerColumn> for PeerStats {
	fn to_column(&self, column: PeerColumn) -> String {
		// Converts optional size to human readable size
		fn size_to_string(size: u64) -> String {
			size.file_size(CONVENTIONAL).unwrap_or("-".to_string())
		}

		match column {
			PeerColumn::Address => self.addr.clone(),
			PeerColumn::State => self.state.clone(),
			PeerColumn::UsedBandwidth => format!(
				"S: {}, R: {}",
				size_to_string(self.sent_bytes_per_sec),
				size_to_string(self.received_bytes_per_sec),
			).to_string(),
			PeerColumn::TotalDifficulty => format!(
				"{} D @ {} H ({}s)",
				self.total_difficulty,
				self.height,
				(Utc::now() - self.last_seen).num_seconds(),
			).to_string(),
			PeerColumn::Direction => self.direction.clone(),
			PeerColumn::Version => self.version.to_string(),
		}
	}

	fn cmp(&self, other: &Self, column: PeerColumn) -> Ordering
	where
		Self: Sized,
	{
		// Compares used bandwidth of two peers
		fn cmp_used_bandwidth(curr: &PeerStats, other: &PeerStats) -> Ordering {
			let curr_recv_bytes = curr.received_bytes_per_sec;
			let curr_sent_bytes = curr.sent_bytes_per_sec;
			let other_recv_bytes = other.received_bytes_per_sec;
			let other_sent_bytes = other.sent_bytes_per_sec;

			let curr_sum = curr_recv_bytes + curr_sent_bytes;
			let other_sum = other_recv_bytes + other_sent_bytes;

			curr_sum.cmp(&other_sum)
		};

		match column {
			PeerColumn::Address => self.addr.cmp(&other.addr),
			PeerColumn::State => self.state.cmp(&other.state),
			PeerColumn::UsedBandwidth => cmp_used_bandwidth(&self, &other),
			PeerColumn::TotalDifficulty => self.total_difficulty.cmp(&other.total_difficulty),
			PeerColumn::Direction => self.direction.cmp(&other.direction),
			PeerColumn::Version => self.version.cmp(&other.version),
		}
	}
}

pub struct TUIPeerView;

impl TUIStatusListener for TUIPeerView {
	fn create() -> Box<View> {
		let table_view = TableView::<PeerStats, PeerColumn>::new()
			.column(PeerColumn::Address, "Address", |c| c.width_percent(16))
			.column(PeerColumn::State, "State", |c| c.width_percent(8))
			.column(PeerColumn::UsedBandwidth, "Used bandwidth", |c| {
				c.width_percent(24)
			}).column(PeerColumn::Direction, "Direction", |c| c.width_percent(8))
			.column(PeerColumn::TotalDifficulty, "Total Difficulty", |c| {
				c.width_percent(24)
			}).column(PeerColumn::Version, "Version", |c| c.width_percent(16));
		let peer_status_view = BoxView::with_full_screen(
			LinearLayout::new(Orientation::Vertical)
				.child(
					LinearLayout::new(Orientation::Horizontal)
						.child(TextView::new("Total Peers: "))
						.child(TextView::new("  ").with_id("peers_total")),
				).child(
					LinearLayout::new(Orientation::Horizontal)
						.child(TextView::new("Longest Chain: "))
						.child(TextView::new("  ").with_id("longest_work_peer")),
				).child(TextView::new("   "))
				.child(
					Dialog::around(table_view.with_id(TABLE_PEER_STATUS).min_size((50, 20)))
						.title("Connected Peers"),
				),
		).with_id(VIEW_PEER_SYNC);
		Box::new(peer_status_view)
	}

	fn update(c: &mut Cursive, stats: &ServerStats) {
		let lp = stats
			.peer_stats
			.iter()
			.max_by(|x, y| x.total_difficulty.cmp(&y.total_difficulty));
		let lp_str = match lp {
			Some(l) => format!(
				"{} D @ {} H vs Us: {} D @ {} H",
				l.total_difficulty, l.height, stats.head.total_difficulty, stats.head.height
			).to_string(),
			None => "".to_string(),
		};
		let _ = c.call_on_id(
			TABLE_PEER_STATUS,
			|t: &mut TableView<PeerStats, PeerColumn>| {
				t.set_items(stats.peer_stats.clone());
			},
		);
		let _ = c.call_on_id("peers_total", |t: &mut TextView| {
			t.set_content(stats.peer_stats.len().to_string());
		});
		let _ = c.call_on_id("longest_work_peer", |t: &mut TextView| {
			t.set_content(lp_str);
		});
	}
}
