// Copyright 2018 The Grin Developers
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

//! Logging wrapper to be used throughout all crates in the workspace
use crate::Mutex;
use std::ops::Deref;

use backtrace::Backtrace;
use std::{panic, thread};

use crate::types::{LogLevel, LoggingConfig};

use log::{LevelFilter, Record};
use log4rs;
use log4rs::append::console::ConsoleAppender;
use log4rs::append::file::FileAppender;
use log4rs::append::rolling_file::{
	policy::compound::roll::fixed_window::FixedWindowRoller,
	policy::compound::trigger::size::SizeTrigger, policy::compound::CompoundPolicy,
	RollingFileAppender,
};
use log4rs::append::Append;
use log4rs::config::{Appender, Config, Root};
use log4rs::encode::pattern::PatternEncoder;
use log4rs::filter::{threshold::ThresholdFilter, Filter, Response};

fn convert_log_level(in_level: &LogLevel) -> LevelFilter {
	match *in_level {
		LogLevel::Info => LevelFilter::Info,
		LogLevel::Warning => LevelFilter::Warn,
		LogLevel::Debug => LevelFilter::Debug,
		LogLevel::Trace => LevelFilter::Trace,
		LogLevel::Error => LevelFilter::Error,
	}
}

lazy_static! {
	/// Flag to observe whether logging was explicitly initialised (don't output otherwise)
	static ref WAS_INIT: Mutex<bool> = Mutex::new(false);
	/// Flag to observe whether tui is running, and we therefore don't want to attempt to write
	/// panics to stdout
	static ref TUI_RUNNING: Mutex<bool> = Mutex::new(false);
	/// Static Logging configuration, should only be set once, before first logging call
	static ref LOGGING_CONFIG: Mutex<LoggingConfig> = Mutex::new(LoggingConfig::default());
}

const LOGGING_PATTERN: &str = "{d(%Y%m%d %H:%M:%S%.3f)} {h({l})} {M} - {m}{n}";

/// This filter is rejecting messages that doesn't start with "grin"
/// in order to save log space for only Grin-related records
#[derive(Debug)]
struct GrinFilter;

impl Filter for GrinFilter {
	fn filter(&self, record: &Record<'_>) -> Response {
		if let Some(module_path) = record.module_path() {
			if module_path.starts_with("grin") {
				return Response::Neutral;
			}
		}

		Response::Reject
	}
}

/// Initialize the logger with the given configuration
pub fn init_logger(config: Option<LoggingConfig>) {
	if let Some(c) = config {
		let tui_running = c.tui_running.unwrap_or(false);
		if tui_running {
			let mut tui_running_ref = TUI_RUNNING.lock();
			*tui_running_ref = true;
		}

		// Save current logging configuration
		let mut config_ref = LOGGING_CONFIG.lock();
		*config_ref = c.clone();

		let level_stdout = convert_log_level(&c.stdout_log_level);
		let level_file = convert_log_level(&c.file_log_level);
		let level_minimum;

		// Determine minimum logging level for Root logger
		if level_stdout > level_file {
			level_minimum = level_stdout;
		} else {
			level_minimum = level_file;
		}

		// Start logger
		let stdout = ConsoleAppender::builder()
			.encoder(Box::new(PatternEncoder::new(&LOGGING_PATTERN)))
			.build();

		let mut root = Root::builder();

		let mut appenders = vec![];

		if c.log_to_stdout && !tui_running {
			let filter = Box::new(ThresholdFilter::new(level_stdout));
			appenders.push(
				Appender::builder()
					.filter(filter)
					.filter(Box::new(GrinFilter))
					.build("stdout", Box::new(stdout)),
			);

			root = root.appender("stdout");
		}

		if c.log_to_file {
			// If maximum log size is specified, use rolling file appender
			// or use basic one otherwise
			let filter = Box::new(ThresholdFilter::new(level_file));
			let file: Box<dyn Append> = {
				if let Some(size) = c.log_max_size {
					let roller = FixedWindowRoller::builder()
						.build(&format!("{}.{{}}.gz", c.log_file_path), 32)
						.unwrap();
					let trigger = SizeTrigger::new(size);

					let policy = CompoundPolicy::new(Box::new(trigger), Box::new(roller));

					Box::new(
						RollingFileAppender::builder()
							.append(c.log_file_append)
							.encoder(Box::new(PatternEncoder::new(&LOGGING_PATTERN)))
							.build(c.log_file_path, Box::new(policy))
							.unwrap(),
					)
				} else {
					Box::new(
						FileAppender::builder()
							.append(c.log_file_append)
							.encoder(Box::new(PatternEncoder::new(&LOGGING_PATTERN)))
							.build(c.log_file_path)
							.unwrap(),
					)
				}
			};

			appenders.push(
				Appender::builder()
					.filter(filter)
					.filter(Box::new(GrinFilter))
					.build("file", file),
			);
			root = root.appender("file");
		}

		let config = Config::builder()
			.appenders(appenders)
			.build(root.build(level_minimum))
			.unwrap();

		let _ = log4rs::init_config(config).unwrap();

		info!(
			"log4rs is initialized, file level: {:?}, stdout level: {:?}, min. level: {:?}",
			level_file, level_stdout, level_minimum
		);

		// Mark logger as initialized
		let mut was_init_ref = WAS_INIT.lock();
		*was_init_ref = true;
	}

	send_panic_to_log();
}

/// Initializes the logger for unit and integration tests
pub fn init_test_logger() {
	let mut was_init_ref = WAS_INIT.lock();
	if *was_init_ref.deref() {
		return;
	}
	let mut logger = LoggingConfig::default();
	logger.log_to_file = false;
	logger.stdout_log_level = LogLevel::Debug;

	// Save current logging configuration
	let mut config_ref = LOGGING_CONFIG.lock();
	*config_ref = logger;

	let level_stdout = convert_log_level(&config_ref.stdout_log_level);
	let level_minimum = level_stdout; // minimum logging level for Root logger

	// Start logger
	let stdout = ConsoleAppender::builder()
		.encoder(Box::new(PatternEncoder::default()))
		.build();

	let mut root = Root::builder();

	let mut appenders = vec![];

	{
		let filter = Box::new(ThresholdFilter::new(level_stdout));
		appenders.push(
			Appender::builder()
				.filter(filter)
				.filter(Box::new(GrinFilter))
				.build("stdout", Box::new(stdout)),
		);

		root = root.appender("stdout");
	}

	let config = Config::builder()
		.appenders(appenders)
		.build(root.build(level_minimum))
		.unwrap();

	let _ = log4rs::init_config(config).unwrap();

	info!(
		"log4rs is initialized, stdout level: {:?}, min. level: {:?}",
		level_stdout, level_minimum
	);

	*was_init_ref = true;
}

/// hook to send panics to logs as well as stderr
fn send_panic_to_log() {
	panic::set_hook(Box::new(|info| {
		let backtrace = Backtrace::new();

		let thread = thread::current();
		let thread = thread.name().unwrap_or("unnamed");

		let msg = match info.payload().downcast_ref::<&'static str>() {
			Some(s) => *s,
			None => match info.payload().downcast_ref::<String>() {
				Some(s) => &**s,
				None => "Box<Any>",
			},
		};

		match info.location() {
			Some(location) => {
				error!(
					"\nthread '{}' panicked at '{}': {}:{}{:?}\n\n",
					thread,
					msg,
					location.file(),
					location.line(),
					backtrace
				);
			}
			None => error!("thread '{}' panicked at '{}'{:?}", thread, msg, backtrace),
		}
		//also print to stderr
		let tui_running = TUI_RUNNING.lock().clone();
		if !tui_running {
			let config = LOGGING_CONFIG.lock();

			eprintln!(
				"Thread '{}' panicked with message:\n\"{}\"\nSee {} for further details.",
				thread, msg, config.log_file_path
			);
		}
	}));
}
