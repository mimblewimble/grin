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

use std::sync::Arc;
use std::net::SocketAddr;

use core::core;
use core::core::hash::{Hash, Hashed};
use core::ser;
use conn::*;
use msg::*;
use rand;
use rand::Rng;
use types::*;
use util::LOGGER;

pub struct Protocol {
	adapter: Arc<NetAdapter>,
	addr: SocketAddr,
}

impl Protocol {
	pub fn new(adapter: Arc<NetAdapter>, addr: SocketAddr) -> Protocol {
		Protocol{adapter, addr}
	}
}

impl MessageHandler for Protocol {
	fn consume(&self, msg: &mut Message) -> Result<Option<(Vec<u8>, Type)>, Error> {
		let adapter = &self.adapter;

		match msg.header.msg_type {

			Type::Ping => {
				let ping: Ping = msg.body()?;
				adapter.peer_difficulty(self.addr, ping.total_difficulty, ping.height);

				let pong_bytes = ser::ser_vec(
					&Pong {
						total_difficulty: adapter.total_difficulty(),
						height: adapter.total_height(),
					}).unwrap();

				Ok(Some((pong_bytes, Type::Pong)))
			}

			Type::Pong => {
				let pong: Pong = msg.body()?;
				adapter.peer_difficulty(self.addr, pong.total_difficulty, pong.height);
				Ok(None)
			},

			Type::Transaction => {
				let tx: core::Transaction = msg.body()?;
				adapter.transaction_received(tx);
				Ok(None)
			}

			Type::GetBlock => {
				let h: Hash = msg.body()?;
				debug!(LOGGER, "handle_payload: GetBlock {}", h);

				let bo = adapter.get_block(h);
				if let Some(b) = bo {
					let block_bytes = ser::ser_vec(&b).unwrap();
					return Ok(Some((block_bytes, Type::Block)));
				}
				Ok(None)
			}

			Type::Block => {
				let b: core::Block = msg.body()?;
				let bh = b.hash();

				debug!(LOGGER, "handle_payload: Block {}", bh);

				adapter.block_received(b, self.addr);
				Ok(None)
			}


			Type::GetCompactBlock => {
				let h: Hash = msg.body()?;
				debug!(LOGGER, "handle_payload: GetCompactBlock: {}", h);

				if let Some(b) = adapter.get_block(h) {
					let cb = b.as_compact_block();

					// serialize and send the block over in compact representation

					// if we have txs in the block send a compact block
					// but if block is empty -
					// to allow us to test all code paths, randomly choose to send
					// either the block or the compact block
					let mut rng = rand::thread_rng();

					if cb.kern_ids.is_empty() && rng.gen() {
						debug!(
							LOGGER,
							"handle_payload: GetCompactBlock: empty block, sending full block",
							);

						let block_bytes = ser::ser_vec(&b).unwrap();
						Ok(Some((block_bytes, Type::Block)))
					} else {
						let compact_block_bytes = ser::ser_vec(&cb).unwrap();
						Ok(Some((compact_block_bytes, Type::CompactBlock)))
					}
				} else {
					Ok(None)
				}
			}

			Type::CompactBlock => {
				let b: core::CompactBlock = msg.body()?;
				let bh = b.hash();
				debug!(LOGGER, "handle_payload: CompactBlock: {}", bh);

				adapter.compact_block_received(b, self.addr);
				Ok(None)
			}

			Type::GetHeaders => {
				// load headers from the locator
				let loc: Locator = msg.body()?;
				let headers = adapter.locate_headers(loc.hashes);

				// serialize and send all the headers over
				let header_bytes = ser::ser_vec(&Headers { headers: headers }).unwrap();
				return Ok(Some((header_bytes, Type::Headers)));
			}

			// "header first" block propagation - if we have not yet seen this block
			// we can go request it from some of our peers
			Type::Header => {
				let header: core::BlockHeader = msg.body()?;

				adapter.header_received(header, self.addr);

				// we do not return a hash here as we never request a single header
				// a header will always arrive unsolicited
				Ok(None)
			}

			Type::Headers => {
				let headers: Headers = msg.body()?;
				adapter.headers_received(headers.headers, self.addr);
				Ok(None)
			}

			Type::GetPeerAddrs => {
				let get_peers: GetPeerAddrs = msg.body()?;
				let peer_addrs = adapter.find_peer_addrs(get_peers.capabilities);
				let peer_addrs_bytes = ser::ser_vec(
					&PeerAddrs {
						peers: peer_addrs.iter().map(|sa| SockAddr(*sa)).collect(),
					}).unwrap();
				return Ok(Some((peer_addrs_bytes, Type::PeerAddrs)));
			}

			Type::PeerAddrs => {
				let peer_addrs: PeerAddrs = msg.body()?;
				adapter.peer_addrs_received(peer_addrs.peers.iter().map(|pa| pa.0).collect());
				Ok(None)
			}

			_ => {
				debug!(LOGGER, "unknown message type {:?}", msg.header.msg_type);
				Ok(None)
			}
		}
	}
}
