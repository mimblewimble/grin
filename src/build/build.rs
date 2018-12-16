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

use built;

use reqwest;

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
	// don't fail the build if something's missing, may just be cargo release
	let _ = built::write_built_file_with_opts(
		&opts,
		env!("CARGO_MANIFEST_DIR"),
		format!("{}{}", env::var("OUT_DIR").unwrap(), "/built.rs"),
	);

	let web_wallet_install = install_web_wallet();
	match web_wallet_install {
		Ok(true) => {}
		_ => println!(
			"WARNING : Web wallet could not be installed due to {:?}",
			web_wallet_install
		),
	}
}

fn download_and_decompress(target_file: &str) -> Result<bool, Box<std::error::Error>> {
	let req_path = format!("https://github.com/mimblewimble/grin-web-wallet/releases/download/{}/grin-web-wallet.tar.gz", WEB_WALLET_TAG);
	let mut resp = reqwest::get(&req_path)?;

	if !resp.status().is_success() {
		return Ok(false);
	}

	// read response
	let mut out: Vec<u8> = vec![];
	resp.read_to_end(&mut out)?;

	// Gunzip
	let mut d = GzDecoder::new(&out[..]);
	let mut decomp: Vec<u8> = vec![];
	d.read_to_end(&mut decomp)?;

	// write temp file
	let mut buffer = File::create(target_file.clone())?;
	buffer.write_all(&decomp)?;
	buffer.flush()?;

	Ok(true)
}

/// Download and unzip tagged web-wallet build
fn install_web_wallet() -> Result<bool, Box<std::error::Error>> {
	let target_file = format!(
		"{}/grin-web-wallet-{}.tar",
		env::var("OUT_DIR")?,
		WEB_WALLET_TAG
	);
	let out_dir = env::var("OUT_DIR")?;
	let mut out_path = PathBuf::from(&out_dir);
	out_path.pop();
	out_path.pop();
	out_path.pop();

	// only re-download if needed
	println!("{}", target_file);
	if !Path::new(&target_file).is_file() {
		let success = download_and_decompress(&target_file)?;
		if !success {
			return Ok(false); // could not download and decompress
		}
	}

	// remove old version
	let mut remove_path = out_path.clone();
	remove_path.push("grin-wallet");
	let _ = fs::remove_dir_all(remove_path);

	// Untar
	let file = File::open(target_file)?;
	let mut a = Archive::new(file);

	for file in a.entries()? {
		let mut file = file?;
		let h = file.header().clone();
		let path = h.path()?.clone().into_owned();
		let is_dir = path.to_str().unwrap().ends_with(path::MAIN_SEPARATOR);
		let path = path.strip_prefix("dist")?;
		let mut final_path = out_path.clone();
		final_path.push(path);

		let mut tmp: Vec<u8> = vec![];
		file.read_to_end(&mut tmp)?;
		if is_dir {
			fs::create_dir_all(final_path)?;
		} else {
			let mut buffer = File::create(final_path)?;
			buffer.write_all(&tmp)?;
			buffer.flush()?;
		}
	}

	Ok(true)
}
