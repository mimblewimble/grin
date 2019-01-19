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

//! Logging configuration types

/// Log level types
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum LogLevel {
	/// Error
	Error,
	/// Warning
	Warning,
	/// Info
	Info,
	/// Debug
	Debug,
	/// Trace
	Trace,
}

/// Logging config
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LoggingConfig {
	/// whether to log to stdout
	pub log_to_stdout: bool,
	/// logging level for stdout
	pub stdout_log_level: LogLevel,
	/// whether to log to file
	pub log_to_file: bool,
	/// log file level
	pub file_log_level: LogLevel,
	/// Log file path
	pub log_file_path: String,
	/// Whether to append to log or replace
	pub log_file_append: bool,
	/// Size of the log in bytes to rotate over (optional)
	pub log_max_size: Option<u64>,
	/// Whether the tui is running (optional)
	pub tui_running: Option<bool>,
}

impl Default for LoggingConfig {
	fn default() -> LoggingConfig {
		LoggingConfig {
			log_to_stdout: true,
			stdout_log_level: LogLevel::Warning,
			log_to_file: true,
			file_log_level: LogLevel::Info,
			log_file_path: String::from("grin.log"),
			log_file_append: true,
			log_max_size: Some(1024 * 1024 * 16), // 16 megabytes default
			tui_running: None,
		}
	}
}

use std::ops::Deref;
use zeroize::Zeroize;
/// Zeroing string, mainly useful for password
#[derive(Clone, PartialEq, PartialOrd)]
pub struct ZeroingString(String);

impl Drop for ZeroingString {
	fn drop(&mut self) {
		self.0.zeroize();
	}
}

impl From<&str> for ZeroingString {
	fn from(s: &str) -> Self {
		ZeroingString(String::from(s))
	}
}

impl From<String> for ZeroingString {
	fn from(s: String) -> Self {
		ZeroingString(s)
	}
}

impl Deref for ZeroingString {
	type Target = str;

	fn deref(&self) -> &str {
		&self.0
	}
}
