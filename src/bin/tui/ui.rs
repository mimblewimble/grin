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
use std::cmp::Ordering;
use time;

use cursive::Cursive;
use cursive::theme::{BaseColor, BorderStyle, Color, Theme};
use cursive::theme::PaletteColor::*;
use cursive::theme::Color::*;
use cursive::theme::BaseColor::*;
use cursive::utils::markup::StyledString;
use cursive::align::HAlign;
use cursive::event::{EventResult, Key};
use cursive::view::AnyView;
use cursive::views::{BoxView, Dialog, LinearLayout, OnEventView, Panel, SelectView, StackView,
                     TextView};
use cursive::direction::Orientation;
use cursive::traits::*;

use grin::Server;
use grin::types::PeerStats;
use util::LOGGER;

use tui::table::{TableView, TableViewItem};

const _WELCOME_LOGO: &str = "                 GGGGG                      GGGGGGG         
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

#[derive(Copy, Clone, PartialEq, Eq, Hash)]
enum PeerColumn {
	Address,
	State,
	TotalDifficulty,
	Direction,
	Version,
}

impl PeerColumn {
	fn as_str(&self) -> &str {
		match *self {
			PeerColumn::Address => "Address",
			PeerColumn::State => "State",
			PeerColumn::Version => "Version",
			PeerColumn::TotalDifficulty => "Total Difficulty",
			PeerColumn::Direction => "Direction",
		}
	}
}

impl TableViewItem<PeerColumn> for PeerStats {
	fn to_column(&self, column: PeerColumn) -> String {
		match column {
			PeerColumn::Address => self.addr.clone(),
			PeerColumn::State => self.state.clone(),
			PeerColumn::TotalDifficulty => self.total_difficulty.to_string(),
			PeerColumn::Direction => self.direction.clone(),
			PeerColumn::Version => self.version.to_string(),
		}
	}

	fn cmp(&self, other: &Self, column: PeerColumn) -> Ordering
	where
		Self: Sized,
	{
		match column {
			PeerColumn::Address => self.addr.cmp(&other.addr),
			PeerColumn::State => self.state.cmp(&other.state),
			PeerColumn::TotalDifficulty => self.total_difficulty.cmp(&other.total_difficulty),
			PeerColumn::Direction => self.direction.cmp(&other.direction),
			PeerColumn::Version => self.version.cmp(&other.version),
		}
	}
}

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
	pub peer_stats: Vec<PeerStats>,
}

pub enum UIMessage {
	UpdateStatus(StatusUpdates),
}

fn create_main_menu() -> Box<AnyView> {
	let mut main_menu = SelectView::new().h_align(HAlign::Left);
	main_menu.add_item("Basic Status", "basic_status_view");
	main_menu.add_item("Peers and Sync", "peer_sync_view");
	main_menu.add_item("Mining", "mining_view");
	let change_view = |s: &mut Cursive, v: &str| {
		if v == "" {
			return;
		}

		let _ = s.call_on_id("root_stack", |sv: &mut StackView| {
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

fn create_basic_status_view() -> Box<AnyView> {
	/*let mut logo_string = StyledString::new();
	logo_string.append(StyledString::styled(
		WELCOME_LOGO,
		Color::Dark(BaseColor::Green),
	));*/

	let mut title_string = StyledString::new();
	title_string.append(StyledString::styled(
		"Grin Version 0.0.1",
		Color::Dark(BaseColor::Green),
	));
	/*let mut logo_view = TextView::new(logo_string)
		.v_align(VAlign::Top)
		.h_align(HAlign::Left);
	logo_view.set_scrollable(false);*/
	let basic_status_view =
		LinearLayout::new(Orientation::Vertical).child(BoxView::with_full_screen(
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
						.child(TextView::new("  ").with_id("chain_height")),
				)
				.child(
					LinearLayout::new(Orientation::Horizontal)
						.child(TextView::new("------------------------")),
				)
				.child(
					LinearLayout::new(Orientation::Horizontal)
						.child(TextView::new("  ").with_id("basic_mining_config_status")),
				)
				.child(
					LinearLayout::new(Orientation::Horizontal)
						.child(TextView::new("  ").with_id("basic_mining_status")),
				)
				.child(
					LinearLayout::new(Orientation::Horizontal)
						.child(TextView::new("  ").with_id("basic_network_info")),
				), //.child(logo_view)
		));
	Box::new(basic_status_view.with_id("basic_status_view"))
}

fn create_peer_status_view() -> Box<AnyView> {
	let table_view =
		TableView::<PeerStats, PeerColumn>::new()
			.column(PeerColumn::Address, "Address", |c| c.width_percent(20))
			.column(PeerColumn::State, "State", |c| c.width_percent(20))
			.column(PeerColumn::Direction, "Direction", |c| {
				c.width_percent(20)
			})
			.column(PeerColumn::TotalDifficulty, "Total Difficulty", |c| {
				c.width_percent(20)
			})
			.column(PeerColumn::Version, "Version", |c| c.width_percent(20));

	let peer_status_view = BoxView::with_full_screen(
		Dialog::around(table_view.with_id("peer_status_table").min_size((50, 20)))
			.title("Connected Peers"),
	).with_id("peer_sync_view");
	Box::new(peer_status_view)
}

fn update_peer_status_view(c: &mut Cursive, peer_info: Vec<PeerStats>) {
	let _ = c.call_on_id(
		"peer_status_table",
		|t: &mut TableView<PeerStats, PeerColumn>| {
			t.set_items(peer_info);
		},
	);
}

fn create_mining_status_view() -> Box<AnyView> {
	let mining_view = BoxView::with_full_screen(TextView::new("Mining status coming soon!"))
		.with_id("mining_view");
	Box::new(mining_view)
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
			cursive: Cursive::new(),
			ui_tx: ui_tx,
			ui_rx: ui_rx,
			controller_tx: controller_tx,
		};

		// Create UI objects, etc
		let main_menu = create_main_menu();
		let basic_status_view = create_basic_status_view();
		let peer_status_view = create_peer_status_view();
		let mining_status_view = create_mining_status_view();

		let root_stack = StackView::new()
			.layer(mining_status_view)
			.layer(peer_status_view)
			.layer(basic_status_view)
			.with_id("root_stack");

		let main_layer = LinearLayout::new(Orientation::Horizontal)
			.child(Panel::new(main_menu))
			.child(Panel::new(root_stack));

		//set theme
		let mut theme = grin_ui.cursive.current_theme().clone();
		modify_theme(&mut theme);
		grin_ui.cursive.set_theme(theme);

		grin_ui.cursive.add_layer(main_layer);

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
					update_peer_status_view(&mut self.cursive, update.peer_stats);
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
						" ".to_string(),
					)
				} else if stats.mining_stats.combined_gps == 0.0 {
					(
						"Mining Status: Starting miner and awating first solution...".to_string(),
						" ".to_string(),
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
				(" ".to_string(), " ".to_string())
			}
		};
		let update = StatusUpdates {
			basic_status: basic_status,
			peer_count: stats.peer_count.to_string(),
			chain_height: stats.head.height.to_string(),
			basic_mining_config_status: basic_mining_config_status.to_string(),
			basic_mining_status: basic_mining_status,
			basic_network_info: basic_network_info,
			peer_stats: stats.peer_stats,
		};
		self.ui.ui_tx.send(UIMessage::UpdateStatus(update)).unwrap();
	}
}
