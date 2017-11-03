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

use core::ser::{self, Readable, Reader, Writeable, Writer};
use grin_store::{self, option_to_not_found, to_key, Error};
use msg::SockAddr;
use types::Capabilities;

const STORE_SUBPATH: &'static str = "peers";

const PEER_PREFIX: u8 = 'p' as u8;

/// Types of messages
enum_from_primitive! {
	#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
	pub enum State {
		Healthy,
		Banned,
		Defunct,
	}
}

/// Data stored for any given peer we've encountered.
#[derive(Debug, Serialize)]
pub struct PeerData {
	/// Network address of the peer.
	pub addr: SocketAddr,
	/// What capabilities the peer advertises. Unknown until a successful
	/// connection.
	pub capabilities: Capabilities,
	/// The peer user agent.
	pub user_agent: String,
	/// State the peer has been detected with.
	pub flags: State,
}

impl Writeable for PeerData {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		SockAddr(self.addr).write(writer)?;
		ser_multiwrite!(
			writer,
			[write_u32, self.capabilities.bits()],
			[write_bytes, &self.user_agent],
			[write_u8, self.flags as u8]
		);
		Ok(())
	}
}

impl Readable for PeerData {
	fn read(reader: &mut Reader) -> Result<PeerData, ser::Error> {
		let addr = SockAddr::read(reader)?;
		let (capab, ua, fl) = ser_multiread!(reader, read_u32, read_vec, read_u8);
		let user_agent = String::from_utf8(ua).map_err(|_| ser::Error::CorruptedData)?;
		let capabilities = Capabilities::from_bits(capab).ok_or(ser::Error::CorruptedData)?;
		match State::from_u8(fl) {
			Some(flags) => Ok(PeerData {
				addr: addr.0,
				capabilities: capabilities,
				user_agent: user_agent,
				flags: flags,
			}),
			None => Err(ser::Error::CorruptedData),
		}
	}
}

/// Storage facility for peer data.
pub struct PeerStore {
	db: grin_store::Store,
}

impl PeerStore {
	/// Instantiates a new peer store under the provided root path.
	pub fn new(root_path: String) -> Result<PeerStore, Error> {
		let db = grin_store::Store::open(format!("{}/{}", root_path, STORE_SUBPATH).as_str())?;
		Ok(PeerStore { db: db })
	}

	pub fn save_peer(&self, p: &PeerData) -> Result<(), Error> {
		// we want to ignore any peer without a well-defined ip
		let ip = p.addr.ip();
		if ip.is_unspecified() || ip.is_loopback() {
			return Ok(());
		}
		self.db.put_ser(
			&to_key(PEER_PREFIX, &mut format!("{}", p.addr).into_bytes())[..],
			p,
		)
	}

	fn get_peer(&self, peer_addr: SocketAddr) -> Result<PeerData, Error> {
		option_to_not_found(self.db.get_ser(&peer_key(peer_addr)[..]))
	}

	pub fn exists_peer(&self, peer_addr: SocketAddr) -> Result<bool, Error> {
		self.db
			.exists(&to_key(PEER_PREFIX, &mut format!("{}", peer_addr).into_bytes())[..])
	}

	pub fn delete_peer(&self, peer_addr: SocketAddr) -> Result<(), Error> {
		self.db
			.delete(&to_key(PEER_PREFIX, &mut format!("{}", peer_addr).into_bytes())[..])
	}

	pub fn find_peers(&self, state: State, cap: Capabilities, count: usize) -> Vec<PeerData> {
		let peers_iter = self.db
			.iter::<PeerData>(&to_key(PEER_PREFIX, &mut "".to_string().into_bytes()));
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

	/// List all known peers (for the /v1/peers api endpoint)
	pub fn all_peers(&self) -> Vec<PeerData> {
		let peers_iter = self.db
			.iter::<PeerData>(&to_key(PEER_PREFIX, &mut "".to_string().into_bytes()));
		let mut peers = vec![];
		for p in peers_iter {
			peers.push(p);
		}
		peers
	}

	/// Convenience method to load a peer data, update its status and save it
	/// back.
	pub fn update_state(&self, peer_addr: SocketAddr, new_state: State) -> Result<(), Error> {
		let mut peer = self.get_peer(peer_addr)?;
		peer.flags = new_state;
		self.save_peer(&peer)
	}
}

fn peer_key(peer_addr: SocketAddr) -> Vec<u8> {
	to_key(PEER_PREFIX, &mut format!("{}", peer_addr).into_bytes())
}
