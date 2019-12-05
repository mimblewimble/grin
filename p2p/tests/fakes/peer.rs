// Copyright 2019 The Grin Developers
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

use grin_chain::Options;
use grin_core::core::hash::Hash;
use grin_core::core::{Block, BlockHeader, CompactBlock, Transaction};
use grin_core::pow::Difficulty;
use grin_core::ser::ProtocolVersion;
use grin_p2p::types::PeerLiveInfo;
use grin_p2p::{Capabilities, ConnectedPeer, Direction, Error, PeerAddr, PeerInfo, ReasonForBan};
use grin_util::RwLock;
use std::cell::Cell;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;

pub struct FakePeerFactory {
	count: Cell<u8>,
}

impl FakePeerFactory {
	pub fn new() -> FakePeerFactory {
		FakePeerFactory {
			count: Cell::new(0),
		}
	}

	pub fn build(&self) -> FakePeer {
		self.count.set(self.count.get() + 1);

		FakePeer {
			info: PeerInfo {
				addr: PeerAddr::from_ip(IpAddr::V4(Ipv4Addr::new(127, 0, 0, self.count.get()))),
				capabilities: Capabilities::FULL_NODE,
				user_agent: "".to_string(),
				version: ProtocolVersion::local(),
				live_info: Arc::new(RwLock::new(PeerLiveInfo::new(Difficulty::min()))),
				direction: Direction::Outbound,
			},
		}
	}
}

#[derive(Debug, Clone)]
pub struct FakePeer {
	pub info: PeerInfo,
}

impl FakePeer {}

impl ConnectedPeer for FakePeer {
	fn info(&self) -> &PeerInfo {
		&self.info
	}

	fn is_connected(&self) -> bool {
		true
	}

	fn is_banned(&self) -> bool {
		unimplemented!()
	}

	fn is_stuck(&self) -> (bool, Difficulty) {
		unimplemented!()
	}

	fn is_abusive(&self) -> bool {
		unimplemented!()
	}

	fn last_min_sent_bytes(&self) -> Option<u64> {
		unimplemented!()
	}

	fn last_min_received_bytes(&self) -> Option<u64> {
		unimplemented!()
	}

	fn last_min_message_counts(&self) -> Option<(u64, u64)> {
		unimplemented!()
	}

	fn set_banned(&self) {
		unimplemented!()
	}

	fn send_ping(&self, _total_difficulty: Difficulty, _height: u64) -> Result<(), Error> {
		unimplemented!()
	}

	fn send_ban_reason(&self, _ban_reason: ReasonForBan) -> Result<(), Error> {
		unimplemented!()
	}

	fn send_block(&self, _b: &Block) -> Result<bool, Error> {
		unimplemented!()
	}

	fn send_compact_block(&self, _b: &CompactBlock) -> Result<bool, Error> {
		unimplemented!()
	}

	fn send_header(&self, _bh: &BlockHeader) -> Result<bool, Error> {
		unimplemented!()
	}

	fn send_tx_kernel_hash(&self, _h: Hash) -> Result<bool, Error> {
		unimplemented!()
	}

	fn send_transaction(&self, _tx: &Transaction) -> Result<bool, Error> {
		unimplemented!()
	}

	fn send_stem_transaction(&self, _tx: &Transaction) -> Result<(), Error> {
		unimplemented!()
	}

	fn send_header_request(&self, _locator: Vec<Hash>) -> Result<(), Error> {
		unimplemented!()
	}

	fn send_tx_request(&self, _h: Hash) -> Result<(), Error> {
		unimplemented!()
	}

	fn send_block_request(&self, _h: Hash, _opts: Options) -> Result<(), Error> {
		unimplemented!()
	}

	fn send_compact_block_request(&self, _h: Hash) -> Result<(), Error> {
		unimplemented!()
	}

	fn send_peer_request(&self, _capab: Capabilities) -> Result<(), Error> {
		unimplemented!()
	}

	fn send_txhashset_request(&self, _height: u64, _hash: Hash) -> Result<(), Error> {
		unimplemented!()
	}

	fn send_kernel_data_request(&self) -> Result<(), Error> {
		unimplemented!()
	}

	fn stop(&self) {
		unimplemented!()
	}

	fn wait(&self) {
		unimplemented!()
	}
}
