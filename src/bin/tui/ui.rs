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

//! Basic TUI to better output the overall system status and status
//! of various subsystems

use chrono::prelude::Utc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};

use cursive::direction::Orientation;
use cursive::theme::BaseColor::{Black, Blue, Cyan, White};
use cursive::theme::Color::Dark;
use cursive::theme::PaletteColor::{
	Background, Highlight, HighlightInactive, Primary, Shadow, View,
};
use cursive::theme::{BaseColor, BorderStyle, Color, Theme};
use cursive::traits::Boxable;
use cursive::traits::Identifiable;
use cursive::utils::markup::StyledString;
use cursive::views::{LinearLayout, Panel, StackView, TextView, ViewBox};
use cursive::Cursive;

use servers::Server;

use tui::constants::ROOT_STACK;
use tui::types::{TUIStatusListener, UIMessage};
use tui::{menu, mining, peers, status, version};

use built_info;

pub struct UI {
	cursive: Cursive,
	ui_rx: mpsc::Receiver<UIMessage>,
	ui_tx: mpsc::Sender<UIMessage>,
	controller_tx: mpsc::Sender<ControllerMessage>,
}

fn modify_theme(theme: &mut Theme) {
	theme.shadow = false;
	theme.borders = BorderStyle::Simple;
	theme.palette[Background] = Dark(Black);
	theme.palette[Shadow] = Dark(Black);
	theme.palette[View] = Dark(Black);
	theme.palette[Primary] = Dark(White);
	theme.palette[Highlight] = Dark(Cyan);
	theme.palette[HighlightInactive] = Dark(Blue);
	// also secondary, tertiary, TitlePrimary, TitleSecondary
}

impl UI {
	/// Create a new UI
	pub fn new(controller_tx: mpsc::Sender<ControllerMessage>) -> UI {
		let (ui_tx, ui_rx) = mpsc::channel::<UIMessage>();
		let mut grin_ui = UI {
			cursive: Cursive::default(),
			ui_tx: ui_tx,
			ui_rx: ui_rx,
			controller_tx: controller_tx,
		};

		// Create UI objects, etc
		let status_view = status::TUIStatusView::create();
		let mining_view = mining::TUIMiningView::create();
		let peer_view = peers::TUIPeerView::create();
		let version_view = version::TUIVersionView::create();

		let main_menu = menu::create();

		let root_stack = StackView::new()
			.layer(version_view)
			.layer(mining_view)
			.layer(peer_view)
			.layer(status_view)
			.with_id(ROOT_STACK)
			.full_height();

		let mut title_string = StyledString::new();
		title_string.append(StyledString::styled(
			format!("Grin Version {}", built_info::PKG_VERSION),
			Color::Dark(BaseColor::Green),
		));

		let main_layer = LinearLayout::new(Orientation::Vertical)
			.child(Panel::new(TextView::new(title_string).full_width()))
			.child(
				LinearLayout::new(Orientation::Horizontal)
					.child(Panel::new(ViewBox::new(main_menu)))
					.child(Panel::new(root_stack)),
			);

		//set theme
		let mut theme = grin_ui.cursive.current_theme().clone();
		modify_theme(&mut theme);
		grin_ui.cursive.set_theme(theme);
		grin_ui.cursive.add_fullscreen_layer(main_layer);

		// Configure a callback (shutdown, for the first test)
		let controller_tx_clone = grin_ui.controller_tx.clone();
		grin_ui.cursive.add_global_callback('q', move |_| {
			controller_tx_clone
				.send(ControllerMessage::Shutdown)
				.unwrap();
		});
		grin_ui.cursive.set_fps(4);
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
				UIMessage::UpdateStatus(update) => {
					status::TUIStatusView::update(&mut self.cursive, &update);
					mining::TUIMiningView::update(&mut self.cursive, &update);
					peers::TUIPeerView::update(&mut self.cursive, &update);
					version::TUIVersionView::update(&mut self.cursive, &update);
				}
			}
		}

		// Step the UI
		self.cursive.step();
		true
	}

	/// Stop the UI
	pub fn stop(&mut self) {
		self.cursive.quit();
	}
}

pub struct Controller {
	rx: mpsc::Receiver<ControllerMessage>,
	ui: UI,
}

pub enum ControllerMessage {
	Shutdown,
}

impl Controller {
	/// Create a new controller
	pub fn new() -> Result<Controller, String> {
		let (tx, rx) = mpsc::channel::<ControllerMessage>();
		Ok(Controller {
			rx: rx,
			ui: UI::new(tx.clone()),
		})
	}
	/// Run the controller
	pub fn run(&mut self, server: Arc<Server>, running: Arc<AtomicBool>) {
		let stat_update_interval = 1;
		let mut next_stat_update = Utc::now().timestamp() + stat_update_interval;
		while self.ui.step() {
			if !running.load(Ordering::SeqCst) {
				warn!("Received SIGINT (Ctrl+C).");
				server.stop();
				self.ui.stop();
			}
			while let Some(message) = self.rx.try_iter().next() {
				match message {
					ControllerMessage::Shutdown => {
						server.stop();
						self.ui.stop();
						running.store(false, Ordering::SeqCst)
						/*self.ui
							.ui_tx
							.send(UIMessage::UpdateOutput("update".to_string()))
							.unwrap();*/
					}
				}
			}

			if Utc::now().timestamp() > next_stat_update {
				let stats = server.get_server_stats().unwrap();
				self.ui.ui_tx.send(UIMessage::UpdateStatus(stats)).unwrap();
				next_stat_update = Utc::now().timestamp() + stat_update_interval;
			}
		}
	}
}
