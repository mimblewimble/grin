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

//! Logging, as well as various low-level utilities that factor Rust
//! patterns that are frequent within the grin codebase.

#![deny(non_upper_case_globals)]
#![deny(non_camel_case_types)]
#![deny(non_snake_case)]
#![deny(unused_mut)]
#![warn(missing_docs)]

#[macro_use]
extern crate log;
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate serde_derive;
// Re-export so only has to be included once
pub use parking_lot::Mutex;
pub use parking_lot::RwLock;

// Re-export so only has to be included once
pub use secp256k1zkp as secp;

// Logging related
pub mod logger;
pub use crate::logger::{init_logger, init_test_logger};

// Static secp instance
pub mod secp_static;
pub use crate::secp_static::static_secp_instance;

pub mod types;
pub use crate::types::{LogLevel, LoggingConfig, ZeroingString};

pub mod macros;

// read_exact and write_all impls
pub mod read_write;

// other utils
#[allow(unused_imports)]
use std::ops::Deref;
use std::sync::Arc;

mod hex;
pub use crate::hex::*;

/// File util
pub mod file;
/// Compress and decompress zip bz2 archives
pub mod zip;

mod rate_counter;
pub use crate::rate_counter::RateCounter;

/// Encapsulation of a RwLock<Option<T>> for one-time initialization.
/// This implementation will purposefully fail hard if not used
/// properly, for example if not initialized before being first used
/// (borrowed).
#[derive(Clone)]
pub struct OneTime<T> {
	/// The inner value.
	inner: Arc<RwLock<Option<T>>>,
}

impl<T> OneTime<T>
where
	T: Clone,
{
	/// Builds a new uninitialized OneTime.
	pub fn new() -> OneTime<T> {
		OneTime {
			inner: Arc::new(RwLock::new(None)),
		}
	}

	/// Initializes the OneTime, should only be called once after construction.
	/// Will panic (via assert) if called more than once.
	pub fn init(&self, value: T) {
		let mut inner = self.inner.write();
		assert!(inner.is_none());
		*inner = Some(value);
	}

	/// Borrows the OneTime, should only be called after initialization.
	/// Will panic (via expect) if called before initialization.
	pub fn borrow(&self) -> T {
		let inner = self.inner.read();
		inner
			.clone()
			.expect("Cannot borrow one_time before initialization.")
	}
}

/// Encode an utf8 string to a base64 string
pub fn to_base64(s: &str) -> String {
	base64::encode(s)
}

/// Global stopped/paused state shared across various subcomponents of Grin.
///
/// Arc<Mutex<StopState>> allows the chain to lock the stop_state during critical processing.
/// Other subcomponents cannot abruptly shutdown the server during block/header processing.
/// This should prevent the chain ever ending up in an inconsistent state on restart.
///
/// "Stopped" allows a clean shutdown of the Grin server.
/// "Paused" is used in some tests to allow nodes to reach steady state etc.
///
pub struct StopState {
	stopped: bool,
	paused: bool,
}

impl StopState {
	/// Create a new stop_state in default "running" state.
	pub fn new() -> StopState {
		StopState {
			stopped: false,
			paused: false,
		}
	}

	/// Check if we are stopped.
	pub fn is_stopped(&self) -> bool {
		self.stopped
	}

	/// Check if we are paused.
	pub fn is_paused(&self) -> bool {
		self.paused
	}

	/// Stop the server.
	pub fn stop(&mut self) {
		self.stopped = true;
	}

	/// Pause the server (only used in tests).
	pub fn pause(&mut self) {
		self.paused = true;
	}

	/// Resume a paused server (only used in tests).
	pub fn resume(&mut self) {
		self.paused = false;
	}
}
