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

//! Main for building the binary of a Grin peer-to-peer node.

use std::sync::{Arc, mpsc};

use cursive::Cursive;
use cursive::theme::{BaseColor, BorderStyle, Color, ColorStyle};
use cursive::event::Key;
use cursive::views::TextView;
use cursive::views::BoxView;
use cursive::views::EditView;
use cursive::views::DummyView;
use cursive::views::Button;
use cursive::views::LinearLayout;
use cursive::views::Panel;
use cursive::views::StackView;
use cursive::views::LayerPosition;
use cursive::views::OnEventView;
use cursive::direction::{Orientation, Direction};
use cursive::view::SizeConstraint;
use cursive::menu::MenuTree;
use cursive::traits::*;
use cursive::views::Dialog;

use grin::Server;

pub struct UI {
	cursive: Cursive,
	ui_rx: mpsc::Receiver<UIMessage>,
	ui_tx: mpsc::Sender<UIMessage>,
	controller_tx: mpsc::Sender<ControllerMessage>,
}

pub enum UIMessage {
	UpdateOutput(String),
}

impl UI {
	/// Create a new UI
	pub fn new(controller_tx: mpsc::Sender<ControllerMessage>) -> UI {
		let (ui_tx, ui_rx) = mpsc::channel::<UIMessage>();
		let mut grin_ui = UI {
			cursive: Cursive::new(),
			ui_tx: ui_tx,
			ui_rx: ui_rx,
			controller_tx: controller_tx
		};

		// Create UI objects, etc
		let basic_status_view = BoxView::with_full_screen(
			LinearLayout::new(Orientation::Vertical)
				.child(TextView::new("Grin - Basic Runtime Info"))
				.child(
					LinearLayout::new(Orientation::Horizontal)
						.child(TextView::new("Current Status:"))
						.child(TextView::new("BASIC_STATUS").with_id("basic_current_status"))
				)
			)
			.with_id("basic_status_view");
		let advanced_status_view = BoxView::with_full_screen(
				TextView::new("Advanced Status Display")
			)
			.with_id("advanced_status");

		let root_stack = StackView::new()
			.layer(advanced_status_view)
			.layer(basic_status_view)
			.with_id("root_stack");

		/*
		let mut basic_button = Button::new("", |s| {
			//
		});
		basic_button.set_label_raw("1 - Basic Status");
		let mut advanced_button = Button::new("2 - Advanced Status", |s| {
			//
		});
		//advanced_button.set_label_raw("2 - Advanced Status");
		let mut config_button = Button::new("", |s| {
			//
		});
		config_button.set_label_raw("3 - Config");
		let mut quit_button = Button::new("", |s| {
			//s.quit();
		});
		quit_button.set_label_raw("Quit");*/

		let temp_text = TextView::new("temp_text").with_id("temp_text");


		let top_layer = LinearLayout::new(Orientation::Vertical)
			.child(Panel::new(root_stack))
			.child(
				LinearLayout::new(Orientation::Horizontal)
					.child(Panel::new(TextView::new("<TAB> Toggle Basic / Advanced view")))
					.child(Panel::new(TextView::new("<Q> Quit")))
			);
			/*.child(
				LinearLayout::new(Orientation::Horizontal)
					.child(Panel::new(basic_button))
					.child(Panel::new(advanced_button))
					.child(Panel::new(config_button))
					.child(Panel::new(quit_button))
			);*/


		grin_ui.cursive.add_global_callback(Key::Tab, |s| {
			//let bas_sta = s.find_id::<Panel<BoxView<TextView>>>("basic_status").unwrap();
			//let mut root_stack = s.find_id::<StackView>("root_stack").unwrap();
			//root_stack.add_layer(adv_sta);
			
			s.call_on_id("root_stack", |sv: &mut StackView| {
				/*if let FunctionalState::BasicStatus = cur_state {
					return;
				}*/
				sv.move_to_front(LayerPosition::FromBack(0));
				//sv.add_layer(advanced_status);
				/*sv.pop_layer();
				sv.add_layer(bas_sta);*/
				
			});
		});
		grin_ui.cursive.add_global_callback('q', |s| {
			s.quit();
		});
		grin_ui.cursive.load_theme_file("guistyle.toml").unwrap();
		grin_ui.cursive.add_layer(top_layer);

		// Configure a callback (shutdown, for the first test)
		let controller_tx_clone = grin_ui.controller_tx.clone();
		grin_ui.cursive.add_global_callback('q', move |c| {
			controller_tx_clone.send(
				ControllerMessage::Shutdown
			).unwrap();
		});
		grin_ui
	}

	/// Step the UI by calling into Cursive's step function, then
	/// processing any UI messages
	pub fn step(&mut self) -> bool {
		if !self.cursive.is_running() {
			return false;
		}

		// Process any pending UI messages
		while let Some(message) = self.ui_rx.try_iter().next() {
			match message {
				UIMessage::UpdateOutput(text) => {
					//find and update here as needed
				}
			}
		}

		// Step the UI
		self.cursive.step();

		true
	}
}

pub struct Controller {
	rx :mpsc::Receiver<ControllerMessage>,
	ui: UI,
}

pub enum ControllerMessage {
	Shutdown,
}

impl Controller {
	/// Create a new controller
	pub fn new() -> Result<Controller, String> {
		let (tx, rx) = mpsc::channel::<ControllerMessage>();
		Ok (Controller {
			rx: rx,
			ui: UI::new(tx.clone()),
		})
	}
	/// Run the controller
	pub fn run(&mut self, server:Arc<Server>) {
		while self.ui.step() {
			while let Some(message) = self.rx.try_iter().next() {
				match message {
					ControllerMessage::Shutdown => {
						server.stop();
						self.ui
							.ui_tx
							.send(UIMessage::UpdateOutput("update".to_string()))
							.unwrap();

					}
				}
			}
		}
	}

}

