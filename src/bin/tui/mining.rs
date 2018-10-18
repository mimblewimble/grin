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

//! Mining status view definition

use std::cmp::Ordering;

use cursive::direction::Orientation;
use cursive::event::Key;
use cursive::traits::{Boxable, Identifiable};
use cursive::view::View;
use cursive::views::{
	BoxView, Button, Dialog, LinearLayout, OnEventView, Panel, StackView, TextView,
};
use cursive::Cursive;
use std::time;
use tui::chrono::prelude::{DateTime, NaiveDateTime, Utc};

use tui::constants::{
	MAIN_MENU, SUBMENU_MINING_BUTTON, TABLE_MINING_DIFF_STATUS, TABLE_MINING_STATUS, VIEW_MINING,
};
use tui::types::TUIStatusListener;

use servers::{DiffBlock, ServerStats, WorkerStats};
use tui::table::{TableView, TableViewItem};

#[derive(Copy, Clone, PartialEq, Eq, Hash)]
enum StratumWorkerColumn {
	Id,
	IsConnected,
	LastSeen,
	PowDifficulty,
	NumAccepted,
	NumRejected,
	NumStale,
}

impl StratumWorkerColumn {
	fn _as_str(&self) -> &str {
		match *self {
			StratumWorkerColumn::Id => "Worker ID",
			StratumWorkerColumn::IsConnected => "Connected",
			StratumWorkerColumn::LastSeen => "Last Seen",
			StratumWorkerColumn::PowDifficulty => "PowDifficulty",
			StratumWorkerColumn::NumAccepted => "Num Accepted",
			StratumWorkerColumn::NumRejected => "Num Rejected",
			StratumWorkerColumn::NumStale => "Num Stale",
		}
	}
}

impl TableViewItem<StratumWorkerColumn> for WorkerStats {
	fn to_column(&self, column: StratumWorkerColumn) -> String {
		let naive_datetime = NaiveDateTime::from_timestamp(
			self.last_seen
				.duration_since(time::UNIX_EPOCH)
				.unwrap()
				.as_secs() as i64,
			0,
		);
		let datetime: DateTime<Utc> = DateTime::from_utc(naive_datetime, Utc);

		match column {
			StratumWorkerColumn::Id => self.id.clone(),
			StratumWorkerColumn::IsConnected => self.is_connected.to_string(),
			StratumWorkerColumn::LastSeen => datetime.to_string(),
			StratumWorkerColumn::PowDifficulty => self.pow_difficulty.to_string(),
			StratumWorkerColumn::NumAccepted => self.num_accepted.to_string(),
			StratumWorkerColumn::NumRejected => self.num_rejected.to_string(),
			StratumWorkerColumn::NumStale => self.num_stale.to_string(),
		}
	}

	fn cmp(&self, _other: &Self, column: StratumWorkerColumn) -> Ordering
	where
		Self: Sized,
	{
		match column {
			StratumWorkerColumn::Id => Ordering::Equal,
			StratumWorkerColumn::IsConnected => Ordering::Equal,
			StratumWorkerColumn::LastSeen => Ordering::Equal,
			StratumWorkerColumn::PowDifficulty => Ordering::Equal,
			StratumWorkerColumn::NumAccepted => Ordering::Equal,
			StratumWorkerColumn::NumRejected => Ordering::Equal,
			StratumWorkerColumn::NumStale => Ordering::Equal,
		}
	}
}
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
enum DiffColumn {
	BlockNumber,
	PoWType,
	Difficulty,
	SecondaryScaling,
	Time,
	Duration,
}

impl DiffColumn {
	fn _as_str(&self) -> &str {
		match *self {
			DiffColumn::BlockNumber => "Block Number",
			DiffColumn::PoWType => "Type",
			DiffColumn::Difficulty => "Network Difficulty",
			DiffColumn::SecondaryScaling => "Sec. Scaling",
			DiffColumn::Time => "Block Time",
			DiffColumn::Duration => "Duration",
		}
	}
}

impl TableViewItem<DiffColumn> for DiffBlock {
	fn to_column(&self, column: DiffColumn) -> String {
		let naive_datetime = NaiveDateTime::from_timestamp(self.time as i64, 0);
		let datetime: DateTime<Utc> = DateTime::from_utc(naive_datetime, Utc);
		let pow_type = match self.is_secondary {
			true => String::from("Secondary"),
			false => String::from("Primary"),
		};

		match column {
			DiffColumn::BlockNumber => self.block_number.to_string(),
			DiffColumn::PoWType => pow_type,
			DiffColumn::Difficulty => self.difficulty.to_string(),
			DiffColumn::SecondaryScaling => self.secondary_scaling.to_string(),
			DiffColumn::Time => format!("{}", datetime).to_string(),
			DiffColumn::Duration => format!("{}s", self.duration).to_string(),
		}
	}

	fn cmp(&self, _other: &Self, column: DiffColumn) -> Ordering
	where
		Self: Sized,
	{
		match column {
			DiffColumn::BlockNumber => Ordering::Equal,
			DiffColumn::PoWType => Ordering::Equal,
			DiffColumn::Difficulty => Ordering::Equal,
			DiffColumn::SecondaryScaling => Ordering::Equal,
			DiffColumn::Time => Ordering::Equal,
			DiffColumn::Duration => Ordering::Equal,
		}
	}
}
/// Mining status view
pub struct TUIMiningView;

impl TUIStatusListener for TUIMiningView {
	/// Create the mining view
	fn create() -> Box<View> {
		let devices_button = Button::new_raw("Mining Server Status", |s| {
			let _ = s.call_on_id("mining_stack_view", |sv: &mut StackView| {
				let pos = sv.find_layer_from_id("mining_device_view").unwrap();
				sv.move_to_front(pos);
			});
		}).with_id(SUBMENU_MINING_BUTTON);
		let difficulty_button = Button::new_raw("Difficulty", |s| {
			let _ = s.call_on_id("mining_stack_view", |sv: &mut StackView| {
				let pos = sv.find_layer_from_id("mining_difficulty_view").unwrap();
				sv.move_to_front(pos);
			});
		});
		let mining_submenu = LinearLayout::new(Orientation::Horizontal)
			.child(Panel::new(devices_button))
			.child(Panel::new(difficulty_button));

		let mining_submenu = OnEventView::new(mining_submenu).on_pre_event(Key::Esc, move |c| {
			let _ = c.focus_id(MAIN_MENU);
		});

		let table_view = TableView::<WorkerStats, StratumWorkerColumn>::new()
			.column(StratumWorkerColumn::Id, "Worker ID", |c| {
				c.width_percent(10)
			}).column(StratumWorkerColumn::IsConnected, "Connected", |c| {
				c.width_percent(10)
			}).column(StratumWorkerColumn::LastSeen, "Last Seen", |c| {
				c.width_percent(20)
			}).column(StratumWorkerColumn::PowDifficulty, "Pow Difficulty", |c| {
				c.width_percent(10)
			}).column(StratumWorkerColumn::NumAccepted, "Num Accepted", |c| {
				c.width_percent(10)
			}).column(StratumWorkerColumn::NumRejected, "Num Rejected", |c| {
				c.width_percent(10)
			}).column(StratumWorkerColumn::NumStale, "Num Stale", |c| {
				c.width_percent(10)
			});

		let status_view = LinearLayout::new(Orientation::Vertical)
			.child(
				LinearLayout::new(Orientation::Horizontal)
					.child(TextView::new("  ").with_id("stratum_config_status")),
			).child(
				LinearLayout::new(Orientation::Horizontal)
					.child(TextView::new("  ").with_id("stratum_is_running_status")),
			).child(
				LinearLayout::new(Orientation::Horizontal)
					.child(TextView::new("  ").with_id("stratum_num_workers_status")),
			).child(
				LinearLayout::new(Orientation::Horizontal)
					.child(TextView::new("  ").with_id("stratum_block_height_status")),
			).child(
				LinearLayout::new(Orientation::Horizontal)
					.child(TextView::new("  ").with_id("stratum_network_difficulty_status")),
			).child(
				LinearLayout::new(Orientation::Horizontal)
					.child(TextView::new("  ").with_id("stratum_network_hashrate")),
			).child(
				LinearLayout::new(Orientation::Horizontal)
					.child(TextView::new("  ").with_id("stratum_edge_bits_status")),
			);

		let mining_device_view = LinearLayout::new(Orientation::Vertical)
			.child(status_view)
			.child(BoxView::with_full_screen(
				Dialog::around(table_view.with_id(TABLE_MINING_STATUS).min_size((50, 20)))
					.title("Mining Workers"),
			)).with_id("mining_device_view");

		let diff_status_view = LinearLayout::new(Orientation::Vertical)
			.child(
				LinearLayout::new(Orientation::Horizontal)
					.child(TextView::new("Tip Height: "))
					.child(TextView::new("").with_id("diff_cur_height")),
			).child(
				LinearLayout::new(Orientation::Horizontal)
					.child(TextView::new("Difficulty Adjustment Window: "))
					.child(TextView::new("").with_id("diff_adjust_window")),
			).child(
				LinearLayout::new(Orientation::Horizontal)
					.child(TextView::new("Average Block Time: "))
					.child(TextView::new("").with_id("diff_avg_block_time")),
			).child(
				LinearLayout::new(Orientation::Horizontal)
					.child(TextView::new("Average Difficulty: "))
					.child(TextView::new("").with_id("diff_avg_difficulty")),
			);

		let diff_table_view = TableView::<DiffBlock, DiffColumn>::new()
			.column(DiffColumn::BlockNumber, "Block Number", |c| {
				c.width_percent(15)
			}).column(DiffColumn::PoWType, "Type", |c| c.width_percent(10))
			.column(DiffColumn::Difficulty, "Network Difficulty", |c| {
				c.width_percent(15)
			}).column(DiffColumn::SecondaryScaling, "Sec. Scaling", |c| {
				c.width_percent(10)
			}).column(DiffColumn::Time, "Block Time", |c| c.width_percent(25))
			.column(DiffColumn::Duration, "Duration", |c| c.width_percent(25));

		let mining_difficulty_view = LinearLayout::new(Orientation::Vertical)
			.child(diff_status_view)
			.child(BoxView::with_full_screen(
				Dialog::around(
					diff_table_view
						.with_id(TABLE_MINING_DIFF_STATUS)
						.min_size((50, 20)),
				).title("Mining Difficulty Data"),
			)).with_id("mining_difficulty_view");

		let view_stack = StackView::new()
			.layer(mining_difficulty_view)
			.layer(mining_device_view)
			.with_id("mining_stack_view");

		let mining_view = LinearLayout::new(Orientation::Vertical)
			.child(mining_submenu)
			.child(view_stack);

		Box::new(mining_view.with_id(VIEW_MINING))
	}

	/// update
	fn update(c: &mut Cursive, stats: &ServerStats) {
		c.call_on_id("diff_cur_height", |t: &mut TextView| {
			t.set_content(stats.diff_stats.height.to_string());
		});
		c.call_on_id("diff_adjust_window", |t: &mut TextView| {
			t.set_content(stats.diff_stats.window_size.to_string());
		});
		let dur = time::Duration::from_secs(stats.diff_stats.average_block_time);
		c.call_on_id("diff_avg_block_time", |t: &mut TextView| {
			t.set_content(format!("{} Secs", dur.as_secs()).to_string());
		});
		c.call_on_id("diff_avg_difficulty", |t: &mut TextView| {
			t.set_content(stats.diff_stats.average_difficulty.to_string());
		});

		let mut diff_stats = stats.diff_stats.last_blocks.clone();
		diff_stats.reverse();
		let _ = c.call_on_id(
			TABLE_MINING_DIFF_STATUS,
			|t: &mut TableView<DiffBlock, DiffColumn>| {
				t.set_items(diff_stats);
			},
		);
		let stratum_stats = stats.stratum_stats.clone();
		let stratum_network_hashrate = format!(
			"Network Hashrate: {:.*}",
			2,
			stratum_stats.network_hashrate()
		);
		let worker_stats = stratum_stats.worker_stats;
		let stratum_enabled = format!("Mining server enabled: {}", stratum_stats.is_enabled);
		let stratum_is_running = format!("Mining server running: {}", stratum_stats.is_running);
		let stratum_num_workers = format!("Number of workers: {}", stratum_stats.num_workers);
		let stratum_block_height = format!("Solving Block Height: {}", stratum_stats.block_height);
		let stratum_network_difficulty =
			format!("Network Difficulty: {}", stratum_stats.network_difficulty);
		let stratum_edge_bits = format!("Cuckoo Size: {}", stratum_stats.edge_bits);

		c.call_on_id("stratum_config_status", |t: &mut TextView| {
			t.set_content(stratum_enabled);
		});
		c.call_on_id("stratum_is_running_status", |t: &mut TextView| {
			t.set_content(stratum_is_running);
		});
		c.call_on_id("stratum_num_workers_status", |t: &mut TextView| {
			t.set_content(stratum_num_workers);
		});
		c.call_on_id("stratum_block_height_status", |t: &mut TextView| {
			t.set_content(stratum_block_height);
		});
		c.call_on_id("stratum_network_difficulty_status", |t: &mut TextView| {
			t.set_content(stratum_network_difficulty);
		});
		c.call_on_id("stratum_network_hashrate", |t: &mut TextView| {
			t.set_content(stratum_network_hashrate);
		});
		c.call_on_id("stratum_edge_bits_status", |t: &mut TextView| {
			t.set_content(stratum_edge_bits);
		});
		let _ = c.call_on_id(
			TABLE_MINING_STATUS,
			|t: &mut TableView<WorkerStats, StratumWorkerColumn>| {
				t.set_items(worker_stats);
			},
		);
	}
}
