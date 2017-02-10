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

//! Storage implementation for peer data.

use std::net::SocketAddr;
use num::FromPrimitive;

use core::ser::{self, Readable, Writeable, Reader, Writer};
use grin_store::{self, Error, to_key};
use msg::SockAddr;
use types::Capabilities;

const STORE_SUBPATH: &'static str = "peers";

const PEER_PREFIX: u8 = 'p' as u8;

/// Types of messages
enum_from_primitive! {
  #[derive(Debug, Clone, Copy, PartialEq)]
  pub enum State {
    Healthy,
    Banned,
    Dead,
  }
}

pub struct Peer {
	pub addr: SocketAddr,
	pub capabilities: Capabilities,
	pub user_agent: String,
  pub flags: State 
}

impl Writeable for Peer {
	fn write(&self, writer: &mut Writer) -> Result<(), ser::Error> {
		SockAddr(self.addr).write(writer)?;
		ser_multiwrite!(writer,
		                [write_u32, self.capabilities.bits()],
		                [write_bytes, &self.user_agent],
                    [write_u8, self.flags as u8]);
    Ok(())
  }
}

impl Readable<Peer> for Peer {
	fn read(reader: &mut Reader) -> Result<Peer, ser::Error> {
		let addr = SockAddr::read(reader)?;
		let (capab, ua, fl) = ser_multiread!(reader, read_u32, read_vec, read_u8);
		let user_agent = String::from_utf8(ua).map_err(|_| ser::Error::CorruptedData)?;
		let capabilities = Capabilities::from_bits(capab).ok_or(ser::Error::CorruptedData)?;
    match State::from_u8(fl) {
      Some(flags) => {
        Ok(Peer {
          addr: addr.0,
          capabilities: capabilities,
          user_agent: user_agent,
          flags: flags,
        })
      }
			None => Err(ser::Error::CorruptedData),
    }
  }
}

pub struct PeerStore {
	db: grin_store::Store,
}

impl PeerStore {
	pub fn new(root_path: String) -> Result<PeerStore, Error> {
		let db = grin_store::Store::open(format!("{}/{}", root_path, STORE_SUBPATH).as_str())?;
		Ok(PeerStore { db: db })
  }

  pub fn save_peer(&self, p: &Peer) -> Result<(), Error> {
		self.db.put_ser(&to_key(PEER_PREFIX, &mut format!("{}", p.addr).into_bytes())[..], p)
  }

  pub fn delete_peer(&self, peer_addr: SocketAddr) -> Result<(), Error> {
		self.db.delete(&to_key(PEER_PREFIX, &mut format!("{}", peer_addr).into_bytes())[..])
  }

  pub fn find_peers(&self, state: State, cap: Capabilities, count: usize) -> Vec<Peer> {
    let peers_iter = self.db.iter::<Peer>(&to_key(PEER_PREFIX, &mut "".to_string().into_bytes()));
    let mut peers = Vec::with_capacity(count);
    for p in peers_iter {
      if p.flags == state && p.capabilities.contains(cap) {
        peers.push(p);
      }
      if peers.len() >= count {
        break;
      }
    }
    peers
  }
}
