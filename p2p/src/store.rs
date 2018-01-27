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
use rand::{thread_rng, Rng};

use core::ser::{self, Readable, Reader, Writeable, Writer};
use grin_store::{self, option_to_not_found, to_key, Error};
use msg::SockAddr;
use types::Capabilities;
use util::LOGGER;

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
#[derive(Debug, Clone, Serialize)]
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
	/// The time the peer was last banned
	pub last_banned: i64,
}

impl Writeable for PeerData {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		SockAddr(self.addr).write(writer)?;
		ser_multiwrite!(
			writer,
			[write_u32, self.capabilities.bits()],
			[write_bytes, &self.user_agent],
			[write_u8, self.flags as u8],
			[write_i64, self.last_banned]
		);
		Ok(())
	}
}

impl Readable for PeerData {
	fn read(reader: &mut Reader) -> Result<PeerData, ser::Error> {
		let addr = SockAddr::read(reader)?;
		let (capab, ua, fl, lb) = ser_multiread!(reader, read_u32, read_vec, read_u8, read_i64);
		let user_agent = String::from_utf8(ua).map_err(|_| ser::Error::CorruptedData)?;
		let capabilities = Capabilities::from_bits(capab).ok_or(ser::Error::CorruptedData)?;
		let last_banned = lb;
		match State::from_u8(fl) {
			Some(flags) => Ok(PeerData {
				addr: addr.0,
				capabilities: capabilities,
				user_agent: user_agent,
				flags: flags,
				last_banned: last_banned,
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
		debug!(LOGGER, "save_peer: {:?} marked {:?}", p.addr, p.flags);

		self.db.put_ser(&peer_key(p.addr)[..], p)
	}

	pub fn get_peer(&self, peer_addr: SocketAddr) -> Result<PeerData, Error> {
		option_to_not_found(self.db.get_ser(&peer_key(peer_addr)[..]))
	}

	pub fn exists_peer(&self, peer_addr: SocketAddr) -> Result<bool, Error> {
		self.db.exists(&peer_key(peer_addr)[..])
	}

	/// TODO - allow below added to avoid github issue reports
	#[allow(dead_code)]
	pub fn delete_peer(&self, peer_addr: SocketAddr) -> Result<(), Error> {
		self.db.delete(&peer_key(peer_addr)[..])
	}

	pub fn find_peers(&self, state: State, cap: Capabilities, count: usize) -> Vec<PeerData> {
		let mut peers = self.db
			.iter::<PeerData>(&to_key(PEER_PREFIX, &mut "".to_string().into_bytes()))
			.filter(|p| p.flags == state && p.capabilities.contains(cap))
			.collect::<Vec<_>>();
		thread_rng().shuffle(&mut peers[..]);
		peers.iter().take(count).cloned().collect()
	}

	/// List all known peers
	/// Used for /v1/peers/all api endpoint
	pub fn all_peers(&self) -> Vec<PeerData> {
		self.db
			.iter::<PeerData>(&to_key(PEER_PREFIX, &mut "".to_string().into_bytes()))
			.collect::<Vec<_>>()
	}

	/// Convenience method to load a peer data, update its status and save it
	/// back.
	pub fn update_state(&self, peer_addr: SocketAddr, new_state: State) -> Result<(), Error> {
		let mut peer = self.get_peer(peer_addr)?;
		peer.flags = new_state;
		self.save_peer(&peer)
	}

	/// Convenience method to load a peer data, update its last banned time and
	/// save it back.
	pub fn update_last_banned(&self, peer_addr: SocketAddr, last_banned: i64) -> Result<(), Error> {
		let mut peer = self.get_peer(peer_addr)?;
		peer.last_banned = last_banned;
		self.save_peer(&peer)
	}
}

fn peer_key(peer_addr: SocketAddr) -> Vec<u8> {
	to_key(PEER_PREFIX, &mut format!("{}:{}", peer_addr.ip(), peer_addr.port()).into_bytes())
}
