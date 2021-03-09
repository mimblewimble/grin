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

//! This module allows to register callbacks on certain events. To add a custom
//! callback simply implement the coresponding trait and add it to the init function

extern crate hyper;
extern crate hyper_rustls;
extern crate tokio;

use crate::chain::BlockStatus;
use crate::common::types::{ServerConfig, WebHooksConfig};
use crate::core::core;
use crate::core::core::hash::Hashed;
use crate::p2p::types::PeerAddr;
use futures::TryFutureExt;
use grin_util::ToHex;
use hyper::client::HttpConnector;
use hyper::header::HeaderValue;
use hyper::Client;
use hyper::{Body, Method, Request};
use hyper_rustls::HttpsConnector;
use serde::Serialize;
use serde_json::{json, to_string};
use std::time::Duration;
use tokio::runtime::{Builder, Runtime};

/// Returns the list of event hooks that will be initialized for network events
pub fn init_net_hooks(config: &ServerConfig) -> Vec<Box<dyn NetEvents + Send + Sync>> {
	let mut list: Vec<Box<dyn NetEvents + Send + Sync>> = Vec::new();
	list.push(Box::new(EventLogger));
	if config.webhook_config.block_received_url.is_some()
		|| config.webhook_config.tx_received_url.is_some()
		|| config.webhook_config.header_received_url.is_some()
	{
		list.push(Box::new(WebHook::from_config(&config.webhook_config)));
	}
	list
}

/// Returns the list of event hooks that will be initialized for chain events
pub fn init_chain_hooks(config: &ServerConfig) -> Vec<Box<dyn ChainEvents + Send + Sync>> {
	let mut list: Vec<Box<dyn ChainEvents + Send + Sync>> = Vec::new();
	list.push(Box::new(EventLogger));
	if config.webhook_config.block_accepted_url.is_some() {
		list.push(Box::new(WebHook::from_config(&config.webhook_config)));
	}
	list
}

#[allow(unused_variables)]
/// Trait to be implemented by Network Event Hooks
pub trait NetEvents {
	/// Triggers when a new transaction arrives
	fn on_transaction_received(&self, tx: &core::Transaction) {}

	/// Triggers when a new block arrives
	fn on_block_received(&self, block: &core::Block, addr: &PeerAddr) {}

	/// Triggers when a new block header arrives
	fn on_header_received(&self, header: &core::BlockHeader, addr: &PeerAddr) {}
}

#[allow(unused_variables)]
/// Trait to be implemented by Chain Event Hooks
pub trait ChainEvents {
	/// Triggers when a new block is accepted by the chain (might be a Reorg or a Fork)
	fn on_block_accepted(&self, block: &core::Block, status: BlockStatus) {}
}

/// Basic Logger
struct EventLogger;

impl NetEvents for EventLogger {
	fn on_transaction_received(&self, tx: &core::Transaction) {
		debug!(
			"Received tx {}, [in/out/kern: {}/{}/{}] going to process.",
			tx.hash(),
			tx.inputs().len(),
			tx.outputs().len(),
			tx.kernels().len(),
		);
	}

	fn on_block_received(&self, block: &core::Block, addr: &PeerAddr) {
		debug!(
			"Received block {} at {} from {} [in/out/kern: {}/{}/{}] going to process.",
			block.hash(),
			block.header.height,
			addr,
			block.inputs().len(),
			block.outputs().len(),
			block.kernels().len(),
		);
	}

	fn on_header_received(&self, header: &core::BlockHeader, addr: &PeerAddr) {
		debug!(
			"Received block header {} at {} from {}, going to process.",
			header.hash(),
			header.height,
			addr
		);
	}
}

impl ChainEvents for EventLogger {
	fn on_block_accepted(&self, block: &core::Block, status: BlockStatus) {
		match status {
			BlockStatus::Reorg {
				prev,
				prev_head,
				fork_point,
			} => {
				warn!(
					"block_accepted (REORG!): {} at {}, (prev: {} at {}, prev_head: {} at {}, fork_point: {} at {}, depth: {})",
					block.hash(),
					block.header.height,
					prev.hash(),
					prev.height,
					prev_head.hash(),
					prev_head.height,
					fork_point.hash(),
					fork_point.height,
					prev_head.height.saturating_sub(fork_point.height),
				);
			}
			BlockStatus::Fork {
				prev,
				head,
				fork_point,
			} => {
				debug!(
					"block_accepted (fork?): {} at {}, (prev: {} at {}, head: {} at {}, fork_point: {} at {}, depth: {})",
					block.hash(),
					block.header.height,
					prev.hash(),
					prev.height,
					head.hash(),
					head.height,
					fork_point.hash(),
					fork_point.height,
					head.height.saturating_sub(fork_point.height),
				);
			}
			BlockStatus::Next { prev } => {
				debug!(
					"block_accepted (head+): {} at {} (prev: {} at {})",
					block.hash(),
					block.header.height,
					prev.hash(),
					prev.height,
				);
			}
		}
	}
}

fn parse_url(value: &Option<String>) -> Option<hyper::Uri> {
	match value {
		Some(url) => {
			let uri: hyper::Uri = match url.parse() {
				Ok(value) => value,
				Err(_) => panic!("Invalid url : {}", url),
			};
			let scheme = uri.scheme().map(|s| s.as_str());
			if (scheme != Some("http")) && (scheme != Some("https")) {
				panic!(
					"Invalid url scheme {}, expected one of ['http', https']",
					url
				)
			};
			Some(uri)
		}
		None => None,
	}
}

/// A struct that holds the hyper/tokio runtime.
struct WebHook {
	/// url to POST transaction data when a new transaction arrives from a peer
	tx_received_url: Option<hyper::Uri>,
	/// url to POST header data when a new header arrives from a peer
	header_received_url: Option<hyper::Uri>,
	/// url to POST block data when a new block arrives from a peer
	block_received_url: Option<hyper::Uri>,
	/// url to POST block data when a new block is accepted by our node (might be a reorg or a fork)
	block_accepted_url: Option<hyper::Uri>,
	/// The hyper client to be used for all requests
	client: Client<HttpsConnector<HttpConnector>>,
	/// The tokio event loop
	runtime: Runtime,
}

impl WebHook {
	/// Instantiates a Webhook struct
	fn new(
		tx_received_url: Option<hyper::Uri>,
		header_received_url: Option<hyper::Uri>,
		block_received_url: Option<hyper::Uri>,
		block_accepted_url: Option<hyper::Uri>,
		nthreads: u16,
		timeout: u16,
	) -> WebHook {
		let keep_alive = Duration::from_secs(timeout as u64);

		info!(
			"Spawning {} threads for webhooks (timeout set to {} secs)",
			nthreads, timeout
		);

		let https = HttpsConnector::new();
		let client = Client::builder()
			.pool_idle_timeout(keep_alive)
			.build::<_, hyper::Body>(https);

		WebHook {
			tx_received_url,
			block_received_url,
			header_received_url,
			block_accepted_url,
			client,
			runtime: Builder::new()
				.threaded_scheduler()
				.enable_all()
				.core_threads(nthreads as usize)
				.build()
				.unwrap(),
		}
	}

	/// Instantiates a Webhook struct from a configuration file
	fn from_config(config: &WebHooksConfig) -> WebHook {
		WebHook::new(
			parse_url(&config.tx_received_url),
			parse_url(&config.header_received_url),
			parse_url(&config.block_received_url),
			parse_url(&config.block_accepted_url),
			config.nthreads,
			config.timeout,
		)
	}

	fn post(&self, url: hyper::Uri, data: String) {
		let mut req = Request::new(Body::from(data));
		*req.method_mut() = Method::POST;
		*req.uri_mut() = url.clone();
		req.headers_mut().insert(
			hyper::header::CONTENT_TYPE,
			HeaderValue::from_static("application/json"),
		);

		let future = self.client.request(req).map_err(move |_res| {
			warn!("Error sending POST request to {}", url);
		});

		self.runtime.spawn(future);
	}
	fn make_request<T: Serialize>(&self, payload: &T, uri: &Option<hyper::Uri>) -> bool {
		if let Some(url) = uri {
			let payload = match to_string(payload) {
				Ok(serialized) => serialized,
				Err(_) => {
					return false; // print error message
				}
			};
			self.post(url.clone(), payload);
		}
		true
	}
}

impl ChainEvents for WebHook {
	fn on_block_accepted(&self, block: &core::Block, status: BlockStatus) {
		let status_str = match status {
			BlockStatus::Reorg { .. } => "reorg",
			BlockStatus::Fork { .. } => "fork",
			BlockStatus::Next { .. } => "head",
		};

		// Add additional `depth` field to the JSON in case of reorg
		let payload = if let BlockStatus::Reorg {
			fork_point,
			prev_head,
			..
		} = status
		{
			let depth = prev_head.height.saturating_sub(fork_point.height);
			json!({
				"hash": block.header.hash().to_hex(),
				"status": status_str,
				"data": block,
				"depth": depth
			})
		} else {
			json!({
				"hash": block.header.hash().to_hex(),
				"status": status_str,
				"data": block
			})
		};

		if !self.make_request(&payload, &self.block_accepted_url) {
			error!(
				"Failed to serialize block {} at height {}",
				block.hash(),
				block.header.height
			);
		}
	}
}

impl NetEvents for WebHook {
	/// Triggers when a new transaction arrives
	fn on_transaction_received(&self, tx: &core::Transaction) {
		let payload = json!({
			"hash": tx.hash().to_hex(),
			"data": tx
		});
		if !self.make_request(&payload, &self.tx_received_url) {
			error!("Failed to serialize transaction {}", tx.hash());
		}
	}

	/// Triggers when a new block arrives
	fn on_block_received(&self, block: &core::Block, addr: &PeerAddr) {
		let payload = json!({
			"hash": block.header.hash().to_hex(),
			"peer": addr,
			"data": block
		});
		if !self.make_request(&payload, &self.block_received_url) {
			error!(
				"Failed to serialize block {} at height {}",
				block.hash().to_hex(),
				block.header.height
			);
		}
	}

	/// Triggers when a new block header arrives
	fn on_header_received(&self, header: &core::BlockHeader, addr: &PeerAddr) {
		let payload = json!({
			"hash": header.hash().to_hex(),
			"peer": addr,
			"data": header
		});
		if !self.make_request(&payload, &self.header_received_url) {
			error!(
				"Failed to serialize header {} at height {}",
				header.hash(),
				header.height
			);
		}
	}
}
