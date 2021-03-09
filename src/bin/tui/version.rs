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

use cursive::direction::Orientation;
use cursive::traits::Identifiable;
use cursive::view::View;
use cursive::views::{LinearLayout, ResizedView, TextView};

use crate::tui::constants::VIEW_VERSION;

use crate::info_strings;

pub struct TUIVersionView;

impl TUIVersionView {
	/// Create basic status view
	pub fn create() -> Box<dyn View> {
		let (basic_info, detailed_info) = info_strings();
		let basic_status_view = ResizedView::with_full_screen(
			LinearLayout::new(Orientation::Vertical)
				.child(TextView::new(basic_info))
				.child(TextView::new(" "))
				.child(TextView::new(detailed_info)),
		);
		Box::new(basic_status_view.with_name(VIEW_VERSION))
	}
}
