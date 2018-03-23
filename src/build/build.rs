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

//! Build hooks to spit out version+build time info

extern crate built;

use std::env;

fn main() {
	let mut opts = built::Options::default();
	opts.set_dependencies(true);
	built::write_built_file_with_opts(
		&opts,
		env!("CARGO_MANIFEST_DIR"),
		format!("{}{}", env::var("OUT_DIR").unwrap(), "/built.rs"),
	).expect("Failed to acquire build-time information");
}
