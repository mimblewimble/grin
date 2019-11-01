// Copyright 2019 The Grin Developers
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

/// whether to log to stdout
const LOGGING_LOG_TO_STDOUT: bool = true;

/// logging level for stdout
const LOGGING_STDOUT_LOG_LEVEL: LogLevel = LogLevel::Warning;

/// whether to log to file
const LOGGING_LOG_TO_FILE: bool = true;

/// log file level
const LOGGING_FILE_LOG_LEVEL: LogLevel = LogLevel::Info;

/// Log file path
const LOGGING_LOG_FILE_PATH: &str = "grin.log";

/// Whether to append to log file or replace
const LOGGING_LOG_FILE_APPEND: bool = true;

/// Size of the log in bytes to rotate over (optional)
const LOGGING_LOG_MAX_SIZE: u64 = 1024 * 1024 * 16; // 16 megabytes default

/// 32 log files to rotate over by default
const LOGGING_ROTATE_LOG_FILES: u32 = 32 as u32;

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
	#[serde(default = "default_logging_log_to_stdout")]
	pub log_to_stdout: bool,
	/// logging level for stdout
	#[serde(default = "default_logging_stdout_log_level")]
	pub stdout_log_level: LogLevel,
	/// whether to log to file
	#[serde(default = "default_logging_log_to_file")]
	pub log_to_file: bool,
	/// log file level
	#[serde(default = "default_logging_file_log_level")]
	pub file_log_level: LogLevel,
	/// Log file path
	#[serde(default = "default_logging_log_file_path")]
	pub log_file_path: String,
	/// Whether to append to log or replace
	#[serde(default = "default_logging_log_file_append")]
	pub log_file_append: bool,
	/// Size of the log in bytes to rotate over (optional)
	#[serde(default = "default_logging_log_max_size")]
	pub log_max_size: Option<u64>,
	/// Number of the log files to rotate over (optional)
	#[serde(default = "default_logging_log_max_files")]
	pub log_max_files: u32,
	/// Whether the tui is running (optional)
	#[serde(default = "default_logging_tui_running")]
	pub tui_running: Option<bool>,
}

impl Default for LoggingConfig {
	fn default() -> LoggingConfig {
		LoggingConfig {
			log_to_stdout: default_logging_log_to_stdout(),
			stdout_log_level: default_logging_stdout_log_level(),
			log_to_file: default_logging_log_to_file(),
			file_log_level: default_logging_file_log_level(),
			log_file_path: default_logging_log_file_path(),
			log_file_append: default_logging_log_file_append(),
			log_max_size: default_logging_log_max_size(),
			log_max_files: default_logging_log_max_files(),
			tui_running: default_logging_tui_running(),
		}
	}
}

fn default_logging_log_to_stdout() -> bool {
	LOGGING_LOG_TO_STDOUT
}

fn default_logging_stdout_log_level() -> LogLevel {
	LOGGING_STDOUT_LOG_LEVEL
}

fn default_logging_log_to_file() -> bool {
	LOGGING_LOG_TO_FILE
}

fn default_logging_file_log_level() -> LogLevel {
	LOGGING_FILE_LOG_LEVEL
}

fn default_logging_log_file_path() -> String {
	LOGGING_LOG_FILE_PATH.to_string()
}

fn default_logging_log_file_append() -> bool {
	LOGGING_LOG_FILE_APPEND
}

fn default_logging_log_max_size() -> Option<u64> {
	Some(LOGGING_LOG_MAX_SIZE)
}

fn default_logging_log_max_files() -> u32 {
	LOGGING_ROTATE_LOG_FILES
}

fn default_logging_tui_running() -> Option<bool> {
	None
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
