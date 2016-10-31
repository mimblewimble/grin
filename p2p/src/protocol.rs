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
use std::io::{Read, Write};
use std::ops::{Deref, DerefMut};
use std::rc::Rc;
use std::sync::Mutex;

use mioco;
use mioco::sync::mpsc::{sync_channel, SyncSender};
use mioco::tcp::{TcpStream, Shutdown};

use core::ser;
use handshake::Handshake;
use msg::*;
use types::*;

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
}

impl Protocol for ProtocolV1 {
	/// Main protocol connection handling loop, starts listening to incoming
	/// messages and transmitting messages the rest of the local system wants
	/// to send. Must be called before any interaction with a protocol instance
	/// and should only be called once. Will block so also needs to be called
	/// within a coroutine.
	fn handle(&self, server: &NetAdapter) -> Option<ser::Error> {
		// setup channels so we can switch between reads, writes and close
		let (msg_send, msg_recv) = sync_channel(10);
		let (stop_send, stop_recv) = sync_channel(1);
		{
			let mut msg_mut = self.msg_send.borrow_mut();
			*msg_mut = Some(msg_send);
			let mut stop_mut = self.stop_send.borrow_mut();
			*stop_mut = Some(stop_send);
		}

		let mut conn = self.conn.borrow_mut();
		loop {
			// main select loop, switches between listening, sending or stopping
			select!(
        r:conn => {
			// deser the header ot get the message type
          let header = try_to_o!(ser::deserialize::<MsgHeader>(conn.deref_mut()));
          if !header.acceptable() {
            continue;
          }
			// check the message and hopefully do what's expected
          match header.msg_type {
            Type::Ping => {
			// respond with pong
              let data = try_to_o!(ser::ser_vec(&MsgHeader::new(Type::Pong)));
              let mut sent_bytes = self.sent_bytes.lock().unwrap();
              *sent_bytes += data.len() as u64;
              try_to_o!(conn.deref_mut().write_all(&data[..]).map_err(&ser::Error::IOErr));
            },
            Type::Pong => {},
            _ => error!("uncaught unknown"),
          }
        },
        r:msg_recv => {
			// relay a message originated from the rest of the local system
          let data = &msg_recv.recv().unwrap()[..];
          let mut sent_bytes = self.sent_bytes.lock().unwrap();
          *sent_bytes += data.len() as u64;
          try_to_o!(conn.deref_mut().write_all(data).map_err(&ser::Error::IOErr));
        },
        r:stop_recv => {
			// shuts the connection don and end the loop
          stop_recv.recv();
		      conn.shutdown(Shutdown::Both);
          return None;
        }
      );
		}
	}

	/// Sends a ping message to the remote peer. Will panic if handle has never
	/// been called on this protocol.
	fn send_ping(&self) -> Option<ser::Error> {
		let data = try_to_o!(ser::ser_vec(&MsgHeader::new(Type::Ping)));
		let msg_send = self.msg_send.borrow();
		msg_send.as_ref().unwrap().send(data);
		None
	}

	fn sent_bytes(&self) -> u64 {
		*self.sent_bytes.lock().unwrap().deref()
	}

	fn close(&self) {
		let stop_send = self.stop_send.borrow();
		stop_send.as_ref().unwrap().send(0);
	}
}

impl ProtocolV1 {
	pub fn new(conn: TcpStream) -> ProtocolV1 {
		ProtocolV1 {
			conn: RefCell::new(conn),
			msg_send: RefCell::new(None),
			stop_send: RefCell::new(None),
			sent_bytes: Mutex::new(0),
		}
	}
}
