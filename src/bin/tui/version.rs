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

//! Version and build info

use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::info_strings;

/// Draw basic version/build info
pub fn draw(f: &mut Frame, area: Rect) {
	let (basic_info, detailed_info) = info_strings();
	let lines = vec![
		Line::from(basic_info),
		Line::from(""),
		Line::from(detailed_info),
	];
	f.render_widget(Paragraph::new(lines), area);
}
