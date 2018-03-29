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

use cursive::Cursive;
use cursive::event::Key;
use cursive::view::View;
use cursive::views::{BoxView, Button, Dialog, LinearLayout, OnEventView, Panel, StackView,
                     TextView};
use cursive::direction::Orientation;
use cursive::traits::*;
use std::time;
use tui::chrono::prelude::*;

use tui::constants::*;
use tui::types::*;

use grin::stats::*;
use tui::pow::cuckoo_miner::CuckooMinerDeviceStats;
use tui::table::{TableView, TableViewItem};

#[derive(Copy, Clone, PartialEq, Eq, Hash)]
enum MiningDeviceColumn {
	PluginId,
	DeviceId,
	DeviceName,
	InUse,
	ErrorStatus,
	LastGraphTime,
	GraphsPerSecond,
}

impl MiningDeviceColumn {
	fn _as_str(&self) -> &str {
		match *self {
			MiningDeviceColumn::PluginId => "Plugin ID",
			MiningDeviceColumn::DeviceId => "Device ID",
			MiningDeviceColumn::DeviceName => "Name",
			MiningDeviceColumn::InUse => "In Use",
			MiningDeviceColumn::ErrorStatus => "Status",
			MiningDeviceColumn::LastGraphTime => "Last Graph Time",
			MiningDeviceColumn::GraphsPerSecond => "GPS",
		}
	}
}

impl TableViewItem<MiningDeviceColumn> for CuckooMinerDeviceStats {
	fn to_column(&self, column: MiningDeviceColumn) -> String {
		let last_solution_time_secs = self.last_solution_time as f64 / 1000000000.0;
		match column {
			MiningDeviceColumn::PluginId => String::from("TBD"),
			MiningDeviceColumn::DeviceId => self.device_id.clone(),
			MiningDeviceColumn::DeviceName => self.device_name.clone(),
			MiningDeviceColumn::InUse => match self.in_use {
				1 => String::from("Yes"),
				_ => String::from("No"),
			},
			MiningDeviceColumn::ErrorStatus => match self.has_errored {
				0 => String::from("OK"),
				_ => String::from("Errored"),
			},
			MiningDeviceColumn::LastGraphTime => {
				String::from(format!("{}s", last_solution_time_secs))
			}
			MiningDeviceColumn::GraphsPerSecond => {
				String::from(format!("{:.*}", 4, 1.0 / last_solution_time_secs))
			}
		}
	}

	fn cmp(&self, other: &Self, column: MiningDeviceColumn) -> Ordering
	where
		Self: Sized,
	{
		let last_solution_time_secs_self = self.last_solution_time as f64 / 1000000000.0;
		let gps_self = 1.0 / last_solution_time_secs_self;
		let last_solution_time_secs_other = other.last_solution_time as f64 / 1000000000.0;
		let gps_other = 1.0 / last_solution_time_secs_other;
		match column {
			MiningDeviceColumn::PluginId => Ordering::Equal,
			MiningDeviceColumn::DeviceId => self.device_id.cmp(&other.device_id),
			MiningDeviceColumn::DeviceName => self.device_name.cmp(&other.device_name),
			MiningDeviceColumn::InUse => self.in_use.cmp(&other.in_use),
			MiningDeviceColumn::ErrorStatus => self.has_errored.cmp(&other.has_errored),
			MiningDeviceColumn::LastGraphTime => {
				self.last_solution_time.cmp(&other.last_solution_time)
			}
			MiningDeviceColumn::GraphsPerSecond => gps_self.partial_cmp(&gps_other).unwrap(),
		}
	}
}

#[derive(Copy, Clone, PartialEq, Eq, Hash)]
enum DiffColumn {
	BlockNumber,
	Difficulty,
	Time,
	Duration,
}

impl DiffColumn {
	fn _as_str(&self) -> &str {
		match *self {
			DiffColumn::BlockNumber => "Block Number",
			DiffColumn::Difficulty => "Network Difficulty",
			DiffColumn::Time => "Block Time",
			DiffColumn::Duration => "Duration",
		}
	}
}

impl TableViewItem<DiffColumn> for DiffBlock {
	fn to_column(&self, column: DiffColumn) -> String {
		let naive_datetime = NaiveDateTime::from_timestamp(self.time as i64, 0);
		let datetime: DateTime<Utc> = DateTime::from_utc(naive_datetime, Utc);

		match column {
			DiffColumn::BlockNumber => self.block_number.to_string(),
			DiffColumn::Difficulty => self.difficulty.to_string(),
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
			DiffColumn::Difficulty => Ordering::Equal,
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
		let devices_button = Button::new_raw("Status / Devices", |s| {
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

		let table_view =
			TableView::<CuckooMinerDeviceStats, MiningDeviceColumn>::new()
				.column(MiningDeviceColumn::PluginId, "Plugin ID", |c| {
					c.width_percent(10)
				})
				.column(MiningDeviceColumn::DeviceId, "Device ID", |c| {
					c.width_percent(10)
				})
				.column(MiningDeviceColumn::DeviceName, "Device Name", |c| {
					c.width_percent(20)
				})
				.column(MiningDeviceColumn::InUse, "In Use", |c| c.width_percent(10))
				.column(MiningDeviceColumn::ErrorStatus, "Status", |c| {
					c.width_percent(10)
				})
				.column(MiningDeviceColumn::LastGraphTime, "Graph Time", |c| {
					c.width_percent(10)
				})
				.column(MiningDeviceColumn::GraphsPerSecond, "GPS", |c| {
					c.width_percent(10)
				});

		let status_view = LinearLayout::new(Orientation::Vertical)
			.child(
				LinearLayout::new(Orientation::Horizontal)
					.child(TextView::new("  ").with_id("mining_config_status")),
			)
			.child(
				LinearLayout::new(Orientation::Horizontal)
					.child(TextView::new("  ").with_id("mining_status")),
			)
			.child(
				LinearLayout::new(Orientation::Horizontal)
					.child(TextView::new("  ").with_id("network_info")),
			);

		let mining_device_view = LinearLayout::new(Orientation::Vertical)
			.child(status_view)
			.child(BoxView::with_full_screen(
				Dialog::around(table_view.with_id(TABLE_MINING_STATUS).min_size((50, 20)))
					.title("Mining Devices"),
			))
			.with_id("mining_device_view");

		let diff_status_view = LinearLayout::new(Orientation::Vertical)
			.child(
				LinearLayout::new(Orientation::Horizontal)
					.child(TextView::new("Tip Height: "))
					.child(TextView::new("").with_id("diff_cur_height")),
			)
			.child(
				LinearLayout::new(Orientation::Horizontal)
					.child(TextView::new("Difficulty Adjustment Window: "))
					.child(TextView::new("").with_id("diff_adjust_window")),
			)
			.child(
				LinearLayout::new(Orientation::Horizontal)
					.child(TextView::new("Average Block Time: "))
					.child(TextView::new("").with_id("diff_avg_block_time")),
			)
			.child(
				LinearLayout::new(Orientation::Horizontal)
					.child(TextView::new("Average Difficulty: "))
					.child(TextView::new("").with_id("diff_avg_difficulty")),
			);

		let diff_table_view = TableView::<DiffBlock, DiffColumn>::new()
			.column(DiffColumn::BlockNumber, "Block Number", |c| {
				c.width_percent(25)
			})
			.column(DiffColumn::Difficulty, "Network Difficulty", |c| {
				c.width_percent(25)
			})
			.column(DiffColumn::Time, "Block Time", |c| c.width_percent(25))
			.column(DiffColumn::Duration, "Duration", |c| c.width_percent(25));

		let mining_difficulty_view = LinearLayout::new(Orientation::Vertical)
			.child(diff_status_view)
			.child(BoxView::with_full_screen(
				Dialog::around(
					diff_table_view
						.with_id(TABLE_MINING_DIFF_STATUS)
						.min_size((50, 20)),
				).title("Mining Difficulty Data"),
			))
			.with_id("mining_difficulty_view");

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

		// device
		c.call_on_id("mining_config_status", |t: &mut TextView| {
			t.set_content(basic_mining_config_status);
		});
		c.call_on_id("mining_status", |t: &mut TextView| {
			t.set_content(basic_mining_status);
		});
		c.call_on_id("network_info", |t: &mut TextView| {
			t.set_content(basic_network_info);
		});

		//diff stats
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

		let mining_stats = stats.mining_stats.clone();
		let device_stats = mining_stats.device_stats;
		let mut diff_stats = stats.diff_stats.last_blocks.clone();
		diff_stats.reverse();
		let _ = c.call_on_id(
			TABLE_MINING_DIFF_STATUS,
			|t: &mut TableView<DiffBlock, DiffColumn>| {
				t.set_items(diff_stats);
			},
		);

		if device_stats.is_none() {
			return;
		}
		let device_stats = device_stats.unwrap();
		let mut flattened_device_stats = vec![];
		for p in device_stats.into_iter() {
			for d in p.into_iter() {
				flattened_device_stats.push(d);
			}
		}

		let _ = c.call_on_id(
			TABLE_MINING_STATUS,
			|t: &mut TableView<CuckooMinerDeviceStats, MiningDeviceColumn>| {
				t.set_items(flattened_device_stats);
			},
		);
	}
}
