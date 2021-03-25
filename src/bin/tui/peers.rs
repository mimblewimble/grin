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

//! TUI peer display

use std::cmp::Ordering;

use crate::servers::{PeerStats, ServerStats};

use chrono::prelude::*;
use humansize::{file_size_opts::CONVENTIONAL, FileSize};

use cursive::direction::Orientation;
use cursive::event::Key;
use cursive::traits::{Boxable, Identifiable};
use cursive::view::View;
use cursive::views::{Dialog, LinearLayout, OnEventView, ResizedView, TextView};
use cursive::Cursive;

use crate::tui::constants::{MAIN_MENU, TABLE_PEER_STATUS, VIEW_PEER_SYNC};
use crate::tui::types::TUIStatusListener;
use cursive_table_view::{TableView, TableViewItem};

#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub enum PeerColumn {
	Address,
	State,
	UsedBandwidth,
	TotalDifficulty,
	Direction,
	Version,
	UserAgent,
	Capabilities,
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
			PeerColumn::UserAgent => "User Agent",
			PeerColumn::Capabilities => "Capabilities",
		}
	}
}

impl TableViewItem<PeerColumn> for PeerStats {
	fn to_column(&self, column: PeerColumn) -> String {
		// Converts optional size to human readable size
		fn size_to_string(size: u64) -> String {
			size.file_size(CONVENTIONAL)
				.unwrap_or_else(|_| "-".to_string())
		}

		match column {
			PeerColumn::Address => self.addr.clone(),
			PeerColumn::State => self.state.clone(),
			PeerColumn::UsedBandwidth => format!(
				"↑: {}, ↓: {}",
				size_to_string(self.sent_bytes_per_sec),
				size_to_string(self.received_bytes_per_sec),
			),
			PeerColumn::TotalDifficulty => format!(
				"{} D @ {} H ({}s)",
				self.total_difficulty,
				self.height,
				(Utc::now() - self.last_seen).num_seconds(),
			),
			PeerColumn::Direction => self.direction.clone(),
			PeerColumn::Version => format!("{}", self.version),
			PeerColumn::UserAgent => self.user_agent.clone(),
			PeerColumn::Capabilities => format!("{}", self.capabilities.bits()),
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
		}

		let sort_by_addr = || self.addr.cmp(&other.addr);

		match column {
			PeerColumn::Address => sort_by_addr(),
			PeerColumn::State => self.state.cmp(&other.state).then(sort_by_addr()),
			PeerColumn::UsedBandwidth => cmp_used_bandwidth(&self, &other).then(sort_by_addr()),
			PeerColumn::TotalDifficulty => self
				.total_difficulty
				.cmp(&other.total_difficulty)
				.then(sort_by_addr()),
			PeerColumn::Direction => self.direction.cmp(&other.direction).then(sort_by_addr()),
			PeerColumn::Version => self.version.cmp(&other.version).then(sort_by_addr()),
			PeerColumn::UserAgent => self.user_agent.cmp(&other.user_agent).then(sort_by_addr()),
			PeerColumn::Capabilities => self
				.capabilities
				.cmp(&other.capabilities)
				.then(sort_by_addr()),
		}
	}
}

pub struct TUIPeerView;

impl TUIPeerView {
	pub fn create() -> impl View {
		let table_view = TableView::<PeerStats, PeerColumn>::new()
			.column(PeerColumn::Address, "Address", |c| c.width_percent(16))
			.column(PeerColumn::State, "State", |c| c.width_percent(8))
			.column(PeerColumn::UsedBandwidth, "Used bandwidth", |c| {
				c.width_percent(16)
			})
			.column(PeerColumn::Direction, "Direction", |c| c.width_percent(8))
			.column(PeerColumn::TotalDifficulty, "Total Difficulty", |c| {
				c.width_percent(24)
			})
			.column(PeerColumn::Version, "Proto", |c| c.width_percent(4))
			.column(PeerColumn::Capabilities, "Capab", |c| c.width_percent(4))
			.column(PeerColumn::UserAgent, "User Agent", |c| c.width_percent(18));
		let peer_status_view = ResizedView::with_full_screen(
			LinearLayout::new(Orientation::Vertical)
				.child(
					LinearLayout::new(Orientation::Horizontal)
						.child(TextView::new("  ").with_name("peers_total")),
				)
				.child(
					LinearLayout::new(Orientation::Horizontal)
						.child(TextView::new("Longest Chain: "))
						.child(TextView::new("  ").with_name("longest_work_peer")),
				)
				.child(TextView::new("   "))
				.child(
					Dialog::around(table_view.with_name(TABLE_PEER_STATUS).min_size((50, 20)))
						.title("Connected Peers"),
				),
		)
		.with_name(VIEW_PEER_SYNC);

		let peer_status_view =
			OnEventView::new(peer_status_view).on_pre_event(Key::Esc, move |c| {
				let _ = c.focus_name(MAIN_MENU);
			});

		peer_status_view
	}
}

impl TUIStatusListener for TUIPeerView {
	fn update(c: &mut Cursive, stats: &ServerStats) {
		let lp = stats
			.peer_stats
			.iter()
			.max_by(|x, y| x.total_difficulty.cmp(&y.total_difficulty));
		let lp_str = match lp {
			Some(l) => format!(
				"{} D @ {} H vs Us: {} D @ {} H",
				l.total_difficulty,
				l.height,
				stats.chain_stats.total_difficulty,
				stats.chain_stats.height
			),
			None => "".to_string(),
		};
		let _ = c.call_on_name(
			TABLE_PEER_STATUS,
			|t: &mut TableView<PeerStats, PeerColumn>| {
				t.set_items_stable(stats.peer_stats.clone());
			},
		);
		let _ = c.call_on_name("peers_total", |t: &mut TextView| {
			t.set_content(format!(
				"Total Peers: {} (Outbound: {})",
				stats.peer_stats.len(),
				stats
					.peer_stats
					.iter()
					.filter(|x| x.direction == "Outbound")
					.count(),
			));
		});
		let _ = c.call_on_name("longest_work_peer", |t: &mut TextView| {
			t.set_content(lp_str);
		});
	}
}
