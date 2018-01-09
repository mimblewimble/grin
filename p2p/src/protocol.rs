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

use std::io::{self, Read};
use std::env;
use std::fs::File;
use std::sync::Arc;
use std::net::SocketAddr;

use futures::Future;
use futures::sync::mpsc::UnboundedSender;
use futures_cpupool::CpuPool;
use tokio_core::net::TcpStream;

use core::core;
use core::core::hash::Hash;
use core::core::target::Difficulty;
use core::ser;
use conn::{Handler, TimeoutConnection};
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

		let handler = ProtocolHandler{adapter, addr};
		let (conn, listener) = TimeoutConnection::listen(conn, pool, handler);
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

	fn send_sumtrees_request(&self, height: u64, hash: Hash) -> Result<(), Error> {
		self.send_request(
			Type::SumtreesRequest,
			Type::SumtreesArchive,
			&SumtreesRequest { hash, height },
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

struct ProtocolHandler {
	adapter: Arc<NetAdapter>,
	addr: SocketAddr,
}

impl Handler for ProtocolHandler {
	fn handle(
		&self,
		sender: UnboundedSender<Vec<u8>>,
		header: MsgHeader,
		buf: Vec<u8>,
		reader: &mut Read,
	) -> Result<Option<Hash>, ser::Error> {

		match header.msg_type {
			Type::Ping => {
				let ping = ser::deserialize::<Ping>(&mut &buf[..])?;
				self.adapter.peer_difficulty(self.addr, ping.total_difficulty, ping.height);
				let pong = Pong {
					total_difficulty: self.adapter.total_difficulty(),
					height: self.adapter.total_height()
				};
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
				self.adapter.peer_difficulty(self.addr, pong.total_difficulty, pong.height);
				Ok(None)
			},
			Type::Transaction => {
				let tx = ser::deserialize::<core::Transaction>(&mut &buf[..])?;
				self.adapter.transaction_received(tx);
				Ok(None)
			}
			Type::GetBlock => {
				let h = ser::deserialize::<Hash>(&mut &buf[..])?;
				debug!(LOGGER, "handle_payload: GetBlock {}", h);

				let bo = self.adapter.get_block(h);
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

				debug!(LOGGER, "handle_payload: Block {}", bh);

				self.adapter.block_received(b, self.addr);
				Ok(Some(bh))
			}
			Type::GetHeaders => {
				// load headers from the locator
				let loc = ser::deserialize::<Locator>(&mut &buf[..])?;
				let headers = self.adapter.locate_headers(loc.hashes);

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
			Type::Headers => {
				let headers = ser::deserialize::<Headers>(&mut &buf[..])?;
				self.adapter.headers_received(headers.headers, self.addr);
				Ok(None)
			}

			Type::GetPeerAddrs => {
				let get_peers = ser::deserialize::<GetPeerAddrs>(&mut &buf[..])?;
				let peer_addrs = self.adapter.find_peer_addrs(get_peers.capabilities);

				// serialize and send all the headers over
				let mut body_data = vec![];
				ser::serialize(&mut body_data,
					&PeerAddrs {
						peers: peer_addrs.iter().map(|sa| SockAddr(*sa)).collect(),
					})?;
				let mut data = vec![];
				ser::serialize(&mut data,
					&MsgHeader::new(Type::PeerAddrs, body_data.len() as u64))?;
				data.append(&mut body_data);
				if let Err(e) = sender.unbounded_send(data) {
					debug!(LOGGER, "handle_payload: GetPeerAddrs, error sending: {:?}", e);
				}

				Ok(None)
			}

			Type::PeerAddrs => {
				let peer_addrs = ser::deserialize::<PeerAddrs>(&mut &buf[..])?;
				self.adapter.peer_addrs_received(peer_addrs.peers.iter().map(|pa| pa.0).collect());
				Ok(None)
			}

			Type::SumtreesRequest => {
				let sm_req = ser::deserialize::<SumtreesRequest>(&mut &buf[..])?;
				debug!(LOGGER, "handle_payload: sumtree req for {} at {}",
							sm_req.hash, sm_req.height);

				let sumtrees = self.adapter.sumtrees_read(sm_req.hash);

				if let Some(mut sumtrees) = sumtrees {
					// first send the sumtree archive information
					let mut data = vec![];
					ser::serialize(&mut data,
						&SumtreesArchive {
							height: sm_req.height as u64,
							hash: sm_req.hash,
							rewind_to_output: sumtrees.output_index,
							rewind_to_kernel: sumtrees.kernel_index,
						})?;
					if let Err(e) = sender.unbounded_send(data) {
						debug!(LOGGER, "handle_payload: error sending sumtrees info: {:?}", e);
					}

					// second, send the archive byte stream
					loop {
						let mut buf = Vec::with_capacity(8000);
						let len = sumtrees.reader.read(&mut buf)?;
						if let Err(e) = sender.unbounded_send(buf) {
							debug!(LOGGER, "handle_payload: error sending sumtrees: {:?}", e);
						}
						if len < 8000 {
							break;
						}
					}
				}
				Ok(None)
			}

			Type::SumtreesArchive => {
				let sm_arch = ser::deserialize::<SumtreesArchive>(&mut &buf[..])?;
				debug!(LOGGER, "handle_payload: sumtree archive for {} at {} rewind to {}/{}",
							sm_arch.hash, sm_arch.height,
							sm_arch.rewind_to_output, sm_arch.rewind_to_kernel);

				let mut tmp = env::temp_dir();
				tmp.push("sumtree.zip");
				{
					let mut tmp_zip = File::create(tmp.clone())?;
					io::copy(reader, &mut tmp_zip)?;
				}

				let tmp_zip = File::open(tmp)?;
				self.adapter.sumtrees_write(
					sm_arch.hash, sm_arch.rewind_to_output, sm_arch.rewind_to_kernel, tmp_zip);
				Ok(None)
			}

			_ => {
				debug!(LOGGER, "unknown message type {:?}", header.msg_type);
				Ok(None)
			}
		}
	}
}
