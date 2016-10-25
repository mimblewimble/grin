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

use types::*;
use core::ser;

pub struct ProtocolV1 {
	comm: &mut Comm,
}

impl Protocol for ProtocolV1 {
	fn new(p: &mut Comm) -> Protocol {
		Protocol { comm: p }
	}
	fn handle(&self, server: &Server) {
		loop {
			let header = ser::deserialize::<MsgHeader>();
			if !header.acceptable() {
				continue;
			}
		}
	}
}

impl ProtocolV1 {
	fn close(err_code: u32, explanation: &'static str) {
		serialize(self.peer,
		          &Err {
			          code: err_code,
			          message: explanation,
		          });
		self.comm.close();
	}
}
