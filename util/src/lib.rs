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

extern crate backtrace;
extern crate base64;
extern crate byteorder;
extern crate rand;
#[macro_use]
extern crate slog;
extern crate slog_async;
extern crate slog_term;

#[macro_use]
extern crate lazy_static;

extern crate serde;
#[macro_use]
extern crate serde_derive;
extern crate walkdir;
extern crate zip as zip_rs;

// Re-export so only has to be included once
pub extern crate secp256k1zkp as secp_;
pub use secp_ as secp;

// Logging related
pub mod logger;
pub use logger::{init_logger, init_test_logger, LOGGER};

// Static secp instance
pub mod secp_static;
pub use secp_static::static_secp_instance;

pub mod types;
pub use types::{LogLevel, LoggingConfig};

pub mod macros;

// other utils
use byteorder::{BigEndian, ByteOrder};
#[allow(unused_imports)]
use std::ops::Deref;
use std::sync::{Arc, RwLock};

mod hex;
pub use hex::*;

/// File util
pub mod file;
/// Compress and decompress zip bz2 archives
pub mod zip;

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
		let mut inner = self.inner.write().unwrap();
		assert!(inner.is_none());
		*inner = Some(value);
	}

	/// Borrows the OneTime, should only be called after initialization.
	/// Will panic (via expect) if called before initialization.
	pub fn borrow(&self) -> T {
		let inner = self.inner.read().unwrap();
		inner
			.clone()
			.expect("Cannot borrow one_time before initialization.")
	}
}

/// Construct msg bytes from tx fee and lock_height
pub fn kernel_sig_msg(fee: u64, lock_height: u64) -> [u8; 32] {
	let mut bytes = [0; 32];
	BigEndian::write_u64(&mut bytes[16..24], fee);
	BigEndian::write_u64(&mut bytes[24..], lock_height);
	bytes
}

/// Encode an utf8 string to a base64 string
pub fn to_base64(s: &str) -> String {
	base64::encode(s)
}
