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

//! Main Menu definition

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{List, ListItem, Paragraph};
use ratatui::Frame;

use crate::tui::app::{App, Focus, Tab};

/// Draw the main menu (tab list) and its keybinding hints
pub fn draw(f: &mut Frame, area: Rect, app: &mut App) {
	let chunks = Layout::vertical([Constraint::Min(0), Constraint::Length(5)]).split(area);
	app.menu_area = chunks[0];

	let items: Vec<ListItem> = Tab::ALL
		.iter()
		.map(|t| {
			let style = if *t == app.tab {
				let base = Style::default().add_modifier(Modifier::BOLD);
				if app.focus == Focus::Menu {
					base.fg(Color::Cyan)
				} else {
					base.fg(Color::Blue)
				}
			} else {
				Style::default()
			};
			ListItem::new(t.title()).style(style)
		})
		.collect();
	f.render_widget(List::new(items), chunks[0]);

	let hints = vec![
		Line::from("------------------"),
		Line::from("Tab/Arrow : Cycle "),
		Line::from("Enter     : Select"),
		Line::from("Esc       : Back  "),
		Line::from("Q         : Quit  "),
	];
	f.render_widget(Paragraph::new(hints), chunks[1]);
}
