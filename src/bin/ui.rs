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

use std::sync::{mpsc, Arc};
use time;

use cursive::Cursive;
use cursive::theme::{BaseColor, BorderStyle, Color};
use cursive::theme::PaletteColor::*;
use cursive::theme::Color::*;
use cursive::theme::BaseColor::*;
use cursive::utils::markup::StyledString;
use cursive::align::{HAlign, VAlign};
use cursive::event::Key;
use cursive::views::{BoxView, LayerPosition, LinearLayout, Panel, StackView, TextView};
use cursive::direction::Orientation;
use cursive::traits::*;

use grin::Server;

const WELCOME_LOGO: &str = "                 GGGGG                      GGGGGGG         
               GGGGGGG                      GGGGGGGGG      
             GGGGGGGGG         GGGG         GGGGGGGGGG     
           GGGGGGGGGGG       GGGGGGGG       GGGGGGGGGGG    
          GGGGGGGGGGGG       GGGGGGGG       GGGGGGGGGGGG   
         GGGGGGGGGGGGG       GGGGGGGG       GGGGGGGGGGGGG  
        GGGGGGGGGGGGGG       GGGGGGGG       GGGGGGGGGGGGGG 
        GGGGGGGGGGGGGG       GGGGGGGGGGGGGGGGGGGGGGGGGGGGG 
       GGGGGGGGGGGGGGG       GGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
       GGGGGGGGGGGGGGG       GGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
                             GGGGGG                        
                             GGGGGGG                       
                             GGGGGGGG                      
       GGGGGGGGGGGGGGG       GGGGGGGG       GGGGGGGGGGGGGGG
       GGGGGGGGGGGGGGG       GGGGGGGG       GGGGGGGGGGGGGGG
        GGGGGGGGGGGGGG       GGGGGGGG       GGGGGGGGGGGGGGG
         GGGGGGGGGGGGG       GGGGGGGG       GGGGGGGGGGGGGG 
          GGGGGGGGGGGG       GGGGGGGG       GGGGGGGGGGGGG  
           GGGGGGGGGGG       GGGGGGGG       GGGGGGGGGGGG   
            GGGGGGGGGG       GGGGGGGG       GGGGGGGGGGG    
              GGGGGGGG       GGGGGGGG       GGGGGGGGG      
               GGGGGGG       GGGGGGGG       GGGGGGG        
                  GGGG       GGGGGGGG       GGGG           
                    GG       GGGGGGGG       GG             
                             GGGGGGGG                       ";

pub struct UI {
	cursive: Cursive,
	ui_rx: mpsc::Receiver<UIMessage>,
	ui_tx: mpsc::Sender<UIMessage>,
	controller_tx: mpsc::Sender<ControllerMessage>,
}

pub struct StatusUpdates {
	pub basic_status: String,
	pub peer_count: String,
	pub chain_height: String,
	pub basic_mining_config_status: String,
	pub basic_mining_status: String,
	pub basic_network_info: String,
}

pub enum UIMessage {
	UpdateStatus(StatusUpdates),
}

impl UI {
	/// Create a new UI
	pub fn new(controller_tx: mpsc::Sender<ControllerMessage>) -> UI {
		let (ui_tx, ui_rx) = mpsc::channel::<UIMessage>();
		let mut grin_ui = UI {
			cursive: Cursive::new(),
			ui_tx: ui_tx,
			ui_rx: ui_rx,
			controller_tx: controller_tx,
		};

		let mut logo_string = StyledString::new();
		logo_string.append(StyledString::styled(
			WELCOME_LOGO,
			Color::Dark(BaseColor::Green),
		));

		let mut title_string = StyledString::new();
		title_string.append(StyledString::styled(
			"Grin Version 0.0.1",
			Color::Dark(BaseColor::Green),
		));
		let mut logo_view = TextView::new(logo_string)
			.v_align(VAlign::Center)
			.h_align(HAlign::Center);
		logo_view.set_scrollable(false);

		// Create UI objects, etc
		let basic_status_view = BoxView::with_full_screen(
			LinearLayout::new(Orientation::Horizontal)
				.child(BoxView::with_full_screen(logo_view))
				.child(BoxView::with_full_screen(
					LinearLayout::new(Orientation::Vertical)
						.child(TextView::new(title_string))
						.child(TextView::new("------------------------"))
						.child(
							LinearLayout::new(Orientation::Horizontal)
								.child(TextView::new("Current Status: "))
								.child(TextView::new("Starting").with_id("basic_current_status")),
						)
						.child(
							LinearLayout::new(Orientation::Horizontal)
								.child(TextView::new("Connected Peers: "))
								.child(TextView::new("0").with_id("connected_peers")),
						)
						.child(
							LinearLayout::new(Orientation::Horizontal)
								.child(TextView::new("Chain Height: "))
								.child(TextView::new("").with_id("chain_height")),
						)
						.child(
							LinearLayout::new(Orientation::Horizontal)
								.child(TextView::new("------------------------")),
						)
						.child(
							LinearLayout::new(Orientation::Horizontal)
								.child(TextView::new("").with_id("basic_mining_config_status")),
						)
						.child(
							LinearLayout::new(Orientation::Horizontal)
								.child(TextView::new("").with_id("basic_mining_status")),
						)
						.child(
							LinearLayout::new(Orientation::Horizontal)
								.child(TextView::new("").with_id("basic_network_info")),
						),
				)),
		).with_id("basic_status_view");

		let advanced_status_view = BoxView::with_full_screen(TextView::new(
			"Advanced Status Display will go here and should contain detailed readouts for:
--Latest Blocks
--Sync Info
--Chain Info
--Peer Info
--Mining Info
			",
		)).with_id("advanced_status");

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

		let top_layer = LinearLayout::new(Orientation::Vertical)
			.child(Panel::new(root_stack))
			.child(
				LinearLayout::new(Orientation::Horizontal)
					.child(Panel::new(TextView::new(
						"<TAB> Toggle Basic / Advanced view",
					)))
					.child(Panel::new(TextView::new("<Q> Quit"))),
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
				sv.add_layer(bas_sta);*/			});
		});

		//set theme
		let mut theme = grin_ui.cursive.current_theme().clone();
		theme.shadow = false;
		theme.borders = BorderStyle::Simple;
		theme.palette[Background] = Dark(Black);
		theme.palette[Shadow] = Dark(Black);
		theme.palette[View] = Dark(Black);
		theme.palette[Primary] = Dark(White);
		theme.palette[Highlight] = Dark(Cyan);
		theme.palette[HighlightInactive] = Dark(Blue);
		// also secondary, tertiary, TitlePrimary, TitleSecondary
		grin_ui.cursive.set_theme(theme);

		grin_ui.cursive.add_layer(top_layer);

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
					//find and update here as needed
					self.cursive
						.call_on_id("basic_current_status", |t: &mut TextView| {
							t.set_content(update.basic_status.clone());
						});
					self.cursive
						.call_on_id("connected_peers", |t: &mut TextView| {
							t.set_content(update.peer_count.clone());
						});
					self.cursive.call_on_id("chain_height", |t: &mut TextView| {
						t.set_content(update.chain_height.clone());
					});
					self.cursive
						.call_on_id("basic_mining_config_status", |t: &mut TextView| {
							t.set_content(update.basic_mining_config_status.clone());
						});
					self.cursive
						.call_on_id("basic_mining_status", |t: &mut TextView| {
							t.set_content(update.basic_mining_status.clone());
						});
					self.cursive
						.call_on_id("basic_network_info", |t: &mut TextView| {
							t.set_content(update.basic_network_info.clone());
						});
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
	pub fn run(&mut self, server: Arc<Server>) {
		let stat_update_interval = 1;
		let mut next_stat_update = time::get_time().sec + stat_update_interval;
		while self.ui.step() {
			while let Some(message) = self.rx.try_iter().next() {
				match message {
					ControllerMessage::Shutdown => {
						server.stop();
						self.ui.stop();
						/*self.ui
							.ui_tx
							.send(UIMessage::UpdateOutput("update".to_string()))
							.unwrap();*/
					}
				}
			}
			if time::get_time().sec > next_stat_update {
				self.update_status(server.clone());
				next_stat_update = time::get_time().sec + stat_update_interval;
			}
		}
	}
	/// update the UI with server status at given intervals (should be
	/// once a second at present
	pub fn update_status(&mut self, server: Arc<Server>) {
		let stats = server.get_server_stats().unwrap();
		let basic_status = {
			if stats.is_syncing {
				if stats.awaiting_peers {
					"Waiting for peers".to_string()
				} else {
					format!("Syncing - Latest header: {}", stats.header_head.height).to_string()
				}
			} else {
				"Running".to_string()
			}
		};
		let basic_mining_config_status = {
			if stats.mining_stats.is_enabled {
				"Configured as mining node"
			} else {
				"Configured as validating node only (not mining)"
			}
		};
		let (basic_mining_status, basic_network_info) = {
			if stats.mining_stats.is_enabled {
				if stats.is_syncing {
					(
						"Mining Status: Paused while syncing".to_string(),
						"".to_string(),
					)
				} else if stats.mining_stats.combined_gps == 0.0 {
					(
						"Mining Status: Starting miner and awaiting first solution...".to_string(),
						"".to_string(),
					)
				} else {
					(
						format!(
							"Mining Status: Mining at height {} at {:.*} GPS",
							stats.mining_stats.block_height, 4, stats.mining_stats.combined_gps
						),
						format!(
							"Cuckoo {} - Network Difficulty {}",
							stats.mining_stats.cuckoo_size,
							stats.mining_stats.network_difficulty.to_string()
						),
					)
				}
			} else {
				("".to_string(), "".to_string())
			}
		};
		let update = StatusUpdates {
			basic_status: basic_status,
			peer_count: stats.peer_count.to_string(),
			chain_height: stats.head.height.to_string(),
			basic_mining_config_status: basic_mining_config_status.to_string(),
			basic_mining_status: basic_mining_status,
			basic_network_info: basic_network_info,
		};
		self.ui.ui_tx.send(UIMessage::UpdateStatus(update)).unwrap();
	}
}
