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

use controller;
use core::libtx::slate::Slate;
use failure::ResultExt;
use libwallet::{Error, ErrorKind};
use serde::Serialize;
use serde_json::{from_str, to_string, Value};
use std::collections::{HashMap, HashSet};
use std::process::{Command, Stdio};
use std::str::from_utf8;
use std::thread::sleep;
use std::time::{Duration, Instant};
use {instantiate_wallet, HTTPNodeClient, WalletCommAdapter, WalletConfig};

const TTL: u16 = 60; // TODO: Pass this as a parameter
const SLEEP_DURATION: Duration = Duration::from_millis(5000);

// Which topic names to use for communication
const SLATE_NEW: &str = "grin_slate_new";
const SLATE_SIGNED: &str = "grin_slate_signed";

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
	let response: Value = from_str(from_utf8(&output).expect("Bad output")).expect("Bad output");
	response
}

/// Get all unread messages from a specific channel/topic and mark as read.
fn read_from_channel(channel: &str, topic: &str) -> Vec<String> {
	let payload = to_string(&json!({
        "method": "read",
        "params": {
            "options": {
                "channel": {
                        "name": channel, "topic_type": "dev", "topic_name": topic
                    },
					"unread_only": true, "peek": false
                },
            }
        }
    )).unwrap();

	let response = api_send(&payload);
	let mut unread: Vec<String> = Vec::new();
	for msg in response["result"]["messages"]
		.as_array()
		.unwrap_or(&vec![json!({})])
		.iter()
	{
		if (msg["msg"]["content"]["type"] == "text") && (msg["msg"]["unread"] == true) {
			let message = msg["msg"]["content"]["text"]["body"].as_str().unwrap_or("");
			unread.push(message.to_owned());
		}
	}
	unread
}

/// Get unread messages from all channels and mark as read.
fn get_unread(topic: &str) -> HashMap<String, String> {
	let payload = to_string(&json!({
        "method": "list",
        "params": {
            "options": {
                    "topic_type": "dev",
                },
            }
        }
    )).unwrap();
	let response = api_send(&payload);

	let mut channels = HashSet::new();
	// Unfortunately the response does not contain the message body
	// and a seperate call is needed for each channel
	for msg in response["result"]["conversations"]
		.as_array()
		.unwrap_or(&vec![json!({})])
		.iter()
	{
		if (msg["unread"] == true) && (msg["channel"]["topic_name"] == topic) {
			let channel = msg["channel"]["name"].as_str().unwrap();
			channels.insert(channel.to_string());
		}
	}
	let mut unread: HashMap<String, String> = HashMap::new();
	for channel in channels.iter() {
		let messages = read_from_channel(channel, topic);
		for msg in messages {
			unread.insert(msg, channel.to_string());
		}
	}
	unread
}

/// Send a message to a keybase channel that self-destructs after ttl seconds.
fn send<T: Serialize>(message: T, channel: &str, topic: &str, ttl: u16) -> bool {
	let seconds = format!("{}s", ttl);
	let payload = to_string(&json!({
		"method": "send",
		"params": {
			"options": {
				"channel": {
						"name": channel, "topic_name": topic, "topic_type": "dev"
					},
						"message": {
								"body": to_string(&message).unwrap()
							},
							"exploding_lifetime": seconds
						}
					}
				}
	)).unwrap();
	let response = api_send(&payload);
	match response["result"]["message"].as_str() {
		Some("message sent") => true,
		_ => false,
	}
}

/// Listen for a message from a specific channel with topic SLATE_SIGNED for nseconds and return the first valid slate.
fn poll(nseconds: u64, channel: &str) -> Option<Slate> {
	let start = Instant::now();
	println!("Waiting for message from {}...", channel);
	while start.elapsed().as_secs() < nseconds {
		let unread = read_from_channel(channel, SLATE_SIGNED);
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

impl WalletCommAdapter for KeybaseWalletCommAdapter {
	fn supports_sync(&self) -> bool {
		true
	}

	// Send a slate to a keybase username then wait for a response for TTL seconds.
	fn send_tx_sync(&self, addr: &str, slate: &Slate) -> Result<Slate, Error> {
		// Send original slate to recipient with the SLATE_NEW topic
		match send(slate, addr, SLATE_NEW, TTL) {
			true => (),
			false => return Err(ErrorKind::ClientCallback("Posting transaction slate"))?,
		}
		println!("Sent new slate to {}", addr);
		// Wait for response from recipient with SLATE_SIGNED topic
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
		_params: HashMap<String, String>,
		config: WalletConfig,
		passphrase: &str,
		account: &str,
		node_api_secret: Option<String>,
	) -> Result<(), Error> {
		let node_client = HTTPNodeClient::new(&config.check_node_api_http_addr, node_api_secret);
		let wallet = instantiate_wallet(config.clone(), node_client, passphrase, account)
			.context(ErrorKind::WalletSeedDecryption)?;

		println!("Listening for messages via keybase chat...");
		loop {
			// listen for messages from all channels with topic SLATE_NEW
			let unread = get_unread(SLATE_NEW);
			for (msg, channel) in &unread {
				let blob = from_str::<Slate>(msg);
				match blob {
					Ok(mut slate) => {
						println!("Received message from channel {}", channel);
						match controller::foreign_single_use(wallet.clone(), |api| {
							api.receive_tx(&mut slate, None, None)?;
							Ok(())
						}) {
							// Reply to the same channel with topic SLATE_SIGNED
							Ok(_) => match send(slate, channel, SLATE_SIGNED, TTL) {
								true => {
									println!("Returned slate to {}", channel);
								}
								false => {
									println!("Failed to return slate to {}", channel);
								}
							},
							Err(e) => {
								println!("Error : {}", e);
							}
						}
					}
					Err(_) => (),
				}
			}
			sleep(SLEEP_DURATION);
		}
		Ok(())
	}
}
