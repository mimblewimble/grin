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

use crate::built_info;
use crate::servers::Server;
use crate::tui::app::{App, Dialog, DialogKind, Focus, MiningSubview, Tab};
use crate::tui::{logs, menu, mining, peers, status, version};
use chrono::prelude::Utc;
use crossterm::event::{
	self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseButton,
	MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
	disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use grin_core::global;
use grin_servers::common::types::{Error, ServerInitStatus};
use grin_util::logger::LogEntry;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::{Frame, Terminal};
use std::io::{self, Stdout};
use std::sync::mpsc;
use std::time::{Duration, Instant};

type Backend = CrosstermBackend<Stdout>;

/// Redraw at least this often even without input or new data, so
/// time-based fields (peer "last seen" ages, uptime, etc.) stay fresh.
/// Matches the 1s stats-update cadence of those fields.
const MAX_REDRAW_INTERVAL: Duration = Duration::from_secs(1);

/// How far PageUp/PageDown move a table selection
const TABLE_PAGE_SIZE: i64 = 10;

/// Undo raw mode / alternate screen / mouse capture set up in `UI::new`.
/// Shared by the panic hook, `stop()`, and `Drop` so the sequence cannot drift.
fn restore_terminal() {
	let _ = disable_raw_mode();
	let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
}

fn install_panic_hook() {
	let original_hook = std::panic::take_hook();
	std::panic::set_hook(Box::new(move |panic_info| {
		restore_terminal();
		original_hook(panic_info);
	}));
}

/// Compute the next table selection given the current selection, row count,
/// and a signed delta (or `i64::MIN`/`i64::MAX` for Home/End).
///
/// `TableState` starts with no selection; the first move anchors to row 0
/// (or the last row for End) rather than jumping past it.
fn next_table_selection(selected: Option<usize>, len: usize, delta: i64) -> Option<usize> {
	if len == 0 {
		return None;
	}
	let next = match selected {
		None if delta == i64::MAX => len - 1,
		None => 0,
		Some(i) => (i as i64).saturating_add(delta).clamp(0, len as i64 - 1) as usize,
	};
	Some(next)
}

pub struct UI {
	terminal: Terminal<Backend>,
	app: App,
	controller_tx: mpsc::Sender<ControllerMessage>,
	logs_rx: Option<mpsc::Receiver<LogEntry>>,
	needs_redraw: bool,
	last_draw: Instant,
	/// Ensures terminal restore runs only once across `stop()` and `Drop`.
	restored: bool,
}

impl UI {
	/// Create a new UI
	pub fn new(
		controller_tx: mpsc::Sender<ControllerMessage>,
		logs_rx: Option<mpsc::Receiver<LogEntry>>,
	) -> io::Result<UI> {
		install_panic_hook();
		enable_raw_mode()?;
		let mut stdout = io::stdout();
		execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
		let terminal = Terminal::new(CrosstermBackend::new(stdout))?;

		Ok(UI {
			terminal,
			app: App::new(),
			controller_tx,
			logs_rx,
			needs_redraw: true,
			last_draw: Instant::now(),
			restored: false,
		})
	}

	fn handle_key(&mut self, code: KeyCode) {
		// Dialogs are modal: only allow quit (and for errors, dismiss-to-exit).
		// Startup info dialogs must not let Tab/j/Enter change the view behind them.
		if let Some(dialog) = &self.app.dialog {
			match dialog.kind {
				DialogKind::Error => {
					if matches!(code, KeyCode::Enter | KeyCode::Char('q') | KeyCode::Esc) {
						self.app.should_quit = true;
					}
				}
				DialogKind::Info => {
					if code == KeyCode::Char('q') {
						self.app.dialog = Some(Dialog {
							text: "Shutting down...".to_string(),
							kind: DialogKind::Info,
						});
						let _ = self.controller_tx.send(ControllerMessage::Shutdown);
					}
				}
			}
			return;
		}

		match code {
			KeyCode::Char('q') => {
				self.app.dialog = Some(Dialog {
					text: "Shutting down...".to_string(),
					kind: DialogKind::Info,
				});
				let _ = self.controller_tx.send(ControllerMessage::Shutdown);
			}
			KeyCode::Char('j') | KeyCode::Down => self.on_down(),
			KeyCode::Char('k') | KeyCode::Up => self.on_up(),
			KeyCode::PageDown => self.move_table_selection(TABLE_PAGE_SIZE),
			KeyCode::PageUp => self.move_table_selection(-TABLE_PAGE_SIZE),
			KeyCode::Home => self.move_table_selection(i64::MIN),
			KeyCode::End => self.move_table_selection(i64::MAX),
			KeyCode::Tab => self.app.select_menu_next(),
			KeyCode::Enter => {
				if self.app.focus == Focus::Menu {
					self.app.focus = Focus::Content;
				}
			}
			KeyCode::Esc => self.app.focus = Focus::Menu,
			KeyCode::Char('w') if self.app.tab == Tab::Mining => {
				self.app.mining_subview = MiningSubview::Workers
			}
			KeyCode::Char('d') if self.app.tab == Tab::Mining => {
				self.app.mining_subview = MiningSubview::Difficulty
			}
			_ => {}
		}
	}

	fn on_down(&mut self) {
		match self.app.focus {
			Focus::Menu => self.app.select_menu_next(),
			Focus::Content => self.move_table_selection(1),
		}
	}

	fn on_up(&mut self) {
		match self.app.focus {
			Focus::Menu => self.app.select_menu_prev(),
			Focus::Content => self.move_table_selection(-1),
		}
	}

	fn move_table_selection(&mut self, delta: i64) {
		let len = self.app.current_table_len();
		let state = match self.app.tab {
			Tab::Peers => Some(&mut self.app.peers_table),
			Tab::Mining => Some(match self.app.mining_subview {
				MiningSubview::Workers => &mut self.app.mining_workers_table,
				MiningSubview::Difficulty => &mut self.app.mining_diff_table,
			}),
			_ => None,
		};
		if let Some(state) = state {
			state.select(next_table_selection(state.selected(), len, delta));
		}
	}

	fn handle_mouse(&mut self, kind: MouseEventKind, column: u16, row: u16) {
		if self.app.dialog.is_some() {
			return;
		}
		match kind {
			MouseEventKind::Down(MouseButton::Left) => {
				let pos = Position { x: column, y: row };
				if self.app.menu_area.contains(pos) {
					let idx = (row - self.app.menu_area.y) as usize;
					if idx < Tab::ALL.len() {
						self.app.tab = Tab::ALL[idx];
						self.app.focus = Focus::Menu;
					}
				}
			}
			MouseEventKind::ScrollDown => self.move_table_selection(1),
			MouseEventKind::ScrollUp => self.move_table_selection(-1),
			_ => {}
		}
	}

	/// Step the UI: drain pending messages, handle one input event (if any),
	/// then redraw if anything changed (or the periodic refresh is due).
	///
	/// Returns `Ok(false)` when the UI should exit, `Err` on terminal I/O failure.
	pub fn step(&mut self) -> io::Result<bool> {
		if self.app.should_quit {
			return Ok(false);
		}

		if let Some(logs_rx) = &self.logs_rx {
			while let Some(message) = logs_rx.try_iter().next() {
				self.app.push_log(message);
				self.needs_redraw = true;
			}
		}

		if event::poll(Duration::from_millis(50))? {
			match event::read()? {
				Event::Key(key) => {
					if key.kind == KeyEventKind::Press {
						self.handle_key(key.code);
						self.needs_redraw = true;
					}
				}
				Event::Mouse(mouse) => {
					self.handle_mouse(mouse.kind, mouse.column, mouse.row);
					self.needs_redraw = true;
				}
				Event::Resize(_, _) => self.needs_redraw = true,
				_ => {}
			}
		}

		if self.needs_redraw || self.last_draw.elapsed() >= MAX_REDRAW_INTERVAL {
			let app = &mut self.app;
			self.terminal.draw(|f| draw(f, app))?;
			self.needs_redraw = false;
			self.last_draw = Instant::now();
		}
		Ok(true)
	}

	/// Stop the UI and restore the terminal
	pub fn stop(&mut self) {
		self.app.should_quit = true;
		self.restore();
	}

	fn restore(&mut self) {
		if self.restored {
			return;
		}
		self.restored = true;
		restore_terminal();
		let _ = self.terminal.show_cursor();
	}
}

impl Drop for UI {
	fn drop(&mut self) {
		self.restore();
	}
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
	let vertical = Layout::vertical([
		Constraint::Percentage((100 - percent_y) / 2),
		Constraint::Percentage(percent_y),
		Constraint::Percentage((100 - percent_y) / 2),
	])
	.split(r);

	Layout::horizontal([
		Constraint::Percentage((100 - percent_x) / 2),
		Constraint::Percentage(percent_x),
		Constraint::Percentage((100 - percent_x) / 2),
	])
	.split(vertical[1])[1]
}

fn draw_dialog(f: &mut Frame, area: Rect, dialog: &Dialog) {
	let popup_area = centered_rect(60, 20, area);
	let (color, title) = match dialog.kind {
		DialogKind::Info => (Color::Green, ""),
		DialogKind::Error => (Color::Red, "Error (press Enter or q to exit)"),
	};
	let block = Block::default().borders(Borders::ALL).title(title);
	let paragraph = Paragraph::new(dialog.text.clone())
		.style(Style::default().fg(color))
		.block(block)
		.wrap(Wrap { trim: false });
	f.render_widget(Clear, popup_area);
	f.render_widget(paragraph, popup_area);
}

fn draw(f: &mut Frame, app: &mut App) {
	let size = f.area();
	let outer = Layout::vertical([Constraint::Length(3), Constraint::Min(0)]).split(size);

	let title = format!(
		"Grin Version {} [{:?}]",
		built_info::PKG_VERSION,
		global::get_chain_type()
	);
	let title_widget = Paragraph::new(title)
		.style(Style::default().fg(Color::Green))
		.block(Block::default().borders(Borders::ALL));
	f.render_widget(title_widget, outer[0]);

	let body = Layout::horizontal([Constraint::Length(24), Constraint::Min(0)]).split(outer[1]);

	menu::draw(f, body[0], app);

	let content_block = Block::default().borders(Borders::ALL);
	let content_area = content_block.inner(body[1]);
	f.render_widget(content_block, body[1]);

	match app.tab {
		Tab::Status => status::draw(f, content_area, app),
		Tab::Peers => peers::draw(f, content_area, app),
		Tab::Mining => mining::draw(f, content_area, app),
		Tab::Logs => logs::draw(f, content_area, app),
		Tab::Version => version::draw(f, content_area),
	}

	if let Some(dialog) = &app.dialog {
		draw_dialog(f, size, dialog);
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
			ui: UI::new(tx, logs_rx).map_err(|e| e.to_string())?,
			serv_rx,
			server: None,
		})
	}

	/// Server initialization status.
	pub fn init_status(&mut self, text: &str, pop: bool) {
		if pop {
			self.pop_dialog();
		}
		self.ui.app.dialog = Some(Dialog {
			text: text.to_string(),
			kind: DialogKind::Info,
		});
		self.ui.needs_redraw = true;
	}

	/// Server initialization error.
	pub fn init_error(&mut self, e: Error) {
		self.pop_dialog();
		self.ui.app.dialog = Some(Dialog {
			text: format!("{:?}", e),
			kind: DialogKind::Error,
		});
		self.ui.needs_redraw = true;
	}

	fn pop_dialog(&mut self) {
		self.ui.app.dialog = None;
		self.ui.needs_redraw = true;
	}

	/// Stop a fully initialized server, including one queued by the startup thread.
	pub fn stop_server(&mut self) {
		if let Some(s) = self.server.take() {
			s.stop();
		}
		while let Ok(message) = self.serv_rx.try_recv() {
			if let ServerInitStatus::FinishedLoading(s) = message {
				s.stop();
			}
		}
	}

	/// Run the controller
	pub fn run(&mut self) -> i32 {
		self.init_status("Starting server...", false);

		let stat_update_interval = 1;
		let mut next_stat_update = Utc::now().timestamp() + stat_update_interval;
		let mut exit_code = 0;
		loop {
			match self.ui.step() {
				Ok(true) => {}
				Ok(false) => break,
				Err(e) => {
					error!("TUI terminal error: {}", e);
					exit_code = 1;
					break;
				}
			}

			if let Some(message) = self.rx.try_iter().next() {
				return match message {
					ControllerMessage::Shutdown => {
						warn!("Shutdown in progress, please wait");
						self.stop_server();
						self.ui.stop();
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
						self.pop_dialog();
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
				// Same controller thread owns App; update stats directly (no UI channel).
				if let Some(server) = &self.server {
					if let Ok(stats) = server.get_server_stats() {
						self.ui.app.stats = Some(stats);
						self.ui.needs_redraw = true;
					}
				}
			}
		}
		self.stop_server();
		exit_code
	}
}

#[cfg(test)]
mod tests {
	use super::next_table_selection;

	#[test]
	fn first_table_move_anchors_to_row_zero() {
		assert_eq!(next_table_selection(None, 5, 1), Some(0));
		assert_eq!(next_table_selection(None, 5, -1), Some(0));
		assert_eq!(next_table_selection(None, 5, 10), Some(0));
		assert_eq!(next_table_selection(None, 5, i64::MIN), Some(0));
		assert_eq!(next_table_selection(None, 5, i64::MAX), Some(4));
	}

	#[test]
	fn empty_table_clears_selection() {
		assert_eq!(next_table_selection(None, 0, 1), None);
		assert_eq!(next_table_selection(Some(2), 0, 1), None);
	}

	#[test]
	fn table_selection_clamps_and_moves() {
		assert_eq!(next_table_selection(Some(0), 5, 1), Some(1));
		assert_eq!(next_table_selection(Some(4), 5, 1), Some(4));
		assert_eq!(next_table_selection(Some(0), 5, -1), Some(0));
		assert_eq!(next_table_selection(Some(2), 5, i64::MAX), Some(4));
		assert_eq!(next_table_selection(Some(2), 5, i64::MIN), Some(0));
		assert_eq!(next_table_selection(Some(0), 5, 10), Some(4));
	}
}
