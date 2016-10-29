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
use std::ops::DerefMut;
use std::rc::Rc;

use mioco;
use mioco::sync::mpsc::sync_channel;
use mioco::tcp::{TcpStream, Shutdown};

use core::ser;
use handshake::Handshake;
use msg::*;
use types::*;

pub struct ProtocolV1 {
	conn: RefCell<TcpStream>,
}

impl Protocol for ProtocolV1 {
	fn handle(&self, server: &NetAdapter) -> Option<ser::Error> {
    // setup a channel so we can switch between reads and writes
    let (send, recv) = sync_channel(10);

    let mut conn = self.conn.borrow_mut();
		loop {
      select!(
        r:conn => {
          let header = try_to_o!(ser::deserialize::<MsgHeader>(conn.deref_mut()));
          if !header.acceptable() {
            continue;
          }
        },
        r:recv => {
          ser::serialize(conn.deref_mut(), recv.recv().unwrap());
        }
      );
		}
	}
}

impl ProtocolV1 {
	pub fn new(conn: TcpStream) -> ProtocolV1 {
		ProtocolV1 { conn: RefCell::new(conn) }
	}

	// fn close(&mut self, err_code: u32, explanation: &'static str) {
	// 	ser::serialize(self.conn,
	// 	               &PeerError {
	// 		               code: err_code,
	// 		               message: explanation.to_string(),
	// 	               });
	// 	self.conn.shutdown(Shutdown::Both);
	// }
}
