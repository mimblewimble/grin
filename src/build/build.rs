// Copyright 2021 The Grin Developers
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

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
	// Setting up git hooks in the project: rustfmt and so on.
	let git_hooks = format!(
		"git config core.hooksPath {}",
		PathBuf::from("./.hooks").to_str().unwrap()
	);

	if cfg!(target_os = "windows") {
		Command::new("cmd")
			.args(&["/C", &git_hooks])
			.output()
			.expect("failed to execute git config for hooks");
	} else {
		Command::new("sh")
			.args(&["-c", &git_hooks])
			.output()
			.expect("failed to execute git config for hooks");
	}

	// build and versioning information
	let mut opts = built::Options::default();
	opts.set_dependencies(true);
	let out_dir_path = format!("{}{}", env::var("OUT_DIR").unwrap(), "/built.rs");
	// don't fail the build if something's missing, may just be cargo release
	let _ = built::write_built_file_with_opts(
		&opts,
		Path::new(env!("CARGO_MANIFEST_DIR")),
		Path::new(&out_dir_path),
	);
}
