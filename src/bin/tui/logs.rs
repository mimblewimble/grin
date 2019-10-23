// Copyright 2019 The Grin Developers
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

use cursive::theme::{Color, ColorStyle};
use cursive::traits::Identifiable;
use cursive::view::View;
use cursive::views::BoxView;
use cursive::{Cursive, Printer};

use crate::tui::constants::VIEW_LOGS;
use grin_util::logger::LogEntry;
use log::Level;
use std::collections::VecDeque;

pub struct TUILogsView;

impl TUILogsView {
	pub fn create() -> Box<dyn View> {
		let logs_view = BoxView::with_full_screen(LogBufferView::new(200).with_id("logs"));
		Box::new(logs_view.with_id(VIEW_LOGS))
	}

	pub fn update(c: &mut Cursive, entry: LogEntry) {
		c.call_on_id("logs", |t: &mut LogBufferView| {
			t.update(entry);
		});
	}
}

struct LogBufferView {
	buffer: VecDeque<LogEntry>,
	green: ColorStyle,
	orange: ColorStyle,
	red: ColorStyle,
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

		LogBufferView {
			buffer,
			red: ColorStyle::new(Color::Rgb(254, 66, 66), Color::Rgb(0, 0, 0)),
			orange: ColorStyle::new(Color::Rgb(255, 134, 0), Color::Rgb(0, 0, 0)),
			green: ColorStyle::new(Color::Rgb(66, 255, 66), Color::Rgb(0, 0, 0)),
		}
	}

	fn update(&mut self, entry: LogEntry) {
		self.buffer.push_front(entry);
		self.buffer.pop_back();
	}
}

impl View for LogBufferView {
	fn draw(&self, printer: &Printer) {
		let mut i = 0;
		for entry in self.buffer.iter().take(printer.size.y) {
			for line in entry.log.lines().rev() {
				let print = |p: &Printer| p.print((0, p.size.y - 1 - i), line);
				match entry.level {
					Level::Info => printer.with_color(self.green, print),
					Level::Warn => printer.with_color(self.orange, print),
					Level::Error => printer.with_color(self.red, print),
					_ => print(printer),
				}
				i += 1;
			}
		}
	}
}
