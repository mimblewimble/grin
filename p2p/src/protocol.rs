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
	fn consume<'a>(&self, mut msg: Message<'a>) -> Result<Option<Response<'a>>, Error> {
		let adapter = &self.adapter;

		match msg.header.msg_type {

			Type::Ping => {
				let ping: Ping = msg.body()?;
				adapter.peer_difficulty(self.addr, ping.total_difficulty, ping.height);

				Ok(Some(
					msg.respond(
						Type::Pong,
						Pong {
							total_difficulty: adapter.total_difficulty(),
							height: adapter.total_height(),
						})
				))
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
					return Ok(Some(msg.respond(Type::Block, b)));
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

						Ok(Some(msg.respond(Type::Block, b)))
					} else {
						Ok(Some(msg.respond(Type::CompactBlock, cb)))
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
				Ok(Some(msg.respond(Type::Headers, Headers { headers: headers })))
			}

			Type::Headers => {
				let headers: Headers = msg.body()?;
				adapter.headers_received(headers.headers, self.addr);
				Ok(None)
			}

			Type::GetPeerAddrs => {
				let get_peers: GetPeerAddrs = msg.body()?;
				let peer_addrs = adapter.find_peer_addrs(get_peers.capabilities);
				Ok(Some(
						msg.respond(
							Type::PeerAddrs,
							PeerAddrs {
								peers: peer_addrs.iter().map(|sa| SockAddr(*sa)).collect(),
							})
				))
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



//			Type::SumtreesRequest => {
//				let sm_req = ser::deserialize::<SumtreesRequest>(&mut &buf[..])?;
//				debug!(LOGGER, "handle_payload: sumtree req for {} at {}",
//							sm_req.hash, sm_req.height);
//
//				let sumtrees = self.adapter.sumtrees_read(sm_req.hash);
//
//				if let Some(mut sumtrees) = sumtrees {
//					let file_sz = sumtrees.reader.metadata()?.len();
//
//					// first send the sumtree archive information
//					let mut body_data = vec![];
//					ser::serialize(&mut body_data,
//						&SumtreesArchive {
//							height: sm_req.height as u64,
//							hash: sm_req.hash,
//							rewind_to_output: sumtrees.output_index,
//							rewind_to_kernel: sumtrees.kernel_index,
//							bytes: file_sz,
//						})?;
//
//					let mut data = vec![];
//					try!(ser::serialize(
//							&mut data,
//							&MsgHeader::new(Type::SumtreesArchive, body_data.len() as u64),
//							));
//					data.append(&mut body_data);
//
//					if let Err(e) = sender.unbounded_send(data) {
//						debug!(LOGGER, "handle_payload: error sending sumtrees info: {:?}", e);
//					}
//
//					// second, send the archive byte stream
//					debug!(LOGGER, "handle_payload: sumtree archive metadata sent, preparing to stream");
//					loop {
//						let mut buf = [0; 8000];
//						let len = sumtrees.reader.read(&mut buf)?;
//						debug!(LOGGER, "handle_payload: sending {} bytes of sumtree data", len);
//						if let Err(e) = sender.unbounded_send(buf[0..len].to_vec()) {
//							debug!(LOGGER, "handle_payload: error sending sumtrees: {:?}", e);
//						}
//						if len == 0 {
//							break;
//						}
//					}
//					debug!(LOGGER, "handle_payload: stream sent");
//				}
//				Ok(None)
//			}
//
//			Type::SumtreesArchive => {
//				let sm_arch = ser::deserialize::<SumtreesArchive>(&mut &buf[..])?;
//				debug!(LOGGER, "handle_payload: sumtree archive for {} at {} rewind to {}/{}",
//							sm_arch.hash, sm_arch.height,
//							sm_arch.rewind_to_output, sm_arch.rewind_to_kernel);
//
//				let mut tmp = env::temp_dir();
//				tmp.push("sumtree.zip");
//				{
//					let mut tmp_zip = File::create(tmp.clone())?;
//	
//					// can't simply use io::copy as we're dealing with an async socket
//					// TODO abort if nothing gets sent for 5 secs
//					let mut buffer = [0; 8000];
//					let mut total_size = 0;
//					'outer: loop {
//						let res = reader.read(&mut buffer);
//						match res {
//							Ok(n) => {
//								if n == 0 {
//									break 'outer;
//								}
//								tmp_zip.write(&buffer[0..n]);
//								total_size += n;
//								if total_size as u64 >= sm_arch.bytes {
//									break 'outer;
//								}
//							}
//							Err(e) => {
//								debug!(LOGGER, "err: {:?}", e);
//							}
//						}
//					}
//					debug!(LOGGER, "handle_payload: wrote {} bytes sumtree archive", total_size);
//					tmp_zip.sync_all()?;
//				}
//
//				let tmp_zip = File::open(tmp)?;
//				self.adapter.sumtrees_write(
//					sm_arch.hash, sm_arch.rewind_to_output, sm_arch.rewind_to_kernel, tmp_zip);
//				Ok(None)
//			}
