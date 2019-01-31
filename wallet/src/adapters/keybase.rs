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

use crate::controller;
use crate::core::libtx::slate::Slate;
use crate::libwallet::{Error, ErrorKind};
use crate::{instantiate_wallet, HTTPNodeClient, WalletCommAdapter, WalletConfig};
use failure::ResultExt;
use serde::Serialize;
use serde_json::{from_str, json, to_string, Value};
use std::collections::{HashMap, HashSet};
use std::process::{Command, Stdio};
use std::str::from_utf8;
use std::thread::sleep;
use std::time::{Duration, Instant};

const TTL: u16 = 60; // TODO: Pass this as a parameter
const LISTEN_SLEEP_DURATION: Duration = Duration::from_millis(5000);
const POLL_SLEEP_DURATION: Duration = Duration::from_millis(1000);

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
fn api_send(payload: &str) -> Result<Value, Error> {
	let mut proc = Command::new("keybase");
	proc.args(&["chat", "api", "-m", &payload]);
	let output = proc.output().expect("No output");
	if !output.status.success() {
		error!(
			"keybase api fail: {} {}",
			String::from_utf8_lossy(&output.stdout),
			String::from_utf8_lossy(&output.stderr)
		);
		Err(ErrorKind::GenericError("keybase api fail".to_owned()))?
	} else {
		let response: Value =
			from_str(from_utf8(&output.stdout).expect("Bad output")).expect("Bad output");
		let err_msg = format!("{}", response["error"]["message"]);
		if err_msg.len() > 0 && err_msg != "null" {
			error!("api_send got error: {}", err_msg);
		}

		Ok(response)
	}
}

/// Get keybase username
fn whoami() -> Result<String, Error> {
	let mut proc = Command::new("keybase");
	proc.args(&["status", "-json"]);
	let output = proc.output().expect("No output");
	if !output.status.success() {
		error!(
			"keybase api fail: {} {}",
			String::from_utf8_lossy(&output.stdout),
			String::from_utf8_lossy(&output.stderr)
		);
		Err(ErrorKind::GenericError("keybase api fail".to_owned()))?
	} else {
		let response: Value =
			from_str(from_utf8(&output.stdout).expect("Bad output")).expect("Bad output");
		let err_msg = format!("{}", response["error"]["message"]);
		if err_msg.len() > 0 && err_msg != "null" {
			error!("status query got error: {}", err_msg);
		}

		let username = response["Username"].as_str();
		if let Some(s) = username {
			Ok(s.to_string())
		} else {
			error!("keybase username query fail");
			Err(ErrorKind::GenericError(
				"keybase username query fail".to_owned(),
			))?
		}
	}
}

/// Get all unread messages from a specific channel/topic and mark as read.
fn read_from_channel(channel: &str, topic: &str) -> Result<Vec<String>, Error> {
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
	))
	.unwrap();

	let response = api_send(&payload);
	if let Ok(res) = response {
		let mut unread: Vec<String> = Vec::new();
		for msg in res["result"]["messages"]
			.as_array()
			.unwrap_or(&vec![json!({})])
			.iter()
		{
			if (msg["msg"]["content"]["type"] == "text") && (msg["msg"]["unread"] == true) {
				let message = msg["msg"]["content"]["text"]["body"].as_str().unwrap_or("");
				unread.push(message.to_owned());
			}
		}
		Ok(unread)
	} else {
		Err(ErrorKind::GenericError("keybase api fail".to_owned()))?
	}
}

/// Get unread messages from all channels and mark as read.
fn get_unread(topic: &str) -> Result<HashMap<String, String>, Error> {
	let payload = to_string(&json!({
		"method": "list",
		"params": {
			"options": {
				"topic_type": "dev",
			},
		}
	}))
	.unwrap();
	let response = api_send(&payload);

	if let Ok(res) = response {
		let mut channels = HashSet::new();
		// Unfortunately the response does not contain the message body
		// and a separate call is needed for each channel
		for msg in res["result"]["conversations"]
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
			if messages.is_err() {
				break;
			}
			for msg in messages.unwrap() {
				unread.insert(msg, channel.to_string());
			}
		}
		Ok(unread)
	} else {
		Err(ErrorKind::GenericError("keybase api fail".to_owned()))?
	}
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
	}))
	.unwrap();
	let response = api_send(&payload);
	if let Ok(res) = response {
		match res["result"]["message"].as_str() {
			Some("message sent") => true,
			_ => false,
		}
	} else {
		false
	}
}

/// Send a notify to self that self-destructs after ttl minutes.
fn notify(message: &str, channel: &str, ttl: u16) -> bool {
	let minutes = format!("{}m", ttl);
	let payload = to_string(&json!({
		"method": "send",
		"params": {
			"options": {
				"channel": {
					"name": channel
				},
				"message": {
					"body": message
				},
				"exploding_lifetime": minutes
			}
		}
	}))
	.unwrap();
	let response = api_send(&payload);
	if let Ok(res) = response {
		match res["result"]["message"].as_str() {
			Some("message sent") => true,
			_ => false,
		}
	} else {
		false
	}
}

/// Listen for a message from a specific channel with topic SLATE_SIGNED for nseconds and return the first valid slate.
fn poll(nseconds: u64, channel: &str) -> Option<Slate> {
	let start = Instant::now();
	info!("Waiting for response message from @{}...", channel);
	while start.elapsed().as_secs() < nseconds {
		let unread = read_from_channel(channel, SLATE_SIGNED);
		for msg in unread.unwrap().iter() {
			let blob = from_str::<Slate>(msg);
			match blob {
				Ok(slate) => {
					info!(
						"keybase response message received from @{}, tx uuid: {}",
						channel, slate.id,
					);
					return Some(slate);
				}
				Err(_) => (),
			}
		}
		sleep(POLL_SLEEP_DURATION);
	}
	error!(
		"No response from @{} in {} seconds. Grin send failed!",
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
		// Limit only one recipient
		if addr.matches(",").count() > 0 {
			error!("Only one recipient is supported!");
			return Err(ErrorKind::GenericError("Tx rejected".to_owned()))?;
		}

		// Send original slate to recipient with the SLATE_NEW topic
		match send(slate, addr, SLATE_NEW, TTL) {
			true => (),
			false => {
				return Err(ErrorKind::ClientCallback(
					"Posting transaction slate".to_owned(),
				))?
			}
		}
		info!(
			"tx request has been sent to @{}, tx uuid: {}",
			addr, slate.id
		);
		// Wait for response from recipient with SLATE_SIGNED topic
		match poll(TTL as u64, addr) {
			Some(slate) => return Ok(slate),
			None => {
				return Err(ErrorKind::ClientCallback(
					"Receiving reply from recipient".to_owned(),
				))?
			}
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

		info!("Listening for transactions on keybase ...");
		loop {
			// listen for messages from all channels with topic SLATE_NEW
			let unread = get_unread(SLATE_NEW);
			if unread.is_err() {
				error!("Listening exited for some keybase api failure");
				break;
			}
			for (msg, channel) in &unread.unwrap() {
				let blob = from_str::<Slate>(msg);
				match blob {
					Ok(mut slate) => {
						let tx_uuid = slate.id;

						// Reject multiple recipients channel for safety
						{
							if channel.matches(",").count() > 1 {
								error!(
									"Incoming tx initiated on channel \"{}\" is rejected, multiple recipients channel! amount: {}(g), tx uuid: {}",
									channel,
									slate.amount as f64 / 1000000000.0,
									tx_uuid,
								);
								continue;
							}
						}

						info!(
							"tx initiated on channel \"{}\", to send you {}(g). tx uuid: {}",
							channel,
							slate.amount as f64 / 1000000000.0,
							tx_uuid,
						);
						match controller::foreign_single_use(wallet.clone(), |api| {
							if let Err(e) = api.verify_slate_messages(&slate) {
								error!("Error validating participant messages: {}", e);
								return Err(e);
							}
							api.receive_tx(&mut slate, None, None)?;
							Ok(())
						}) {
							// Reply to the same channel with topic SLATE_SIGNED
							Ok(_) => match send(slate, channel, SLATE_SIGNED, TTL) {
								true => {
									notify_on_receive(
										config.keybase_notify_ttl.unwrap_or(1440),
										channel.to_string(),
										tx_uuid.to_string(),
									);
									debug!("Returned slate to @{} via keybase", channel);
								}
								false => {
									error!("Failed to return slate to @{} via keybase. Incoming tx failed", channel);
								}
							},
							Err(e) => {
								error!(
									"Error on receiving tx via keybase: {}. Incoming tx failed",
									e
								);
							}
						}
					}
					Err(_) => (),
				}
			}
			sleep(LISTEN_SLEEP_DURATION);
		}
		Ok(())
	}
}

/// Notify in keybase on receiving a transaction
fn notify_on_receive(keybase_notify_ttl: u16, channel: String, tx_uuid: String) {
	if keybase_notify_ttl > 0 {
		let my_username = whoami();
		if let Ok(username) = my_username {
			let split = channel.split(",");
			let vec: Vec<&str> = split.collect();
			if vec.len() > 1 {
				let receiver = username;
				let sender = if vec[0] == receiver {
					vec[1]
				} else {
					if vec[1] != receiver {
						error!("keybase - channel doesn't include my username! channel: {}, username: {}",
							   channel, receiver
						);
					}
					vec[0]
				};

				let msg = format!(
					"[grin wallet notice]: \
					 you could have some coins received from @{}\n\
					 Transaction Id: {}",
					sender, tx_uuid
				);
				notify(&msg, &receiver, keybase_notify_ttl);
				info!(
					"tx from @{} is done, please check on grin wallet. tx uuid: {}",
					sender, tx_uuid,
				);
			}
		} else {
			error!("keybase notification fail on whoami query");
		}
	}
}
