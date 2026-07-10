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

use crate::servers::PeerStats;

use chrono::prelude::*;
use humansize::{file_size_opts::CONVENTIONAL, FileSize};

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};
use ratatui::Frame;

use crate::tui::app::App;

// Converts a byte count to a human readable size
fn size_to_string(size: u64) -> String {
	size.file_size(CONVENTIONAL)
		.unwrap_or_else(|_| "-".to_string())
}

fn peer_row(p: &PeerStats) -> Row<'static> {
	Row::new(vec![
		Cell::from(p.addr.clone()),
		Cell::from(p.state.clone()),
		Cell::from(format!(
			"↑: {}, ↓: {}",
			size_to_string(p.sent_bytes_per_sec),
			size_to_string(p.received_bytes_per_sec),
		)),
		Cell::from(p.direction.clone()),
		Cell::from(format!(
			"{} D @ {} H ({}s)",
			p.total_difficulty,
			p.height,
			(Utc::now() - p.last_seen).num_seconds(),
		)),
		Cell::from(format!("{}", p.version)),
		Cell::from(format!("{}", p.capabilities.bits())),
		Cell::from(p.user_agent.clone()),
	])
}

const HEADERS: [&str; 8] = [
	"Address",
	"State",
	"Used bandwidth",
	"Direction",
	"Total Difficulty",
	"Proto",
	"Capab",
	"User Agent",
];

const WIDTHS: [u16; 8] = [16, 8, 16, 8, 24, 4, 4, 18];

/// Draw the peers/sync view
pub fn draw(f: &mut Frame, area: Rect, app: &mut App) {
	let App {
		stats, peers_table, ..
	} = app;

	let peer_stats: &[PeerStats] = stats
		.as_ref()
		.map(|s| s.peer_stats.as_slice())
		.unwrap_or(&[]);

	let longest_work_peer = stats.as_ref().map(|stats| {
		let lp = stats
			.peer_stats
			.iter()
			.max_by(|x, y| x.total_difficulty.cmp(&y.total_difficulty));
		match lp {
			Some(l) => format!(
				"{} D @ {} H vs Us: {} D @ {} H",
				l.total_difficulty,
				l.height,
				stats.chain_stats.total_difficulty,
				stats.chain_stats.height
			),
			None => "".to_string(),
		}
	});

	let outbound_count = peer_stats
		.iter()
		.filter(|x| x.direction == "Outbound")
		.count();
	let total_line = format!(
		"Total Peers: {} (Outbound: {})",
		peer_stats.len(),
		outbound_count
	);

	let chunks = Layout::vertical([Constraint::Length(3), Constraint::Min(0)]).split(area);

	let header_lines = vec![
		Line::from(total_line),
		Line::from(format!(
			"Longest Chain: {}",
			longest_work_peer.unwrap_or_default()
		)),
	];
	f.render_widget(Paragraph::new(header_lines), chunks[0]);

	let rows: Vec<Row> = peer_stats.iter().map(peer_row).collect();
	let widths: Vec<Constraint> = WIDTHS.iter().map(|w| Constraint::Percentage(*w)).collect();
	let table = Table::new(rows, widths)
		.header(Row::new(HEADERS.to_vec()))
		.block(
			Block::default()
				.borders(Borders::ALL)
				.title("Connected Peers"),
		)
		.row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));

	f.render_stateful_widget(table, chunks[1], peers_table);
}
