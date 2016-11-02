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
use std::io::Write;
use std::ops::{Deref, DerefMut};
use std::sync::Mutex;

use mioco;
use mioco::sync::mpsc::{sync_channel, SyncSender, Receiver};
use mioco::tcp::{TcpStream, Shutdown};

use core::core;
use core::ser;
use msg::*;
use rw;
use types::*;

/// In normal peer operation we don't want to be sent more than 100Mb in a
/// single message.
const MAX_DATA_BYTES: usize = 100 * 1000 * 1000;

/// Number of errors before we disconnect from a peer.
const MAX_ERRORS: u64 = 5;

/// First version of our communication protocol. Manages the underlying
/// connection, listening to incoming messages and transmitting outgoing ones.
pub struct ProtocolV1 {
	// The underlying tcp connection.
	conn: RefCell<TcpStream>,

	// Send channel for the rest of the local system to send messages to the peer we're connected to.
	msg_send: RefCell<Option<SyncSender<Vec<u8>>>>,

	// Stop channel to exit the send/listen loop.
	stop_send: RefCell<Option<SyncSender<u8>>>,

	// Used both to count the amount of data sent and lock writing to the conn. We can't wrap conn with
	// the lock as we're always listening to receive.
	sent_bytes: Mutex<u64>,

	// Bytes we've received.
	received_bytes: Mutex<u64>,

	// Counter for read errors.
	error_count: Mutex<u64>,
}

impl ProtocolV1 {
	/// Creates a new protocol v1
	pub fn new(conn: TcpStream) -> ProtocolV1 {
		ProtocolV1 {
			conn: RefCell::new(conn),
			msg_send: RefCell::new(None),
			stop_send: RefCell::new(None),
			sent_bytes: Mutex::new(0),
			received_bytes: Mutex::new(0),
      error_count: Mutex::new(0),
		}
	}
}

impl Protocol for ProtocolV1 {
	/// Main protocol connection handling loop, starts listening to incoming
	/// messages and transmitting messages the rest of the local system wants
	/// to send. Must be called before any interaction with a protocol instance
	/// and should only be called once. Will block so also needs to be called
	/// within a coroutine.
	fn handle(&self, adapter: &NetAdapter) -> Result<(), ser::Error> {
		// setup channels so we can switch between reads, writes and close
    let (msg_recv, stop_recv) = self.setup_channels();

		let mut conn = self.conn.borrow_mut();
		loop {
			// main select loop, switches between listening, sending or stopping
			select!(
        r:conn => {
          let res = self.read_msg(&mut conn, adapter);
          if let Err(_) = res {
            let mut cnt = self.error_count.lock().unwrap();
            *cnt += 1;
            if *cnt > MAX_ERRORS {
              return res.map(|_| ());
            }
          }
        },
        r:msg_recv => {
			// relay a message originated from the rest of the local system
          let data = &msg_recv.recv().unwrap()[..];
          let mut sent_bytes = self.sent_bytes.lock().unwrap();
          *sent_bytes += data.len() as u64;
          try!(conn.deref_mut().write_all(data).map_err(&ser::Error::IOErr));
        },
        r:stop_recv => {
			// shuts the connection don and end the loop
          stop_recv.recv();
		      conn.shutdown(Shutdown::Both);
          return Ok(());
        }
      );
		}
	}

	/// Sends a ping message to the remote peer. Will panic if handle has never
	/// been called on this protocol.
	fn send_ping(&self) -> Result<(), ser::Error> {
		let data = try!(ser::ser_vec(&MsgHeader::new(Type::Ping)));
		let msg_send = self.msg_send.borrow();
		msg_send.as_ref().unwrap().send(data);
		Ok(())
	}

	/// Serializes and sends a block to our remote peer
	fn send_block(&self, b: &core::Block) -> Result<(), ser::Error> {
		self.send_msg(Type::Block, b)
	}

	/// Serializes and sends a transaction to our remote peer
	fn send_transaction(&self, tx: &core::Transaction) -> Result<(), ser::Error> {
		self.send_msg(Type::Transaction, tx)
	}

	/// Bytes sent and received by this peer to the remote peer.
	fn transmitted_bytes(&self) -> (u64, u64) {
    let sent = *self.sent_bytes.lock().unwrap().deref();
    let received = *self.received_bytes.lock().unwrap().deref();
		(sent, received)
	}

	/// Close the connection to the remote peer
	fn close(&self) {
		let stop_send = self.stop_send.borrow();
		stop_send.as_ref().unwrap().send(0);
	}
}

impl ProtocolV1 {
  fn read_msg(&self, mut conn: &mut TcpStream, adapter: &NetAdapter) -> Result<(), ser::Error> {
    // deser the header to get the message type
    let header = try!(ser::deserialize::<MsgHeader>(conn.deref_mut()));
    if !header.acceptable() {
      return Err(ser::Error::CorruptedData);
    }

    // wrap our connection with limited byte-counting readers
    let mut limit_conn = rw::LimitedRead::new(conn.deref_mut(), MAX_DATA_BYTES);
    let mut read_conn = rw::CountingRead::new(&mut limit_conn);

    // check the message type and hopefully do what's expected with it
    match header.msg_type {
      Type::Ping => {
        // respond with pong
        try!(self.send_pong());
      },
      Type::Pong => {},
      Type::Transaction => {
        let tx = try!(ser::deserialize(&mut read_conn));
        adapter.transaction_received(tx);
      },
      Type::Block => {
        let b = try!(ser::deserialize(&mut read_conn));
        adapter.block_received(b);
      }
      _ => error!("uncaught unknown"),
    }

    // update total of bytes sent
    let mut sent_bytes = self.sent_bytes.lock().unwrap();
    *sent_bytes += header.serialized_len() + (read_conn.bytes_read() as u64);

    Ok(())
  }

	/// Helper function to avoid boilerplate, builds a header followed by the
	/// Writeable body and send the whole thing.
	// TODO serialize straight to the connection
	fn send_msg(&self, t: Type, body: &ser::Writeable) -> Result<(), ser::Error> {
		let mut data = Vec::new();
		try!(ser::serialize(&mut data, &MsgHeader::new(t)));
		try!(ser::serialize(&mut data, body));
		let msg_send = self.msg_send.borrow();
		msg_send.as_ref().unwrap().send(data);
		Ok(())
	}

	fn send_pong(&self) -> Result<(), ser::Error> {
		let data = try!(ser::ser_vec(&MsgHeader::new(Type::Pong)));
		let msg_send = self.msg_send.borrow();
		msg_send.as_ref().unwrap().send(data);
		Ok(())
	}

  /// Setup internal communication channels to select over
  fn setup_channels(&self) -> (Receiver<Vec<u8>>, Receiver<u8>) {
		let (msg_send, msg_recv) = sync_channel(10);
		let (stop_send, stop_recv) = sync_channel(1);
		{
			let mut msg_mut = self.msg_send.borrow_mut();
			*msg_mut = Some(msg_send);
			let mut stop_mut = self.stop_send.borrow_mut();
			*stop_mut = Some(stop_send);
		}
    (msg_recv, stop_recv)
  }
}
