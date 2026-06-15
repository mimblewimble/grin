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

//! Basic TUI to better output the overall system status and status
//! of various subsystems

use super::constants::MAIN_MENU;
use crate::built_info;
use crate::servers::Server;
use crate::tui::constants::{ROOT_STACK, VIEW_BASIC_STATUS, VIEW_MINING, VIEW_PEER_SYNC};
use crate::tui::types::{TUIStatusListener, UIMessage};
use crate::tui::{logs, menu, mining, peers, status, version};
use chrono::prelude::Utc;
use cursive::direction::Orientation;
use cursive::theme::BaseColor::{Black, Blue, Cyan, White};
use cursive::theme::Color::Dark;
use cursive::theme::PaletteColor::{
	Background, Highlight, HighlightInactive, Primary, Shadow, View,
};
use cursive::theme::{BaseColor, BorderStyle, Color, Theme};
use cursive::traits::{Nameable, Resizable};
use cursive::utils::markup::StyledString;
use cursive::views::{
	CircularFocus, Dialog, LinearLayout, Panel, SelectView, StackView, TextView, ViewRef,
};
use cursive::{CursiveRunnable, CursiveRunner};
use grin_core::global;
use grin_servers::common::types::{Error, ServerInitStatus};
use grin_util::logger::LogEntry;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::{thread, time};

pub struct UI {
	cursive: CursiveRunner<CursiveRunnable>,
	ui_rx: mpsc::Receiver<UIMessage>,
	ui_tx: mpsc::Sender<UIMessage>,
	controller_tx: mpsc::Sender<ControllerMessage>,
	logs_rx: Option<mpsc::Receiver<LogEntry>>,
	show_dialog: Arc<AtomicBool>,
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
	pub fn new(
		controller_tx: mpsc::Sender<ControllerMessage>,
		logs_rx: Option<mpsc::Receiver<LogEntry>>,
	) -> UI {
		let (ui_tx, ui_rx) = mpsc::channel::<UIMessage>();

		let mut grin_ui = UI {
			cursive: cursive::default().into_runner(),
			ui_tx,
			ui_rx,
			controller_tx,
			logs_rx,
			show_dialog: Arc::new(AtomicBool::new(false)),
		};

		// Create UI objects, etc
		let status_view = status::TUIStatusView::create();
		let mining_view = mining::TUIMiningView::create();
		let peer_view = peers::TUIPeerView::create();
		let logs_view = logs::TUILogsView::create();
		let version_view = version::TUIVersionView::create();

		let main_menu = menu::create();

		let root_stack = StackView::new()
			.layer(version_view)
			.layer(mining_view)
			.layer(peer_view)
			.layer(logs_view)
			.layer(status_view)
			.with_name(ROOT_STACK)
			.full_height();

		let mut title_string = StyledString::new();
		title_string.append(StyledString::styled(
			format!(
				"Grin Version {} [{:?}]",
				built_info::PKG_VERSION,
				global::get_chain_type()
			),
			Dark(BaseColor::Green),
		));

		let main_layer = LinearLayout::new(Orientation::Vertical)
			.child(Panel::new(TextView::new(title_string).full_width()))
			.child(
				LinearLayout::new(Orientation::Horizontal)
					.child(Panel::new(main_menu))
					.child(Panel::new(root_stack)),
			);

		//set theme
		let mut theme = grin_ui.cursive.current_theme().clone();
		modify_theme(&mut theme);
		grin_ui.cursive.set_theme(theme);

		grin_ui.cursive.add_fullscreen_layer(main_layer);

		// Configure a callback (shutdown, for the first test)
		let controller_tx_clone = grin_ui.controller_tx.clone();
		let show_dialog_clone = grin_ui.show_dialog.clone();
		grin_ui.cursive.add_global_callback('q', move |c| {
			if show_dialog_clone.load(Ordering::Relaxed) {
				c.pop_layer();
			}
			let content = StyledString::styled("Shutting down...", Color::Light(BaseColor::Yellow));
			c.add_layer(CircularFocus::new(Dialog::around(TextView::new(content))).wrap_tab());
			let _ = controller_tx_clone.send(ControllerMessage::Shutdown);
		});

		grin_ui.cursive.set_fps(3);
		grin_ui
	}

	/// Step the UI by calling into Cursive's step function, then
	/// processing any UI messages
	pub fn step(&mut self) -> bool {
		if !self.cursive.is_running() {
			return false;
		}

		if let Some(logs_rx) = &self.logs_rx {
			while let Some(message) = logs_rx.try_iter().next() {
				logs::TUILogsView::update(&mut self.cursive, message);
			}
		}

		// Process any pending UI messages
		while let Some(message) = self.ui_rx.try_iter().next() {
			let menu: ViewRef<SelectView<&str>> = self.cursive.find_name(MAIN_MENU).unwrap();
			if let Some(selection) = menu.selection() {
				match message {
					UIMessage::UpdateStatus(update) => match *selection {
						VIEW_BASIC_STATUS => {
							status::TUIStatusView::update(&mut self.cursive, &update)
						}
						VIEW_MINING => mining::TUIMiningView::update(&mut self.cursive, &update),
						VIEW_PEER_SYNC => peers::TUIPeerView::update(&mut self.cursive, &update),
						_ => {}
					},
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
	serv_rx: mpsc::Receiver<ServerInitStatus>,
	server: Option<Server>,
}

pub enum ControllerMessage {
	Shutdown,
}

impl Controller {
	/// Create a new controller
	pub fn new(
		logs_rx: Option<mpsc::Receiver<LogEntry>>,
		serv_rx: mpsc::Receiver<ServerInitStatus>,
	) -> Result<Controller, String> {
		let (tx, rx) = mpsc::channel::<ControllerMessage>();
		Ok(Controller {
			rx,
			ui: UI::new(tx, logs_rx),
			serv_rx,
			server: None,
		})
	}

	/// Server initialization status.
	pub fn init_status(&mut self, text: &str, pop: bool) {
		if pop {
			self.ui.cursive.pop_layer();
		}
		let content = StyledString::styled(text, Color::Light(BaseColor::Green));
		self.ui
			.cursive
			.add_layer(CircularFocus::new(Dialog::around(TextView::new(content))).wrap_tab());
		self.ui.show_dialog.store(true, Ordering::Relaxed);
	}

	/// Server initialization error.
	pub fn init_error(&mut self, e: Error) {
		let content = StyledString::styled(format!("{:?}", e), Color::Light(BaseColor::Red));
		self.ui.cursive.add_layer(
			CircularFocus::new(Dialog::around(TextView::new(content)).button("Exit", |s| {
				s.quit();
			}))
			.wrap_tab(),
		);
		self.ui.show_dialog.store(true, Ordering::Relaxed);
	}

	/// Server UI after initialization.
	pub fn server(&mut self, server: &Server) {
		if let Ok(stats) = server.get_server_stats() {
			self.ui.ui_tx.send(UIMessage::UpdateStatus(stats)).unwrap();
		}
	}

	/// Run the controller
	pub fn run(&mut self) -> i32 {
		self.init_status("Starting server...", false);

		let stat_update_interval = 1;
		let mut next_stat_update = Utc::now().timestamp() + stat_update_interval;
		let delay = time::Duration::from_millis(50);
		let mut exit_code = 0;
		while self.ui.step() {
			if let Some(message) = self.rx.try_iter().next() {
				return match message {
					ControllerMessage::Shutdown => {
						warn!("Shutdown in progress, please wait");
						self.ui.stop();
						if let Some(s) = self.server.take() {
							s.stop();
						}
						exit_code
					}
				};
			}

			if let Some(m) = self.serv_rx.try_iter().next() {
				match m {
					ServerInitStatus::LoadDatabase => self.init_status("Loading database...", true),
					ServerInitStatus::StartSync => self.init_status("Start syncing...", true),
					ServerInitStatus::StartAPI => self.init_status("Starting API...", true),
					ServerInitStatus::FinishedLoading(s) => {
						self.ui.cursive.pop_layer();
						self.ui.show_dialog.store(false, Ordering::Relaxed);
						self.server = Some(s)
					}
					ServerInitStatus::ErrorLoading(e) => {
						exit_code = 1;
						self.init_error(e);
					}
					ServerInitStatus::DBMigrationProgress(p) => {
						let status = format!("Migrating database: {}%, please wait...", p);
						self.init_status(status.as_str(), true);
					}
				}
			}

			if Utc::now().timestamp() > next_stat_update {
				next_stat_update = Utc::now().timestamp() + stat_update_interval;
				if let Some(server) = &self.server {
					if let Ok(stats) = server.get_server_stats() {
						self.ui.ui_tx.send(UIMessage::UpdateStatus(stats)).unwrap();
					}
				}
			}
			thread::sleep(delay);
		}
		exit_code
	}
}
