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

use std::env;
use std::fs::File;
use std::io::{self, BufWriter};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::time;

use conn::{Message, MessageHandler, Response};
use core::core::{self, hash::Hash, CompactBlock};
use core::{global, ser};

use msg::{
	read_exact, BanReason, GetPeerAddrs, Headers, Locator, PeerAddrs, Ping, Pong, SockAddr,
	TxHashSetArchive, TxHashSetRequest, Type,
};
use types::{Error, NetAdapter};
use util::LOGGER;

pub struct Protocol {
	adapter: Arc<NetAdapter>,
	addr: SocketAddr,
}

impl Protocol {
	pub fn new(adapter: Arc<NetAdapter>, addr: SocketAddr) -> Protocol {
		Protocol { adapter, addr }
	}
}

impl MessageHandler for Protocol {
	fn consume<'a>(&self, mut msg: Message<'a>) -> Result<Option<Response<'a>>, Error> {
		let adapter = &self.adapter;

		// If we received a msg from a banned peer then log and drop it.
		// If we are getting a lot of these then maybe we are not cleaning
		// banned peers up correctly?
		if adapter.is_banned(self.addr.clone()) {
			debug!(
				LOGGER,
				"handler: consume: peer {:?} banned, received: {:?}, dropping.",
				self.addr,
				msg.header.msg_type,
			);
			return Ok(None);
		}

		match msg.header.msg_type {
			Type::Ping => {
				let ping: Ping = msg.body()?;
				adapter.peer_difficulty(self.addr, ping.total_difficulty, ping.height);

				Ok(Some(msg.respond(
					Type::Pong,
					Pong {
						total_difficulty: adapter.total_difficulty(),
						height: adapter.total_height(),
					},
				)))
			}

			Type::Pong => {
				let pong: Pong = msg.body()?;
				adapter.peer_difficulty(self.addr, pong.total_difficulty, pong.height);
				Ok(None)
			}

			Type::BanReason => {
				let ban_reason: BanReason = msg.body()?;
				error!(LOGGER, "handle_payload: BanReason {:?}", ban_reason);
				Ok(None)
			}

			Type::Transaction => {
				debug!(
					LOGGER,
					"handle_payload: received tx: msg_len: {}", msg.header.msg_len
				);
				let tx: core::Transaction = msg.body()?;
				adapter.transaction_received(tx, false);
				Ok(None)
			}

			Type::StemTransaction => {
				debug!(
					LOGGER,
					"handle_payload: received stem tx: msg_len: {}", msg.header.msg_len
				);
				let tx: core::Transaction = msg.body()?;
				adapter.transaction_received(tx, true);
				Ok(None)
			}

			Type::GetBlock => {
				let h: Hash = msg.body()?;
				trace!(
					LOGGER,
					"handle_payload: Getblock: {}, msg_len: {}",
					h,
					msg.header.msg_len,
				);

				let bo = adapter.get_block(h);
				if let Some(b) = bo {
					return Ok(Some(msg.respond(Type::Block, b)));
				}
				Ok(None)
			}

			Type::Block => {
				debug!(
					LOGGER,
					"handle_payload: received block: msg_len: {}", msg.header.msg_len
				);
				let b: core::Block = msg.body()?;

				adapter.block_received(b, self.addr);
				Ok(None)
			}

			Type::GetCompactBlock => {
				let h: Hash = msg.body()?;
				if let Some(b) = adapter.get_block(h) {
					let cb: CompactBlock = b.into();
					Ok(Some(msg.respond(Type::CompactBlock, cb)))
				} else {
					Ok(None)
				}
			}

			Type::CompactBlock => {
				debug!(
					LOGGER,
					"handle_payload: received compact block: msg_len: {}", msg.header.msg_len
				);
				let b: core::CompactBlock = msg.body()?;

				adapter.compact_block_received(b, self.addr);
				Ok(None)
			}

			Type::GetHeaders => {
				// load headers from the locator
				let loc: Locator = msg.body()?;
				let headers = adapter.locate_headers(loc.hashes);

				// serialize and send all the headers over
				Ok(Some(
					msg.respond(Type::Headers, Headers { headers: headers }),
				))
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
				let conn = &mut msg.get_conn();

				let header_size: u64 = headers_header_size(conn, msg.header.msg_len)?;
				let mut total_read: u64 = 2;
				let mut reserved: Vec<u8> = vec![];

				while total_read < msg.header.msg_len || reserved.len() > 0 {
					let headers: Headers = headers_streaming_body(
						conn,
						msg.header.msg_len,
						8,
						&mut total_read,
						&mut reserved,
						header_size,
					)?;
					adapter.headers_received(headers.headers, self.addr);
				}
				Ok(None)
			}

			Type::GetPeerAddrs => {
				let get_peers: GetPeerAddrs = msg.body()?;
				let peer_addrs = adapter.find_peer_addrs(get_peers.capabilities);
				Ok(Some(msg.respond(
					Type::PeerAddrs,
					PeerAddrs {
						peers: peer_addrs.iter().map(|sa| SockAddr(*sa)).collect(),
					},
				)))
			}

			Type::PeerAddrs => {
				let peer_addrs: PeerAddrs = msg.body()?;
				adapter.peer_addrs_received(peer_addrs.peers.iter().map(|pa| pa.0).collect());
				Ok(None)
			}

			Type::TxHashSetRequest => {
				let sm_req: TxHashSetRequest = msg.body()?;
				debug!(
					LOGGER,
					"handle_payload: txhashset req for {} at {}", sm_req.hash, sm_req.height
				);

				let txhashset = self.adapter.txhashset_read(sm_req.hash);

				if let Some(txhashset) = txhashset {
					let file_sz = txhashset.reader.metadata()?.len();
					let mut resp = msg.respond(
						Type::TxHashSetArchive,
						&TxHashSetArchive {
							height: sm_req.height as u64,
							hash: sm_req.hash,
							bytes: file_sz,
						},
					);
					resp.add_attachment(txhashset.reader);
					Ok(Some(resp))
				} else {
					Ok(None)
				}
			}

			Type::TxHashSetArchive => {
				let sm_arch: TxHashSetArchive = msg.body()?;
				debug!(
					LOGGER,
					"handle_payload: txhashset archive for {} at {}. size={}",
					sm_arch.hash,
					sm_arch.height,
					sm_arch.bytes,
				);
				if !self.adapter.txhashset_receive_ready() {
					error!(
						LOGGER,
						"handle_payload: txhashset archive received but SyncStatus not on TxHashsetDownload",
					);
					return Err(Error::BadMessage);
				}
				let mut tmp = env::temp_dir();
				tmp.push("txhashset.zip");
				let mut save_txhashset_to_file = |file| -> Result<(), Error> {
					let mut tmp_zip = BufWriter::new(File::create(file)?);
					msg.copy_attachment(sm_arch.bytes as usize, &mut tmp_zip)?;
					tmp_zip.into_inner().unwrap().sync_all()?;
					Ok(())
				};

				if let Err(e) = save_txhashset_to_file(tmp.clone()) {
					error!(
						LOGGER,
						"handle_payload: txhashset archive save to file fail. err={:?}", e
					);
					return Err(e);
				}

				trace!(
					LOGGER,
					"handle_payload: txhashset archive save to file {:?} success",
					tmp,
				);

				let tmp_zip = File::open(tmp)?;
				let res = self
					.adapter
					.txhashset_write(sm_arch.hash, tmp_zip, self.addr);

				debug!(
					LOGGER,
					"handle_payload: txhashset archive for {} at {}, DONE. Data Ok: {}",
					sm_arch.hash,
					sm_arch.height,
					res
				);

				Ok(None)
			}

			_ => {
				debug!(LOGGER, "unknown message type {:?}", msg.header.msg_type);
				Ok(None)
			}
		}
	}
}

/// Read the Headers Vec size from the underlying connection, and calculate maximum header_size of one Header
fn headers_header_size(conn: &mut TcpStream, msg_len: u64) -> Result<u64, Error> {
	let mut size = vec![0u8; 2];
	// read size of Vec<BlockHeader>
	read_exact(conn, &mut size, time::Duration::from_millis(10), true)?;

	let total_headers = size[0] as u64 * 256 + size[1] as u64;
	if total_headers == 0 || total_headers > 10_000 {
		return Err(Error::Connection(io::Error::new(
			io::ErrorKind::InvalidData,
			"headers_header_size",
		)));
	}
	let average_header_size = (msg_len - 2) / total_headers;

	// support size of Cuckoo: from Cuckoo 30 to Cuckoo 36, with version 2
	// having slightly larger headers
	let minimum_size = core::serialized_size_of_header(1, global::min_sizeshift());
	let maximum_size = core::serialized_size_of_header(2, global::min_sizeshift() + 6);
	if average_header_size < minimum_size as u64 || average_header_size > maximum_size as u64 {
		debug!(
			LOGGER,
			"headers_header_size - size of Vec: {}, average_header_size: {}, min: {}, max: {}",
			total_headers,
			average_header_size,
			minimum_size,
			maximum_size,
		);
		return Err(Error::Connection(io::Error::new(
			io::ErrorKind::InvalidData,
			"headers_header_size",
		)));
	}
	return Ok(maximum_size as u64);
}

/// Read the Headers streaming body from the underlying connection
fn headers_streaming_body(
	conn: &mut TcpStream,   // (i) underlying connection
	msg_len: u64,           // (i) length of whole 'Headers'
	headers_num: u64,       // (i) how many BlockHeader(s) do you want to read
	total_read: &mut u64,   // (i/o) how many bytes already read on this 'Headers' message
	reserved: &mut Vec<u8>, // (i/o) reserved part of previous read, which is not a whole header
	max_header_size: u64,   // (i) maximum possible size of single BlockHeader
) -> Result<Headers, Error> {
	if headers_num == 0 || msg_len < *total_read || *total_read < 2 {
		return Err(Error::Connection(io::Error::new(
			io::ErrorKind::InvalidInput,
			"headers_streaming_body",
		)));
	}

	// Note:
	// As we allow Cuckoo sizes greater than 30 now, the proof of work part of the header
	// could be 30*42 bits, 31*42 bits, 32*42 bits, etc.
	// So, for compatibility with variable size of block header, we read max possible size, for
	// up to Cuckoo 36.
	//
	let mut read_size = headers_num * max_header_size - reserved.len() as u64;
	if *total_read + read_size > msg_len {
		read_size = msg_len - *total_read;
	}

	// 1st part
	let mut body = vec![0u8; 2]; // for Vec<> size
	let mut final_headers_num = (read_size + reserved.len() as u64) / max_header_size;
	let remaining = msg_len - *total_read - read_size;
	if final_headers_num == 0 && remaining == 0 {
		final_headers_num = 1;
	}
	body[0] = (final_headers_num >> 8) as u8;
	body[1] = (final_headers_num & 0x00ff) as u8;

	// 2nd part
	body.append(reserved);

	// 3rd part
	let mut read_body = vec![0u8; read_size as usize];
	if read_size > 0 {
		read_exact(conn, &mut read_body, time::Duration::from_secs(20), true)?;
		*total_read += read_size;
	}
	body.append(&mut read_body);

	// deserialize these assembled 3 parts
	let result: Result<Headers, Error> = ser::deserialize(&mut &body[..]).map_err(From::from);
	let headers = result?;

	// remaining data
	let mut deserialized_size = 2; // for Vec<> size
	for header in &headers.headers {
		deserialized_size += header.serialized_size();
	}
	*reserved = body[deserialized_size..].to_vec();

	Ok(headers)
}
