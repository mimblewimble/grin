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

use core::ser;
use msg::*;
use types::*;
use peer::PeerConn;

pub struct ProtocolV1<'a> {
	peer: &'a mut PeerConn,
}

impl<'a> Protocol for ProtocolV1<'a> {
	fn handle(&mut self, server: &NetAdapter) -> Option<ser::Error> {
		loop {
			let header = try_to_o!(ser::deserialize::<MsgHeader>(self.peer));
			if !header.acceptable() {
				continue;
			}
		}
	}
}

impl<'a> ProtocolV1<'a> {
  pub fn new(p: &mut PeerConn) -> ProtocolV1 {
    ProtocolV1{peer: p}
  }

	fn close(&mut self, err_code: u32, explanation: &'static str) {
		ser::serialize(self.peer,
		          &PeerError {
			          code: err_code,
			          message: explanation.to_string(),
		          });
		self.peer.close();
	}
}
