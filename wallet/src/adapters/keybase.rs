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

// Keybase Wallet Plugin

use libtx::slate::Slate;
use libwallet::{Error, ErrorKind};
use serde::Serialize;
use serde_json::{from_str, to_string, Value};
use std::collections::{HashMap, HashSet};
use std::process::{Command, Stdio};
use std::str::from_utf8;
use std::thread::sleep;
use std::time::{Duration, Instant};
use {WalletCommAdapter, WalletConfig};
use reqwest;

const TTL: u16 = 60; // TODO: Pass this as a parameter
const SLEEP_DURATION: Duration = Duration::from_millis(5000);

#[derive(Clone)]
pub struct KeybaseWalletCommAdapter {}

impl KeybaseWalletCommAdapter {
	/// Check if keybase is installed and return an adapter object.
	pub fn new() -> Box<WalletCommAdapter> {
		let mut proc = if cfg!(target_os = "windows") {
			Command::new("where")
		} else {
			Command::new("which")
		};
		proc.arg("keybase")
			.stdout(Stdio::null())
			.status()
			.expect("Keybase executable not found, make sure it is installed and in your PATH");

		Box::new(KeybaseWalletCommAdapter {})
	}
}

/// Send a json object to the keybase process. Type `keybase chat api --help` for a list of available methods.
fn api_send(payload: &str) -> Value {
	let mut proc = Command::new("keybase");
	proc.args(&["chat", "api", "-m", &payload]);
	let output = proc.output().expect("No output").stdout;
	let response: Value = from_str(from_utf8(&output).expect("Bad output")).unwrap();
	response
}

/// Get all unread messages from a specific channel and mark as read.
fn read_from_channel(channel: &str) -> Vec<String> {
	let payload = to_string(&json!({
        "method": "read",
        "params": {
            "options": {
                "channel": {
                        "name": channel, "topic_type": "dev"
                    }
                },
                "unread_only": true, "peek": false
            }
        }
    )).unwrap();

	let response = api_send(&payload);
	let mut unread: Vec<String> = Vec::new();
	for msg in response["result"]["messages"].as_array().unwrap().iter() {
		if (msg["msg"]["content"]["type"] == "text") && (msg["msg"]["unread"] == true) {
			let message = msg["msg"]["content"]["text"]["body"].as_str().unwrap();
			unread.push(message.to_owned());
		}
	}
	unread
}

/// Get unread messages from all channels and mark as read.
fn get_unread() -> HashMap<String, String> {
	let payload = to_string(&json!({
        "method": "list",
        "params": {
            "options": {
                    "topic_type": "dev"
                },
                "unread_only": true, "peek": false
            }
        }
    )).unwrap();
	let response = api_send(&payload);

	let mut channels = HashSet::new();
	// Unfortunately the response does not contain the message body 
	// and a seperate call is needed for each channel
	for msg in response["result"]["conversations"]
		.as_array()
		.unwrap()
		.iter()
	{
		if msg["unread"] == true {
			let channel = msg["channel"]["name"].as_str().unwrap();
			channels.insert(channel.to_string());
			println!("Received message from channel {}", channel);
		}
	}
	let mut unread: HashMap<String, String> = HashMap::new();
	for channel in channels.iter() {
		let messages = read_from_channel(channel);
		for msg in messages {
			unread.insert(msg, channel.to_string());
		}
	}
	unread
}

/// Send a message to a keybase channel that self-destructs after ttl seconds.
fn send<T: Serialize>(message: T, channel: &str, ttl: u16) -> bool {
	let seconds = format!("{}s", ttl);
	// TODO: replace with api_send call
	let mut proc = Command::new("keybase");
	let msg = to_string(&message).expect("Serialization error");
	let args = [
		"chat",
		"send",
		"--exploding-lifetime",
		&seconds,
		"--topic-type",
		"dev",
		channel,
		&msg,
	];
	proc.args(&args).stdout(Stdio::null());
	proc.status().is_ok()
}

/// Listen for a message from a specific channel for nseconds and return the first valid slate.
fn poll(nseconds: u64, channel: &str) -> Option<Slate> {
	let start = Instant::now();
	println!("Waiting for message from {}...", channel);
	while start.elapsed().as_secs() < nseconds {
		let unread = read_from_channel(channel);
		for msg in unread.iter() {
			let blob = from_str::<Slate>(msg);
			match blob {
				Ok(slate) => {
					println!("Received message from {}", channel);
					return Some(slate);
				}
				Err(_) => (),
			}
		}
		sleep(SLEEP_DURATION);
	}
	println!(
		"Did not receive reply from {} in {} seconds",
		channel, nseconds
	);
	None
}

/// Send a received slate to grin foreign api for signing and return the response.
pub fn receive_tx(host: &str, slate: &Slate) -> Result<Slate, Error> {
	let url = format!("http://{}/v1/wallet/foreign/receive_tx", host);
	println!{"Signing slate.."};
	let client: reqwest::Client = reqwest::Client::new();
	let mut res = match client.post(&url).json(&slate).send() {
		Ok(r) => r,
		Err(_) => {
			return Err(ErrorKind::WalletComms(format!(
				"Could not connect to {}",
				url
			)))?
		}
	};
	let txt = match res.text() {
		Ok(text) => text,
		Err(_) => return Err(ErrorKind::WalletComms(format!("Bad response from {}", url)))?,
	};

	if !res.status().is_success() {
		return Err(ErrorKind::WalletComms(format!(
			"Status code {} with reason {}",
			res.status().as_u16(),
			txt
		)))?;
	}
	let v = match from_str::<Slate>(&txt) {
		Ok(json) => json,
		Err(_) => return Err(ErrorKind::Format)?,
	};
	Ok(v)
}

impl WalletCommAdapter for KeybaseWalletCommAdapter {
	fn supports_sync(&self) -> bool {
		true
	}

	// Send a slate to a keybase username then wait for a response for TTL seconds.
	fn send_tx_sync(&self, addr: &str, slate: &Slate) -> Result<Slate, Error> {
		match send(slate, addr, TTL) {
			true => (),
			false => return Err(ErrorKind::ClientCallback("Posting transaction slate"))?,
		}
		match poll(TTL as u64, addr) {
			Some(slate) => return Ok(slate),
			None => return Err(ErrorKind::ClientCallback("Receiving reply from recipient"))?,
		}
	}

	/// Send a transaction asynchronously (result will be returned via the listener)
	fn send_tx_async(&self, _addr: &str, _slate: &Slate) -> Result<(), Error> {
		unimplemented!();
	}

	/// Receive a transaction async. (Actually just read it from wherever and return the slate)
	fn receive_tx_async(&self, _params: &str) -> Result<Slate, Error> {
		unimplemented!();
	}

	/// Start a listener, passing received messages to the wallet api directly
	#[allow(unreachable_code)]
	fn listen(
		&self,
		params: HashMap<String, String>,
		_config: WalletConfig,
		_passphrase: &str,
		_account: &str,
		_node_api_secret: Option<String>,
	) -> Result<(), Error> {
		let listen_addr = params.get("api_listen_addr").unwrap();
		println!("Listening for messages via keybase chat...");
		loop {
			let unread = get_unread();
			for (msg, channel) in &unread {

				let blob = from_str::<Slate>(msg);
				match blob {
					Ok(slate) => match receive_tx(listen_addr, &slate) {
						Ok(signed) => {
							match send(signed, channel, TTL) {
								true => { println!("Returned slate to {}", channel); },
								false => { println!("Failed to return slate to {}", channel); }
							}
						}
						Err(e) => {
							println!("Error : {}", e);
						}
					},
					Err(_) => (),
				}
			}
			sleep(SLEEP_DURATION);
		}
		Ok(())
	}
}
