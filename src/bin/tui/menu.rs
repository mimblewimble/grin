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

//! Main Menu definition

use cursive::Cursive;
use cursive::view::AnyView;
use cursive::align::HAlign;
use cursive::event::{EventResult, Key};
use cursive::views::{BoxView, LinearLayout, OnEventView, SelectView, StackView, TextView};
use cursive::direction::Orientation;

use tui::constants::*;

pub fn create() -> Box<AnyView> {
	let mut main_menu = SelectView::new().h_align(HAlign::Left);
	main_menu.add_item("Basic Status", VIEW_BASIC_STATUS);
	main_menu.add_item("Peers and Sync", VIEW_PEER_SYNC);
	main_menu.add_item("Mining", VIEW_MINING);
	let change_view = |s: &mut Cursive, v: &str| {
		if v == "" {
			return;
		}

		let _ = s.call_on_id(ROOT_STACK, |sv: &mut StackView| {
			let pos = sv.find_layer_from_id(v).unwrap();
			sv.move_to_front(pos);
		});
	};

	main_menu.set_on_submit(change_view);

	let main_menu = OnEventView::new(main_menu)
		.on_pre_event_inner('k', |s| {
			s.select_up(1);
			Some(EventResult::Consumed(None))
		})
		.on_pre_event_inner('j', |s| {
			s.select_down(1);
			Some(EventResult::Consumed(None))
		})
		.on_pre_event_inner(Key::Tab, |s| {
			if s.selected_id().unwrap() == s.len() - 1 {
				s.set_selection(0);
			} else {
				s.select_down(1);
			}
			Some(EventResult::Consumed(None))
		});
	let main_menu = LinearLayout::new(Orientation::Vertical)
		.child(BoxView::with_full_height(main_menu))
		.child(TextView::new("------------------"))
		.child(TextView::new("Tab/Arrow : Cycle "))
		.child(TextView::new("Enter     : Select"))
		.child(TextView::new("Q         : Quit  "));
	Box::new(main_menu)
}
