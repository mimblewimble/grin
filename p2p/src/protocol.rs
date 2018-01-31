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

use futures::Future;
use futures::sync::mpsc::UnboundedSender;
use futures_cpupool::CpuPool;
use tokio_core::net::TcpStream;

use core::core;
use core::core::hash::{Hash, Hashed};
use core::core::target::Difficulty;
use core::ser;
use conn::TimeoutConnection;
use msg::*;
use types::*;
use util::LOGGER;
use util::OneTime;

#[allow(dead_code)]
pub struct ProtocolV1 {
	conn: OneTime<TimeoutConnection>,
}

impl ProtocolV1 {
	pub fn new() -> ProtocolV1 {
		ProtocolV1 {
			conn: OneTime::new(),
		}
	}
}

impl Protocol for ProtocolV1 {
	/// Sets up the protocol reading, writing and closing logic.
	fn handle(
		&self,
		conn: TcpStream,
		adapter: Arc<NetAdapter>,
		addr: SocketAddr,
		pool: CpuPool,
	) -> Box<Future<Item = (), Error = Error>> {
		let (conn, listener) = TimeoutConnection::listen(conn, pool, move |sender, header, data| {
			let adapt = adapter.as_ref();
			handle_payload(adapt, sender, header, data, addr)
		});

		self.conn.init(conn);

		listener
	}

	/// Bytes sent and received.
	fn transmitted_bytes(&self) -> (u64, u64) {
		self.conn.borrow().transmitted_bytes()
	}

	/// Sends a ping message to the remote peer. Will panic if handle has never
	/// been called on this protocol.
	fn send_ping(&self, total_difficulty: Difficulty, height: u64) -> Result<(), Error> {
		self.send_request(
			Type::Ping,
			Type::Pong,
			&Ping { total_difficulty, height },
			None,
		)
	}

	/// Serializes and sends a block to our remote peer
	fn send_block(&self, b: &core::Block) -> Result<(), Error> {
		self.send_msg(Type::Block, b)
	}

	fn send_compact_block(&self, cb: &core::CompactBlock) -> Result<(), Error> {
		self.send_msg(Type::CompactBlock, cb)
	}

	/// Serializes and sends a block header to our remote peer ("header first" propagation)
	fn send_header(&self, bh: &core::BlockHeader) -> Result<(), Error> {
		self.send_msg(Type::Header, bh)
	}

	/// Serializes and sends a transaction to our remote peer
	fn send_transaction(&self, tx: &core::Transaction) -> Result<(), Error> {
		self.send_msg(Type::Transaction, tx)
	}

	fn send_header_request(&self, locator: Vec<Hash>) -> Result<(), Error> {
		self.send_request(
			Type::GetHeaders,
			Type::Headers,
			&Locator { hashes: locator },
			None,
		)
	}

	fn send_block_request(&self, h: Hash) -> Result<(), Error> {
		self.send_request(Type::GetBlock, Type::Block, &h, Some(h))
	}

	fn send_compact_block_request(&self, h: Hash) -> Result<(), Error> {
		self.send_request(Type::GetCompactBlock, Type::CompactBlock, &h, Some(h))
	}

	fn send_peer_request(&self, capab: Capabilities) -> Result<(), Error> {
		self.send_request(
			Type::GetPeerAddrs,
			Type::PeerAddrs,
			&GetPeerAddrs {
				capabilities: capab,
			},
			None,
		)
	}

	/// Close the connection to the remote peer
	fn close(&self) {
		// TODO some kind of shutdown signal
	}
}

impl ProtocolV1 {
	fn send_msg<W: ser::Writeable>(&self, t: Type, body: &W) -> Result<(), Error> {
		self.conn.borrow().send_msg(t, body)
	}

	fn send_request<W: ser::Writeable>(
		&self,
		t: Type,
		rt: Type,
		body: &W,
		expect_resp: Option<Hash>,
	) -> Result<(), Error> {
		if self.conn.is_initialized() {
			self.conn.borrow().send_request(t, rt, body, expect_resp)
		} else {
			Ok(())
		}
	}
}

fn handle_payload(
	adapter: &NetAdapter,
	sender: UnboundedSender<Vec<u8>>,
	header: MsgHeader,
	buf: Vec<u8>,
	addr: SocketAddr,
) -> Result<Option<Hash>, ser::Error> {
	match header.msg_type {
		Type::Ping => {
			let ping = ser::deserialize::<Ping>(&mut &buf[..])?;
			adapter.peer_difficulty(addr, ping.total_difficulty, ping.height);
			let pong = Pong { total_difficulty: adapter.total_difficulty(), height: adapter.total_height() };
			let mut body_data = vec![];
			try!(ser::serialize(&mut body_data, &pong));
			let mut data = vec![];
			try!(ser::serialize(
				&mut data,
				&MsgHeader::new(Type::Pong, body_data.len() as u64),
			));
			data.append(&mut body_data);

			if let Err(e) = sender.unbounded_send(data) {
				debug!(LOGGER, "handle_payload: Ping, error sending: {:?}", e);
			}

			Ok(None)
		}
		Type::Pong => {
			let pong = ser::deserialize::<Pong>(&mut &buf[..])?;
			adapter.peer_difficulty(addr, pong.total_difficulty, pong.height);
			Ok(None)
		},
		Type::Transaction => {
			let tx = ser::deserialize::<core::Transaction>(&mut &buf[..])?;
			debug!(LOGGER, "handle_payload: Transaction: {}", tx.hash());

			adapter.transaction_received(tx);
			Ok(None)
		}
		Type::GetBlock => {
			let h = ser::deserialize::<Hash>(&mut &buf[..])?;
			debug!(LOGGER, "handle_payload: GetBlock: {}", h);

			let bo = adapter.get_block(h);
			if let Some(b) = bo {
				// serialize and send the block over
				let mut body_data = vec![];
				try!(ser::serialize(&mut body_data, &b));
				let mut data = vec![];
				try!(ser::serialize(
					&mut data,
					&MsgHeader::new(Type::Block, body_data.len() as u64),
				));
				data.append(&mut body_data);
				if let Err(e) = sender.unbounded_send(data) {
					debug!(LOGGER, "handle_payload: GetBlock, error sending: {:?}", e);
				}
			}
			Ok(None)
		}
		Type::Block => {
			let b = ser::deserialize::<core::Block>(&mut &buf[..])?;
			let bh = b.hash();
			debug!(LOGGER, "handle_payload: Block: {}", bh);

			adapter.block_received(b, addr);
			Ok(Some(bh))
		}
		Type::GetCompactBlock => {
			let h = ser::deserialize::<Hash>(&mut &buf[..])?;
			debug!(LOGGER, "handle_payload: GetCompactBlock: {}", h);

			if let Some(b) = adapter.get_block(h) {
				let cb = b.as_compact_block();

				// serialize and send the block over in compact representation
				let mut body_data = vec![];
				let mut data = vec![];

				// if we have txs in the block send a compact block
				// but if block is empty then send the full block
				if cb.kern_ids.is_empty() {
					debug!(
						LOGGER,
						"handle_payload: GetCompactBlock: empty block, sending full block",
					);

					try!(ser::serialize(&mut body_data, &b));
					try!(ser::serialize(
						&mut data,
						&MsgHeader::new(Type::Block, body_data.len() as u64),
					));
				} else {
					try!(ser::serialize(&mut body_data, &cb));
					try!(ser::serialize(
						&mut data,
						&MsgHeader::new(Type::CompactBlock, body_data.len() as u64),
					));
				}

				data.append(&mut body_data);
				if let Err(e) = sender.unbounded_send(data) {
					debug!(LOGGER, "handle_payload: GetCompactBlock, error sending: {:?}", e);
				}
			}
			Ok(None)
		}
		Type::CompactBlock => {
			let b = ser::deserialize::<core::CompactBlock>(&mut &buf[..])?;
			let bh = b.hash();
			debug!(LOGGER, "handle_payload: CompactBlock: {}", bh);

			adapter.compact_block_received(b, addr);
			Ok(Some(bh))
		}
		// A peer is asking us for some headers via a locator
		Type::GetHeaders => {
			let loc = ser::deserialize::<Locator>(&mut &buf[..])?;
			debug!(LOGGER, "handle_payload: GetHeaders: {:?}", loc);

			let headers = adapter.locate_headers(loc.hashes);

			// serialize and send all the headers over
			let mut body_data = vec![];
			try!(ser::serialize(
				&mut body_data,
				&Headers { headers: headers },
			));
			let mut data = vec![];
			try!(ser::serialize(
				&mut data,
				&MsgHeader::new(Type::Headers, body_data.len() as u64),
			));
			data.append(&mut body_data);
			if let Err(e) = sender.unbounded_send(data) {
				debug!(LOGGER, "handle_payload: GetHeaders, error sending: {:?}", e);
			}

			Ok(None)
		}
		// "header first" block propagation - if we have not yet seen this block
		// we can go request it from some of our peers
		Type::Header => {
			let header = ser::deserialize::<core::BlockHeader>(&mut &buf[..])?;
			debug!(LOGGER, "handle_payload: Header: {}", header.hash());

			adapter.header_received(header, addr);

			// we do not return a hash here as we never request a single header
			// a header will always arrive unsolicited
			Ok(None)
		}
		// receive headers as part of the sync process
		Type::Headers => {
			let headers = ser::deserialize::<Headers>(&mut &buf[..])?;
			debug!(LOGGER, "handle_payload: Headers: {}", headers.headers.len());

			adapter.headers_received(headers.headers, addr);
			Ok(None)
		}
		Type::GetPeerAddrs => {
			let get_peers = ser::deserialize::<GetPeerAddrs>(&mut &buf[..])?;
			let peer_addrs = adapter.find_peer_addrs(get_peers.capabilities);

			// serialize and send all the headers over
			let mut body_data = vec![];
			try!(ser::serialize(
				&mut body_data,
				&PeerAddrs {
					peers: peer_addrs.iter().map(|sa| SockAddr(*sa)).collect(),
				},
			));
			let mut data = vec![];
			try!(ser::serialize(
				&mut data,
				&MsgHeader::new(Type::PeerAddrs, body_data.len() as u64),
			));
			data.append(&mut body_data);
			if let Err(e) = sender.unbounded_send(data) {
				debug!(LOGGER, "handle_payload: GetPeerAddrs, error sending: {:?}", e);
			}

			Ok(None)
		}
		Type::PeerAddrs => {
			let peer_addrs = ser::deserialize::<PeerAddrs>(&mut &buf[..])?;
			adapter.peer_addrs_received(peer_addrs.peers.iter().map(|pa| pa.0).collect());
			Ok(None)
		}
		_ => {
			debug!(LOGGER, "unknown message type {:?}", header.msg_type);
			Ok(None)
		}
	}
}
