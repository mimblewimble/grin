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
use ratatui::widgets::{Paragraph, Wrap};
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

/// Draw the logs view, bottom-anchoring the newest log lines.
///
/// Uses ratatui's wrapping so display width and whitespace match the terminal.
/// When content is shorter than the pane, it is shifted down so the newest
/// lines still sit on the bottom edge (the old cursive view behaved this way).
pub fn draw(f: &mut Frame, area: Rect, app: &App) {
	if area.width == 0 || area.height == 0 {
		return;
	}

	// logs ring buffer is newest-first; reverse so oldest is at the top.
	let mut lines: Vec<Line> = Vec::new();
	for entry in app.logs.iter().rev() {
		for row in entry.log.trim_end_matches('\n').split('\n') {
			lines.push(Line::from(Span::styled(
				row.to_string(),
				color(entry.level),
			)));
		}
	}

	let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
	let total = paragraph.line_count(area.width);
	let height = area.height as usize;

	if total > height {
		// Scroll so the newest (wrapped) lines fill the pane.
		let scroll = (total - height) as u16;
		f.render_widget(paragraph.scroll((scroll, 0)), area);
	} else {
		// Bottom-align short content by rendering into a sub-area at the bottom.
		let y_offset = (height - total) as u16;
		let content_area = Rect {
			x: area.x,
			y: area.y + y_offset,
			width: area.width,
			height: total as u16,
		};
		f.render_widget(paragraph, content_area);
	}
}
