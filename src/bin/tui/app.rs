// Copyright 2024 The Grin Developers
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

//! Central application state for the ratatui-based TUI

use crate::servers::ServerStats;
use grin_util::logger::LogEntry;
use ratatui::layout::Rect;
use ratatui::widgets::TableState;
use std::collections::VecDeque;

/// Number of log lines retained in the ring buffer
pub const LOG_BUFFER_SIZE: usize = 200;

/// Top level tabs, in the order they appear in the side menu
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Tab {
	Status,
	Peers,
	Mining,
	Logs,
	Version,
}

impl Tab {
	pub const ALL: [Tab; 5] = [
		Tab::Status,
		Tab::Peers,
		Tab::Mining,
		Tab::Logs,
		Tab::Version,
	];

	pub fn title(&self) -> &'static str {
		match self {
			Tab::Status => "Basic Status",
			Tab::Peers => "Peers and Sync",
			Tab::Mining => "Mining",
			Tab::Logs => "Logs",
			Tab::Version => "Version Info",
		}
	}

	pub fn index(&self) -> usize {
		Tab::ALL.iter().position(|t| t == self).unwrap_or(0)
	}
}

/// Which sub-screen of the Mining tab is showing
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum MiningSubview {
	Workers,
	Difficulty,
}

/// Which pane currently receives key input
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Focus {
	Menu,
	Content,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum DialogKind {
	Info,
	Error,
}

pub struct Dialog {
	pub text: String,
	pub kind: DialogKind,
}

/// All mutable state the UI renders from. Replaces the tree of named
/// cursive views with a single struct that every `draw` function reads.
pub struct App {
	pub tab: Tab,
	pub mining_subview: MiningSubview,
	pub focus: Focus,
	pub stats: Option<ServerStats>,
	pub logs: VecDeque<LogEntry>,
	pub peers_table: TableState,
	pub mining_workers_table: TableState,
	pub mining_diff_table: TableState,
	pub dialog: Option<Dialog>,
	pub should_quit: bool,
	/// Screen area of the menu list, stored at draw time for mouse hit-testing
	pub menu_area: Rect,
}

impl App {
	pub fn new() -> App {
		App {
			tab: Tab::Status,
			mining_subview: MiningSubview::Workers,
			focus: Focus::Menu,
			stats: None,
			logs: VecDeque::with_capacity(LOG_BUFFER_SIZE),
			peers_table: TableState::default(),
			mining_workers_table: TableState::default(),
			mining_diff_table: TableState::default(),
			dialog: None,
			should_quit: false,
			menu_area: Rect::default(),
		}
	}

	/// Number of rows in the table currently on screen, if any
	pub fn current_table_len(&self) -> usize {
		let stats = match &self.stats {
			Some(s) => s,
			None => return 0,
		};
		match self.tab {
			Tab::Peers => stats.peer_stats.len(),
			Tab::Mining => match self.mining_subview {
				MiningSubview::Workers => stats.stratum_stats.worker_stats.len(),
				MiningSubview::Difficulty => stats.diff_stats.last_blocks.len(),
			},
			_ => 0,
		}
	}

	pub fn push_log(&mut self, entry: LogEntry) {
		self.logs.push_front(entry);
		if self.logs.len() > LOG_BUFFER_SIZE {
			self.logs.pop_back();
		}
	}

	pub fn select_menu_next(&mut self) {
		let next = (self.tab.index() + 1) % Tab::ALL.len();
		self.tab = Tab::ALL[next];
	}

	pub fn select_menu_prev(&mut self) {
		let len = Tab::ALL.len();
		let prev = (self.tab.index() + len - 1) % len;
		self.tab = Tab::ALL[prev];
	}
}
