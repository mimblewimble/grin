// Copyright 2018 The Grin Developers
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

use cursive::Cursive;
use cursive::view::AnyView;
use cursive::views::{BoxView, TextView};
use cursive::traits::*;

use tui::constants::*;
use tui::types::*;

use grin::types::ServerStats;

/// Mining status view
pub struct TUIMiningView;

impl TUIStatusListener for TUIMiningView {
	/// Create the mining view
	fn create() -> Box<AnyView> {
		let mining_view = BoxView::with_full_screen(TextView::new("Mining status coming soon!"))
			.with_id(VIEW_MINING);
		Box::new(mining_view)
	}

	/// update
	fn update(c: &mut Cursive, stats: &ServerStats) {}
}
