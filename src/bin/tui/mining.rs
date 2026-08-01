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

//! Mining status view definition

use chrono::prelude::{DateTime, Utc};
use std::time;

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};
use ratatui::Frame;

use crate::servers::{DiffBlock, ServerStats, WorkerStats};
use crate::tui::app::{App, MiningSubview};
use ratatui::widgets::TableState;

fn worker_row(w: &WorkerStats) -> Row<'static> {
	let naive_datetime = DateTime::<Utc>::from_timestamp(
		w.last_seen
			.duration_since(time::UNIX_EPOCH)
			.unwrap()
			.as_secs() as i64,
		0,
	)
	.unwrap_or_default()
	.naive_utc();
	let datetime: DateTime<Utc> = DateTime::from_naive_utc_and_offset(naive_datetime, Utc);

	Row::new(vec![
		Cell::from(w.id.clone()),
		Cell::from(w.is_connected.to_string()),
		Cell::from(datetime.to_string()),
		Cell::from(w.pow_difficulty.to_string()),
		Cell::from(w.num_accepted.to_string()),
		Cell::from(w.num_rejected.to_string()),
		Cell::from(w.num_stale.to_string()),
		Cell::from(w.num_blocks_found.to_string()),
	])
}

const WORKER_HEADERS: [&str; 8] = [
	"ID",
	"Connected",
	"Last Seen",
	"Difficulty",
	"Accepted",
	"Rejected",
	"Stale",
	"Blocks Found",
];
const WORKER_WIDTHS: [u16; 8] = [6, 14, 20, 10, 5, 5, 5, 35];

fn diff_row(d: &DiffBlock) -> Row<'static> {
	let naive_datetime = DateTime::<Utc>::from_timestamp(d.time as i64, 0)
		.unwrap_or_default()
		.naive_utc();
	let datetime: DateTime<Utc> = DateTime::from_naive_utc_and_offset(naive_datetime, Utc);

	Row::new(vec![
		Cell::from(d.block_height.to_string()),
		Cell::from(d.block_hash.to_string()),
		Cell::from(d.difficulty.to_string()),
		Cell::from(format!("{}", datetime)),
		Cell::from(format!("{}s", d.duration)),
	])
}

const DIFF_HEADERS: [&str; 5] = [
	"Height",
	"Hash",
	"Network Difficulty",
	"Block Time",
	"Duration",
];
const DIFF_WIDTHS: [u16; 5] = [15, 15, 15, 30, 25];

/// Draw the mining view
pub fn draw(f: &mut Frame, area: Rect, app: &mut App) {
	let App {
		stats,
		mining_subview,
		mining_workers_table,
		mining_diff_table,
		..
	} = app;
	let stats = stats.as_ref();

	let chunks = Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).split(area);

	let submenu = Line::from(vec![
		ratatui::text::Span::styled(
			" Mining Server Status ",
			if *mining_subview == MiningSubview::Workers {
				Style::default().add_modifier(Modifier::REVERSED)
			} else {
				Style::default()
			},
		),
		ratatui::text::Span::raw("  "),
		ratatui::text::Span::styled(
			" Difficulty ",
			if *mining_subview == MiningSubview::Difficulty {
				Style::default().add_modifier(Modifier::REVERSED)
			} else {
				Style::default()
			},
		),
		ratatui::text::Span::raw("   (w: workers, d: difficulty)"),
	]);
	f.render_widget(Paragraph::new(submenu), chunks[0]);

	match mining_subview {
		MiningSubview::Workers => draw_workers(f, chunks[1], stats, mining_workers_table),
		MiningSubview::Difficulty => draw_difficulty(f, chunks[1], stats, mining_diff_table),
	}
}

fn draw_workers(f: &mut Frame, area: Rect, stats: Option<&ServerStats>, state: &mut TableState) {
	let chunks = Layout::vertical([Constraint::Length(7), Constraint::Min(0)]).split(area);

	let lines = match stats {
		Some(stats) => {
			let s = &stats.stratum_stats;
			let block_height = if s.num_workers == 0 {
				"Solving Block Height:  n/a".to_string()
			} else {
				format!("Solving Block Height:  {}", s.block_height)
			};
			let network_difficulty = if s.num_workers == 0 {
				"Network Difficulty:    n/a".to_string()
			} else {
				format!("Network Difficulty:    {}", s.network_difficulty)
			};
			let network_hashrate = if s.num_workers == 0 {
				"Network Hashrate:      n/a".to_string()
			} else {
				format!(
					"Network Hashrate C{}:  {:.2}",
					s.edge_bits, s.network_hashrate
				)
			};
			vec![
				Line::from(format!("Mining server enabled: {}", s.is_enabled)),
				Line::from(format!("Mining server running: {}", s.is_running)),
				Line::from(format!("Active workers:        {}", s.num_workers)),
				Line::from(block_height),
				Line::from(format!("Blocks Found:          {}", s.blocks_found)),
				Line::from(network_difficulty),
				Line::from(network_hashrate),
			]
		}
		None => (0..7).map(|_| Line::from("")).collect(),
	};
	f.render_widget(Paragraph::new(lines), chunks[0]);

	let workers: &[WorkerStats] = stats
		.map(|s| s.stratum_stats.worker_stats.as_slice())
		.unwrap_or(&[]);
	let rows: Vec<Row> = workers.iter().map(worker_row).collect();
	let widths: Vec<Constraint> = WORKER_WIDTHS
		.iter()
		.map(|w| Constraint::Percentage(*w))
		.collect();
	let table = Table::new(rows, widths)
		.header(Row::new(WORKER_HEADERS.to_vec()))
		.block(
			Block::default()
				.borders(Borders::ALL)
				.title("Mining Workers"),
		)
		.row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));
	f.render_stateful_widget(table, chunks[1], state);
}

fn draw_difficulty(f: &mut Frame, area: Rect, stats: Option<&ServerStats>, state: &mut TableState) {
	let chunks = Layout::vertical([Constraint::Length(4), Constraint::Min(0)]).split(area);

	let lines = match stats {
		Some(stats) => {
			let d = &stats.diff_stats;
			let dur = time::Duration::from_secs(d.average_block_time);
			vec![
				Line::from(format!("Tip Height: {}", d.height)),
				Line::from(format!("Difficulty Adjustment Window: {}", d.window_size)),
				Line::from(format!("Average Block Time: {} Secs", dur.as_secs())),
				Line::from(format!("Average Difficulty: {}", d.average_difficulty)),
			]
		}
		None => vec![
			Line::from(""),
			Line::from(""),
			Line::from(""),
			Line::from(""),
		],
	};
	f.render_widget(Paragraph::new(lines), chunks[0]);

	let diff_blocks: &[DiffBlock] = stats
		.map(|s| s.diff_stats.last_blocks.as_slice())
		.unwrap_or(&[]);
	// Newest block first, matching the previous view's reversed ordering
	let rows: Vec<Row> = diff_blocks.iter().rev().map(diff_row).collect();
	let widths: Vec<Constraint> = DIFF_WIDTHS
		.iter()
		.map(|w| Constraint::Percentage(*w))
		.collect();
	let table = Table::new(rows, widths)
		.header(Row::new(DIFF_HEADERS.to_vec()))
		.block(
			Block::default()
				.borders(Borders::ALL)
				.title("Mining Difficulty Data"),
		)
		.row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));
	f.render_stateful_widget(table, chunks[1], state);
}
