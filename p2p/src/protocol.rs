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

use std::sync::{Mutex, Arc};

use futures;
use futures::Future;
use futures::stream;
use futures::sync::mpsc::UnboundedSender;
use tokio_core::net::TcpStream;

use core::core;
use core::core::hash::Hash;
use core::ser;
use conn::Connection;
use msg::*;
use types::*;
use util::OneTime;

pub struct ProtocolV1 {
  conn: OneTime<Connection>,

  expected_responses: Mutex<Vec<(Type, Hash)>>,
}

impl ProtocolV1 {
  pub fn new() -> ProtocolV1 {
    ProtocolV1 {
      conn: OneTime::new(),
      expected_responses: Mutex::new(vec![]),
    }
  }
}

impl Protocol for ProtocolV1 {
	/// Sets up the protocol reading, writing and closing logic.
	fn handle(&self,
	          conn: TcpStream,
	          adapter: Arc<NetAdapter>)
	          -> Box<Future<Item = (), Error = ser::Error>> {

    let (conn, listener) = Connection::listen(conn, move |sender, header, data| {
      let adapt = adapter.as_ref();
      handle_payload(adapt, sender, header, data)
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

	fn send_msg(&self, t: Type, body: &ser::Writeable) -> Result<(), ser::Error> {
    self.conn.borrow().send_msg(t, body)
  }

	fn send_request(&self, t: Type, body: &ser::Writeable, expect_resp: Option<(Type, Hash)>) -> Result<(), ser::Error> {
    let sent = self.send_msg(t, body);

		if let Err(e) = sent {
			warn!("Couldn't send message to remote peer: {}", e);
		} else if let Some(exp) = expect_resp {
      let mut expects = self.expected_responses.lock().unwrap();
      expects.push(exp);
    }
    Ok(())
  }
}

fn handle_payload(adapter: &NetAdapter,
                  sender: UnboundedSender<Vec<u8>>,
                  header: MsgHeader,
                  buf: Vec<u8>)
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
