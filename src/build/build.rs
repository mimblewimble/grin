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
extern crate flate2;
extern crate reqwest;
extern crate tar;

use flate2::read::GzDecoder;
use std::env;
use std::fs::{self, File};
use std::io::prelude::*;
use std::io::Read;
use std::path::{self, Path, PathBuf};
use std::process::Command;

use tar::Archive;

const WEB_WALLET_TAG: &str = "0.3.0.1";

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
	built::write_built_file_with_opts(
		&opts,
		env!("CARGO_MANIFEST_DIR"),
		format!("{}{}", env::var("OUT_DIR").unwrap(), "/built.rs"),
	).expect("Failed to acquire build-time information");

	install_web_wallet();
}

fn download_and_decompress(target_file: &str) {
	let req_path = format!("https://github.com/mimblewimble/grin-web-wallet/releases/download/{}/grin-web-wallet.tar.gz", WEB_WALLET_TAG);
	let resp = reqwest::get(&req_path);

	// don't interfere if this doesn't work
	if resp.is_err() {
		println!("Warning: Failed to download grin-web-wallet. Web wallet will not be available");
		return;
	}

	let mut resp = resp.unwrap();
	if resp.status().is_success() {
		// read response
		let mut out: Vec<u8> = vec![];
		let r2 = resp.read_to_end(&mut out);
		if r2.is_err() {
			println!(
				"Warning: Failed to download grin-web-wallet. Web wallet will not be available"
			);
			return;
		}

		// Gunzip
		let mut d = GzDecoder::new(&out[..]);
		let mut decomp: Vec<u8> = vec![];
		d.read_to_end(&mut decomp).unwrap();

		// write temp file
		let mut buffer = File::create(target_file.clone()).unwrap();
		buffer.write_all(&decomp).unwrap();
		buffer.flush().unwrap();
	}
}

/// Download and unzip tagged web-wallet build
fn install_web_wallet() {
	let target_file = format!(
		"{}/grin-web-wallet-{}.tar",
		env::var("OUT_DIR").unwrap(),
		WEB_WALLET_TAG
	);
	let out_dir = env::var("OUT_DIR").unwrap();
	let mut out_path = PathBuf::from(&out_dir);
	out_path.pop();
	out_path.pop();
	out_path.pop();

	// only re-download if needed
	if !Path::new(&target_file).is_file() {
		download_and_decompress(&target_file);
	}

	// remove old version
	let mut remove_path = out_path.clone();
	remove_path.push("grin-wallet");
	let _ = fs::remove_dir_all(remove_path);

	// Untar
	let file = File::open(target_file).unwrap();
	let mut a = Archive::new(file);

	for file in a.entries().unwrap() {
		let mut file = file.unwrap();
		let h = file.header().clone();
		let path = h.path().unwrap().clone().into_owned();
		let is_dir = path.to_str().unwrap().ends_with(path::MAIN_SEPARATOR);
		let path = path.strip_prefix("dist").unwrap();
		let mut final_path = out_path.clone();
		final_path.push(path);

		let mut tmp: Vec<u8> = vec![];
		file.read_to_end(&mut tmp).unwrap();
		if is_dir {
			fs::create_dir_all(final_path).unwrap();
		} else {
			let mut buffer = File::create(final_path).unwrap();
			buffer.write_all(&tmp).unwrap();
			buffer.flush().unwrap();
		}
	}
}
