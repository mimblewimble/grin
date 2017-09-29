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

/// Various low-level utilities that factor Rust patterns that are frequent
/// within the grin codebase.

use std::cell::{RefCell, Ref};
#[allow(unused_imports)]
use std::ops::Deref;

mod hex;
pub use hex::*;

// Encapsulation of a RefCell<Option<T>> for one-time initialization after
// construction. This implementation will purposefully fail hard if not used
// properly, for example if it's not initialized before being first used
// (borrowed).
#[derive(Clone)]
pub struct OneTime<T> {
	inner: RefCell<Option<T>>,
}

unsafe impl<T> Sync for OneTime<T> {}
unsafe impl<T> Send for OneTime<T> {}

impl<T> OneTime<T> {
	/// Builds a new uninitialized OneTime.
	pub fn new() -> OneTime<T> {
		OneTime { inner: RefCell::new(None) }
	}

	/// Initializes the OneTime, should only be called once after construction.
	pub fn init(&self, value: T) {
		let mut inner_mut = self.inner.borrow_mut();
		*inner_mut = Some(value);
	}

	/// Borrows the OneTime, should only be called after initialization.
	pub fn borrow(&self) -> Ref<T> {
		Ref::map(self.inner.borrow(), |o| o.as_ref().unwrap())
	}
}
