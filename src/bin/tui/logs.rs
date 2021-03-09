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

use cursive::theme::{BaseColor, Color, ColorStyle};
use cursive::traits::Identifiable;
use cursive::view::View;
use cursive::views::ResizedView;
use cursive::{Cursive, Printer};

use crate::tui::constants::VIEW_LOGS;
use cursive::utils::lines::spans::{LinesIterator, Row};
use cursive::utils::markup::StyledString;
use grin_util::logger::LogEntry;
use log::Level;
use std::collections::VecDeque;

pub struct TUILogsView;

impl TUILogsView {
	pub fn create() -> impl View {
		let logs_view = ResizedView::with_full_screen(LogBufferView::new(200).with_name("logs"));
		logs_view.with_name(VIEW_LOGS)
	}

	pub fn update(c: &mut Cursive, entry: LogEntry) {
		c.call_on_name("logs", |t: &mut LogBufferView| {
			t.update(entry);
		});
	}
}

struct LogBufferView {
	buffer: VecDeque<LogEntry>,
}

impl LogBufferView {
	fn new(size: usize) -> Self {
		let mut buffer = VecDeque::new();
		buffer.resize(
			size,
			LogEntry {
				log: String::new(),
				level: Level::Info,
			},
		);

		LogBufferView { buffer }
	}

	fn update(&mut self, entry: LogEntry) {
		self.buffer.push_front(entry);
		self.buffer.pop_back();
	}

	fn color(level: Level) -> ColorStyle {
		match level {
			Level::Info => ColorStyle::new(
				Color::Light(BaseColor::Green),
				Color::Dark(BaseColor::Black),
			),
			Level::Warn => ColorStyle::new(
				Color::Light(BaseColor::Yellow),
				Color::Dark(BaseColor::Black),
			),
			Level::Error => {
				ColorStyle::new(Color::Light(BaseColor::Red), Color::Dark(BaseColor::Black))
			}
			_ => ColorStyle::new(
				Color::Light(BaseColor::White),
				Color::Dark(BaseColor::Black),
			),
		}
	}
}

impl View for LogBufferView {
	fn draw(&self, printer: &Printer) {
		let mut i = 0;
		for entry in self.buffer.iter().take(printer.size.y) {
			printer.with_color(LogBufferView::color(entry.level), |p| {
				let log_message = StyledString::plain(entry.log.as_str());
				let mut rows: Vec<Row> = LinesIterator::new(&log_message, printer.size.x).collect();
				rows.reverse(); // So stack traces are in the right order.
				for row in rows {
					for span in row.resolve(&log_message) {
						p.print((0, p.size.y.saturating_sub(i + 1)), span.content);
						i += 1;
					}
				}
			});
		}
	}
}
