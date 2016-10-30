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
use mioco::sync::mpsc::{sync_channel, SyncSender};
use mioco::tcp::{TcpStream, Shutdown};

use core::ser;
use handshake::Handshake;
use msg::*;
use types::*;

pub struct ProtocolV1 {
	conn: RefCell<TcpStream>,
  //msg_send: Option<SyncSender<ser::Writeable>>,
  stop_send: RefCell<Option<SyncSender<u8>>>,
}

impl Protocol for ProtocolV1 {
	fn handle(&self, server: &NetAdapter) -> Option<ser::Error> {
    // setup channels so we can switch between reads, writes and close
    let (msg_send, msg_recv) = sync_channel(10);
    let (stop_send, stop_recv) = sync_channel(1);

    //self.msg_send = Some(msg_send);
    let mut stop_mut = self.stop_send.borrow_mut();
    *stop_mut = Some(stop_send);

    let mut conn = self.conn.borrow_mut();
		loop {
      select!(
        r:conn => {
          let header = try_to_o!(ser::deserialize::<MsgHeader>(conn.deref_mut()));
          if !header.acceptable() {
            continue;
          }
        },
        r:msg_recv => {
          ser::serialize(conn.deref_mut(), msg_recv.recv().unwrap());
        },
        r:stop_recv => {
          stop_recv.recv();
		      conn.shutdown(Shutdown::Both);
          return None;;
        }
      );
		}
	}

  fn close(&self) {
    let stop_send = self.stop_send.borrow();
    stop_send.as_ref().unwrap().send(0);
  }
}

impl ProtocolV1 {
	pub fn new(conn: TcpStream) -> ProtocolV1 {
		ProtocolV1 { conn: RefCell::new(conn), /* msg_send: None, */ stop_send: RefCell::new(None) }
	}
}
