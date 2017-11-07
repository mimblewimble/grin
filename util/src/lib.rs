// Copyright 2016 The Grin Developers
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
extern crate slog;
extern crate slog_async;
extern crate slog_term;

extern crate rand;

#[macro_use]
extern crate lazy_static;

extern crate serde;
#[macro_use]
extern crate serde_derive;

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
pub use types::LoggingConfig;

// other utils
use std::cell::{Ref, RefCell};
#[allow(unused_imports)]
use std::ops::Deref;

mod hex;
pub use hex::*;

/// Encapsulation of a RefCell<Option<T>> for one-time initialization after
/// construction. This implementation will purposefully fail hard if not used
/// properly, for example if it's not initialized before being first used
/// (borrowed).
#[derive(Clone)]
pub struct OneTime<T> {
	/// inner
	inner: RefCell<Option<T>>,
}

unsafe impl<T> Sync for OneTime<T> {}
unsafe impl<T> Send for OneTime<T> {}

impl<T> OneTime<T> {
	/// Builds a new uninitialized OneTime.
	pub fn new() -> OneTime<T> {
		OneTime {
			inner: RefCell::new(None),
		}
	}

	/// Initializes the OneTime, should only be called once after construction.
	pub fn init(&self, value: T) {
		let mut inner_mut = self.inner.borrow_mut();
		*inner_mut = Some(value);
	}

	/// Whether the OneTime has been initialized
	pub fn is_initialized(&self) -> bool {
		let inner = self.inner.borrow();
		inner.is_some()
	}

	/// Borrows the OneTime, should only be called after initialization.
	pub fn borrow(&self) -> Ref<T> {
		Ref::map(self.inner.borrow(), |o| o.as_ref().unwrap())
	}
}
