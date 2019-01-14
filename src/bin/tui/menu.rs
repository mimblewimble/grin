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

use cursive::align::HAlign;
use cursive::direction::Orientation;
use cursive::event::{EventResult, Key};
use cursive::view::Identifiable;
use cursive::view::View;
use cursive::views::{
	BoxView, LinearLayout, OnEventView, SelectView, StackView, TextView, ViewRef,
};
use cursive::Cursive;

use crate::tui::constants::{
	MAIN_MENU, ROOT_STACK, SUBMENU_MINING_BUTTON, VIEW_BASIC_STATUS, VIEW_MINING, VIEW_PEER_SYNC,
	VIEW_VERSION,
};

pub fn create() -> Box<dyn View> {
	let mut main_menu = SelectView::new().h_align(HAlign::Left).with_id(MAIN_MENU);
	main_menu
		.get_mut()
		.add_item("Basic Status", VIEW_BASIC_STATUS);
	main_menu
		.get_mut()
		.add_item("Peers and Sync", VIEW_PEER_SYNC);
	main_menu.get_mut().add_item("Mining", VIEW_MINING);
	main_menu.get_mut().add_item("Version Info", VIEW_VERSION);
	let change_view = |s: &mut Cursive, v: &&str| {
		if *v == "" {
			return;
		}

		let _ = s.call_on_id(ROOT_STACK, |sv: &mut StackView| {
			let pos = sv.find_layer_from_id(v).unwrap();
			sv.move_to_front(pos);
		});
	};

	main_menu.get_mut().set_on_select(change_view);
	main_menu
		.get_mut()
		.set_on_submit(|c: &mut Cursive, v: &str| {
			if v == VIEW_MINING {
				let _ = c.focus_id(SUBMENU_MINING_BUTTON);
			}
		});
	let main_menu = OnEventView::new(main_menu)
		.on_pre_event('j', move |c| {
			let mut s: ViewRef<SelectView<&str>> = c.find_id(MAIN_MENU).unwrap();
			s.select_down(1)(c);
			Some(EventResult::Consumed(None));
		})
		.on_pre_event('k', move |c| {
			let mut s: ViewRef<SelectView<&str>> = c.find_id(MAIN_MENU).unwrap();
			s.select_up(1)(c);
			Some(EventResult::Consumed(None));
		})
		.on_pre_event(Key::Tab, move |c| {
			let mut s: ViewRef<SelectView<&str>> = c.find_id(MAIN_MENU).unwrap();
			if s.selected_id().unwrap() == s.len() - 1 {
				s.set_selection(0)(c);
			} else {
				s.select_down(1)(c);
			}
			Some(EventResult::Consumed(None));
		});
	let main_menu = LinearLayout::new(Orientation::Vertical)
		.child(BoxView::with_full_height(main_menu))
		.child(TextView::new("------------------"))
		.child(TextView::new("Tab/Arrow : Cycle "))
		.child(TextView::new("Enter     : Select"))
		.child(TextView::new("Esc       : Back  "))
		.child(TextView::new("Q         : Quit  "));
	Box::new(main_menu)
}
