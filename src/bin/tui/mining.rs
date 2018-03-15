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
use cursive::view::View;
use cursive::views::{BoxView, Dialog, LinearLayout, TextView};
use cursive::direction::Orientation;
use cursive::traits::*;

use tui::constants::*;
use tui::types::*;

use grin::types::ServerStats;
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

/// Mining status view
pub struct TUIMiningView;

impl TUIStatusListener for TUIMiningView {
	/// Create the mining view
	fn create() -> Box<View> {
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

		let mining_view = LinearLayout::new(Orientation::Vertical)
			.child(status_view)
			.child(BoxView::with_full_screen(
				Dialog::around(table_view.with_id(TABLE_MINING_STATUS).min_size((50, 20)))
					.title("Mining Devices"),
			));

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
						"Mining Status: Starting miner and awating first solution...".to_string(),
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

		c.call_on_id("mining_config_status", |t: &mut TextView| {
			t.set_content(basic_mining_config_status);
		});
		c.call_on_id("mining_status", |t: &mut TextView| {
			t.set_content(basic_mining_status);
		});
		c.call_on_id("network_info", |t: &mut TextView| {
			t.set_content(basic_network_info);
		});

		let mining_stats = stats.mining_stats.clone();
		let device_stats = mining_stats.device_stats;
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
