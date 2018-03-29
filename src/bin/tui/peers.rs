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

use grin::stats::{PeerStats, ServerStats};

use cursive::Cursive;
use cursive::view::View;
use cursive::views::{BoxView, Dialog, LinearLayout, TextView};
use cursive::direction::Orientation;
use cursive::traits::*;

use tui::table::{TableView, TableViewItem};
use tui::constants::*;
use tui::types::*;

#[derive(Copy, Clone, PartialEq, Eq, Hash)]
enum PeerColumn {
	Address,
	State,
	TotalDifficulty,
	Direction,
	Version,
}

impl PeerColumn {
	fn _as_str(&self) -> &str {
		match *self {
			PeerColumn::Address => "Address",
			PeerColumn::State => "State",
			PeerColumn::Version => "Version",
			PeerColumn::TotalDifficulty => "Total Difficulty",
			PeerColumn::Direction => "Direction",
		}
	}
}

impl TableViewItem<PeerColumn> for PeerStats {
	fn to_column(&self, column: PeerColumn) -> String {
		match column {
			PeerColumn::Address => self.addr.clone(),
			PeerColumn::State => self.state.clone(),
			PeerColumn::TotalDifficulty => {
				format!("{} D @ {} H", self.total_difficulty, self.height).to_string()
			}
			PeerColumn::Direction => self.direction.clone(),
			PeerColumn::Version => self.version.to_string(),
		}
	}

	fn cmp(&self, other: &Self, column: PeerColumn) -> Ordering
	where
		Self: Sized,
	{
		match column {
			PeerColumn::Address => self.addr.cmp(&other.addr),
			PeerColumn::State => self.state.cmp(&other.state),
			PeerColumn::TotalDifficulty => self.total_difficulty.cmp(&other.total_difficulty),
			PeerColumn::Direction => self.direction.cmp(&other.direction),
			PeerColumn::Version => self.version.cmp(&other.version),
		}
	}
}

pub struct TUIPeerView;

impl TUIStatusListener for TUIPeerView {
	fn create() -> Box<View> {
		let table_view =
			TableView::<PeerStats, PeerColumn>::new()
				.column(PeerColumn::Address, "Address", |c| c.width_percent(20))
				.column(PeerColumn::State, "State", |c| c.width_percent(20))
				.column(PeerColumn::Direction, "Direction", |c| c.width_percent(20))
				.column(PeerColumn::TotalDifficulty, "Total Difficulty", |c| {
					c.width_percent(20)
				})
				.column(PeerColumn::Version, "Version", |c| c.width_percent(20));
		let peer_status_view = BoxView::with_full_screen(
			LinearLayout::new(Orientation::Vertical)
				.child(
					LinearLayout::new(Orientation::Horizontal)
						.child(TextView::new("Total Peers: "))
						.child(TextView::new("  ").with_id("peers_total")),
				)
				.child(
					LinearLayout::new(Orientation::Horizontal)
						.child(TextView::new("Longest Chain: "))
						.child(TextView::new("  ").with_id("longest_work_peer")),
				)
				.child(TextView::new("   "))
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
