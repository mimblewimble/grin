// Copyright 2020 The Grin Developers
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

use super::utils::w;
use crate::error::*;
use crate::p2p::types::{PeerAddr, PeerInfoDisplay, ReasonForBan};
use crate::p2p::{self, PeerData};
use std::net::SocketAddr;
use std::sync::Weak;

pub struct PeersConnectedHandler {
	pub peers: Weak<p2p::Peers>,
}

impl PeersConnectedHandler {
	pub fn get_connected_peers(&self) -> Result<Vec<PeerInfoDisplay>, Error> {
		let peers = w(&self.peers)?
			.iter()
			.connected()
			.into_iter()
			.map(|p| p.info.clone().into())
			.collect::<Vec<PeerInfoDisplay>>();
		Ok(peers)
	}
}

/// Peer operations
pub struct PeerHandler {
	pub peers: Weak<p2p::Peers>,
}

impl PeerHandler {
	pub fn get_peers(&self, addr: Option<SocketAddr>) -> Result<Vec<PeerData>, Error> {
		if let Some(addr) = addr {
			let peer_addr = PeerAddr(addr);
			let peer_data: PeerData = w(&self.peers)?.get_peer(peer_addr).map_err(|e| {
				let e: Error = ErrorKind::Internal(format!("get peer error: {:?}", e)).into();
				e
			})?;
			return Ok(vec![peer_data]);
		}
		let peers = w(&self.peers)?.all_peer_data();
		Ok(peers)
	}

	pub fn ban_peer(&self, addr: SocketAddr) -> Result<(), Error> {
		let peer_addr = PeerAddr(addr);
		w(&self.peers)?
			.ban_peer(peer_addr, ReasonForBan::ManualBan)
			.map_err(|e| ErrorKind::Internal(format!("ban peer error: {:?}", e)).into())
	}

	pub fn unban_peer(&self, addr: SocketAddr) -> Result<(), Error> {
		let peer_addr = PeerAddr(addr);
		w(&self.peers)?
			.unban_peer(peer_addr)
			.map_err(|e| ErrorKind::Internal(format!("unban peer error: {:?}", e)).into())
	}
}
