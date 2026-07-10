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

//! TUI log display: newest entries anchored to the bottom of the pane,
//! matching the behavior of the previous cursive-based log view.

use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::tui::app::App;
use log::Level;

fn color(level: Level) -> Color {
	match level {
		Level::Info => Color::Green,
		Level::Warn => Color::Yellow,
		Level::Error => Color::Red,
		_ => Color::White,
	}
}

/// Word-wraps `text` to `width` columns, hard-breaking words that don't fit
/// on their own. Empty input produces a single empty line.
fn wrap_text(text: &str, width: usize) -> Vec<String> {
	let width = width.max(1);
	let mut out = Vec::new();
	for raw_line in text.split('\n') {
		let mut current = String::new();
		let mut current_len = 0usize;
		for word in raw_line.split(' ') {
			let mut word_chars: Vec<char> = word.chars().collect();
			while word_chars.len() > width {
				if !current.is_empty() {
					out.push(std::mem::take(&mut current));
					current_len = 0;
				}
				let rest = word_chars.split_off(width);
				out.push(word_chars.into_iter().collect());
				word_chars = rest;
			}
			let word_len = word_chars.len();
			let needed = word_len + if current.is_empty() { 0 } else { 1 };
			if current_len + needed > width && !current.is_empty() {
				out.push(std::mem::take(&mut current));
				current_len = 0;
			}
			if !current.is_empty() {
				current.push(' ');
				current_len += 1;
			}
			current.push_str(&word_chars.into_iter().collect::<String>());
			current_len += word_len;
		}
		out.push(current);
	}
	out
}

/// Draw the logs view, bottom-anchoring the newest log lines.
pub fn draw(f: &mut Frame, area: Rect, app: &App) {
	let width = area.width as usize;
	let height = area.height as usize;

	// Walk entries newest-first, wrapping each until we have enough rows to
	// fill the pane, keeping each entry's own lines in the collected block.
	let mut blocks: Vec<(Vec<String>, Level)> = Vec::new();
	let mut rows_collected = 0usize;
	for entry in app.logs.iter() {
		if rows_collected >= height {
			break;
		}
		let wrapped = wrap_text(entry.log.trim_end_matches('\n'), width);
		rows_collected += wrapped.len();
		blocks.push((wrapped, entry.level));
	}

	// Blocks are newest-first; reverse so oldest is at the top, newest at
	// the bottom, matching the old bottom-anchored view.
	blocks.reverse();

	let mut lines: Vec<Line> = Vec::new();
	for (wrapped, level) in &blocks {
		for row in wrapped {
			lines.push(Line::from(Span::styled(row.clone(), color(*level))));
		}
	}

	// If the newest block overflowed the pane, keep only the tail.
	if lines.len() > height {
		lines.drain(0..lines.len() - height);
	}

	// Pad with blank lines at the top so short logs still anchor to the
	// bottom of the pane.
	let pad = height.saturating_sub(lines.len());
	let mut padded = Vec::with_capacity(height);
	padded.resize_with(pad, || Line::from(""));
	padded.extend(lines);

	f.render_widget(Paragraph::new(padded), area);
}
