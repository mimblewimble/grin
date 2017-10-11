// Copyright 2017 The Grin Developers
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

use slog::{Logger, Drain};
use slog_term;
use slog_async;

lazy_static! {
	pub static ref LOGGER: Logger = {
		let decorator = slog_term::TermDecorator::new().build();
		let drain = slog_term::FullFormat::new(decorator).build().fuse();
		let drain = slog_async::Async::new(drain).build().fuse();
		let log = Logger::root(drain, o!());
		log 
  };
}
