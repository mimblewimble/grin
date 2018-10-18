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
use std::ops::Deref;
use Mutex;

use backtrace::Backtrace;
use std::{panic, thread};

use types::{LogLevel, LoggingConfig};

use flexi_logger::{Duplicate, LevelFilter, LogSpecBuilder, Logger};

fn convert_log_level(in_level: &LogLevel) -> Duplicate {
	match *in_level {
		LogLevel::Info => Duplicate::Info,
		LogLevel::Critical => Duplicate::Error,
		LogLevel::Warning => Duplicate::Warn,
		LogLevel::Debug => Duplicate::Debug,
		LogLevel::Trace => Duplicate::Trace,
		LogLevel::Error => Duplicate::Error,
	}
}

fn convert_log_level_for_spec(in_level: &LogLevel) -> LevelFilter {
	match *in_level {
		LogLevel::Info => LevelFilter::Info,
		LogLevel::Critical => LevelFilter::Error,
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

/// Initialize the logger with the given configuration
pub fn init_logger(config: Option<LoggingConfig>) {
	if let Some(c) = config {
		let level_stdout = convert_log_level(&c.stdout_log_level);
		let level_file = convert_log_level_for_spec(&c.file_log_level);

		// Start logger
		let mut builder = LogSpecBuilder::new();
		let spec = builder.default(level_file).build();
		Logger::with(spec)
			.log_to_file()
			.duplicate_to_stderr(if c.log_to_stdout {
				level_stdout
			} else {
				Duplicate::None
			}).directory("grin_logs")
			.o_append(c.log_file_append)
			.o_rotate_over_size(c.log_rotate_over_size)
			.start()
			.expect("Unable to start logger");

		// Logger configuration successfully injected into LOGGING_CONFIG...
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
	let mut config_ref = LOGGING_CONFIG.lock();
	*config_ref = logger;
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
			eprintln!(
				"Thread '{}' panicked with message:\n\"{}\"\nSee grin.log for further details.",
				thread, msg
			);
		}
	}));
}
