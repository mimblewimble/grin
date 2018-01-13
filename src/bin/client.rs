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

extern crate term;

use api;
use grin::ServerConfig;

pub fn show_status(config: &ServerConfig) {
	println!();
	let title = format!("Grin Server Status ");
	let mut t = term::stdout().unwrap();
	let mut e = term::stdout().unwrap();
	t.fg(term::color::MAGENTA).unwrap();
	writeln!(t, "{}", title).unwrap();
	writeln!(t, "--------------------------").unwrap();
	t.reset().unwrap();
	match get_status_from_node(config) {
		Ok(status) => {
			writeln!(e, "Protocol version: {}", status.protocol_version).unwrap();
            writeln!(e, "User agent: {}", status.user_agent).unwrap();
			writeln!(e, "Connections: {}", status.connections).unwrap();
			writeln!(e, "Chain height: {}", status.tip.height).unwrap();
			writeln!(e, "Last block hash: {}", status.tip.last_block_pushed).unwrap();
			writeln!(e, "Previous block hash: {}", status.tip.prev_block_to_last).unwrap();
			writeln!(e, "Total difficulty: {}", status.tip.total_difficulty).unwrap()
		}
		Err(_) => writeln!(
			e,
			"WARNING: Client failed to get data. Is your `grin server` offline or broken?"
		).unwrap(),
	};
	e.reset().unwrap();
	println!();
}

fn get_status_from_node(config: &ServerConfig) -> Result<api::Status, Error> {
	let url = format!("http://{}/v1/status", config.api_http_addr);
	api::client::get::<api::Status>(url.as_str()).map_err(|e| Error::API(e))
}

/// Error type wrapping underlying module errors.
#[derive(Debug)]
enum Error {
	/// Error originating from HTTP API calls.
	API(api::Error),
}
