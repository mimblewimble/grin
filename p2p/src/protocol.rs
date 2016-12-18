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

use std::cell::RefCell;
use std::iter;
use std::ops::DerefMut;
use std::sync::{Mutex, Arc};

use futures;
use futures::{Stream, Future};
use futures::stream;
use futures::sync::mpsc::{UnboundedSender, UnboundedReceiver};
use tokio_core::io::{Io, WriteHalf, ReadHalf, write_all, read_exact};
use tokio_core::net::TcpStream;

use core::core;
use core::ser;
use msg::*;
use types::*;

pub struct ProtocolV1 {
	outbound_chan: RefCell<Option<UnboundedSender<Vec<u8>>>>,

	// Bytes we've sent.
	sent_bytes: Arc<Mutex<u64>>,

	// Bytes we've received.
	received_bytes: Arc<Mutex<u64>>,

	// Counter for read errors.
	error_count: Mutex<u64>,
}

impl ProtocolV1 {
	pub fn new() -> ProtocolV1 {
		ProtocolV1 {
			outbound_chan: RefCell::new(None),
			sent_bytes: Arc::new(Mutex::new(0)),
			received_bytes: Arc::new(Mutex::new(0)),
			error_count: Mutex::new(0),
		}
	}
}

impl Protocol for ProtocolV1 {
	fn handle(&self,
	          conn: TcpStream,
	          adapter: Arc<NetAdapter>)
	          -> Box<Future<Item = (), Error = ser::Error>> {
		let (reader, writer) = conn.split();

		// prepare the channel that will transmit data to the connection writer
		let (tx, rx) = futures::sync::mpsc::unbounded();
		{
			let mut out_mut = self.outbound_chan.borrow_mut();
			*out_mut = Some(tx.clone());
		}

		// setup the reading future, getting messages from the peer and processing them
		let read_msg = self.read_msg(tx, reader, adapter).map(|_| ());

		// setting the writing future, getting messages from our system and sending
		// them out
		let write_msg = self.write_msg(rx, writer).map(|_| ());

		// select between our different futures and return them
		Box::new(read_msg.select(write_msg).map(|_| ()).map_err(|(e, _)| e))
	}

	/// Bytes sent and received by this peer to the remote peer.
	fn transmitted_bytes(&self) -> (u64, u64) {
		let sent = *self.sent_bytes.lock().unwrap();
		let recv = *self.received_bytes.lock().unwrap();
		(sent, recv)
	}

	/// Sends a ping message to the remote peer. Will panic if handle has never
	/// been called on this protocol.
	fn send_ping(&self) -> Result<(), ser::Error> {
		self.send_msg(Type::Ping, &Empty {})
	}

	/// Serializes and sends a block to our remote peer
	fn send_block(&self, b: &core::Block) -> Result<(), ser::Error> {
		self.send_msg(Type::Block, b)
	}

	/// Serializes and sends a transaction to our remote peer
	fn send_transaction(&self, tx: &core::Transaction) -> Result<(), ser::Error> {
		self.send_msg(Type::Transaction, tx)
	}

	/// Close the connection to the remote peer
	fn close(&self) {
		// TODO some kind of shutdown signal
	}
}

impl ProtocolV1 {
	/// Prepares the future reading from the peer connection, parsing each
	/// message and forwarding them appropriately based on their type
	fn read_msg(&self,
	            sender: UnboundedSender<Vec<u8>>,
	            reader: ReadHalf<TcpStream>,
	            adapter: Arc<NetAdapter>)
	            -> Box<Future<Item = ReadHalf<TcpStream>, Error = ser::Error>> {

		// infinite iterator stream so we repeat the message reading logic until the
		// peer is stopped
		let iter = stream::iter(iter::repeat(()).map(Ok::<(), ser::Error>));

		// setup the reading future, getting messages from the peer and processing them
		let recv_bytes = self.received_bytes.clone();
		let read_msg = iter.fold(reader, move |reader, _| {
			let mut sender_inner = sender.clone();
			let recv_bytes = recv_bytes.clone();
			let adapter = adapter.clone();

			// first read the message header
			read_exact(reader, vec![0u8; HEADER_LEN as usize])
				.map_err(|e| ser::Error::IOErr(e))
				.and_then(move |(reader, buf)| {
					let header = try!(ser::deserialize::<MsgHeader>(&mut &buf[..]));
					Ok((reader, header))
				})
				.and_then(move |(reader, header)| {
					// now that we have a size, proceed with the body
					read_exact(reader, vec![0u8; header.msg_len as usize])
						.map(|(reader, buf)| (reader, header, buf))
						.map_err(|e| ser::Error::IOErr(e))
				})
				.map(move |(reader, header, buf)| {
					// add the count of bytes received
					let mut recv_bytes = recv_bytes.lock().unwrap();
					*recv_bytes += header.serialized_len() + header.msg_len;

					// and handle the different message types
					if let Err(e) = handle_payload(adapter, &header, buf, &mut sender_inner) {
						debug!("Invalid {:?} message: {}", header.msg_type, e);
					}

					reader
				})
		});
		Box::new(read_msg)
	}

	/// Prepares the future that gets message data produced by our system and
	/// sends it to the peer connection
	fn write_msg(&self,
	             rx: UnboundedReceiver<Vec<u8>>,
	             writer: WriteHalf<TcpStream>)
	             -> Box<Future<Item = WriteHalf<TcpStream>, Error = ser::Error>> {

		let sent_bytes = self.sent_bytes.clone();
		let send_data = rx.map(move |data| {
        // add the count of bytes sent
				let mut sent_bytes = sent_bytes.lock().unwrap();
				*sent_bytes += data.len() as u64;
				data
			})
      // write the data and make sure the future returns the right types
			.fold(writer,
			      |writer, data| write_all(writer, data).map_err(|_| ()).map(|(writer, buf)| writer))
			.map_err(|_| ser::Error::CorruptedData);
		Box::new(send_data)
	}

	/// Utility function to send any Writeable. Handles adding the header and
	/// serialization.
	fn send_msg(&self, t: Type, body: &ser::Writeable) -> Result<(), ser::Error> {
		let mut body_data = vec![];
		try!(ser::serialize(&mut body_data, body));
		let mut data = vec![];
		try!(ser::serialize(&mut data, &MsgHeader::new(t, body_data.len() as u64)));
		data.append(&mut body_data);

		let mut msg_send = self.outbound_chan.borrow_mut();
		if let Err(e) = msg_send.deref_mut().as_mut().unwrap().send(data) {
			warn!("Couldn't send message to remote peer: {}", e);
		}
		Ok(())
	}
}

fn handle_payload(adapter: Arc<NetAdapter>,
                  header: &MsgHeader,
                  buf: Vec<u8>,
                  sender: &mut UnboundedSender<Vec<u8>>)
                  -> Result<(), ser::Error> {
	match header.msg_type {
		Type::Ping => {
			let data = try!(ser::ser_vec(&MsgHeader::new(Type::Pong, 0)));
			sender.send(data);
		}
		Type::Pong => {}
		Type::Transaction => {
			let tx = try!(ser::deserialize::<core::Transaction>(&mut &buf[..]));
			adapter.transaction_received(tx);
		}
		Type::Block => {
			let b = try!(ser::deserialize::<core::Block>(&mut &buf[..]));
			adapter.block_received(b);
		}
		_ => {
			debug!("unknown message type {:?}", header.msg_type);
		}
	};
	Ok(())
}
