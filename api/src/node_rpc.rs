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

//! JSON-RPC Stub generation for the Node API

use crate::core::core::hash::Hash;
use crate::core::core::transaction::Transaction;
use crate::node::Node;
use crate::p2p::types::PeerInfoDisplay;
use crate::p2p::PeerData;
use crate::pool::PoolEntry;
use crate::rest::ErrorKind;
use crate::types::{
	BlockHeaderPrintable, BlockPrintable, LocatedTxKernel, OutputListing, OutputPrintable, Status,
	Tip, Version,
};
use crate::util;
use std::net::SocketAddr;

/// Public definition used to generate Node jsonrpc api.
/// * When running `grin` with defaults, the V2 api is available at
/// `localhost:3413/v2`
/// * The endpoint only supports POST operations, with the json-rpc request as the body
#[easy_jsonrpc_mw::rpc]
pub trait NodeRpc: Sync + Send {
	fn get_header(
		&self,
		height: Option<u64>,
		hash: Option<String>,
		commit: Option<String>,
	) -> Result<BlockHeaderPrintable, ErrorKind>;
	fn get_block(
		&self,
		height: Option<u64>,
		hash: Option<String>,
		commit: Option<String>,
	) -> Result<BlockPrintable, ErrorKind>;
	fn get_status(&self) -> Result<Status, ErrorKind>;
	fn get_version(&self) -> Result<Version, ErrorKind>;
	fn get_tip(&self) -> Result<Tip, ErrorKind>;
	fn get_kernel(
		&self,
		excess: String,
		min_height: Option<u64>,
		max_height: Option<u64>,
	) -> Result<LocatedTxKernel, ErrorKind>;
	fn get_outputs(
		&self,
		commits: Option<Vec<String>>,
		start_height: Option<u64>,
		end_height: Option<u64>,
		include_proof: Option<bool>,
		include_merkle_proof: Option<bool>,
	) -> Result<Vec<OutputPrintable>, ErrorKind>;
	fn get_unspent_outputs(
		&self,
		start_index: u64,
		max: u64,
		include_proof: Option<bool>,
	) -> Result<OutputListing, ErrorKind>;
	fn validate_chain(&self) -> Result<(), ErrorKind>;
	fn compact_chain(&self) -> Result<(), ErrorKind>;
	fn get_peers(&self, peer_addr: Option<SocketAddr>) -> Result<Vec<PeerData>, ErrorKind>;
	fn get_connected_peers(&self) -> Result<Vec<PeerInfoDisplay>, ErrorKind>;
	fn ban_peer(&self, peer_addr: SocketAddr) -> Result<(), ErrorKind>;
	fn unban_peer(&self, peer_addr: SocketAddr) -> Result<(), ErrorKind>;
	fn get_pool_size(&self) -> Result<usize, ErrorKind>;
	fn get_stempool_size(&self) -> Result<usize, ErrorKind>;
	fn get_unconfirmed_transactions(&self) -> Result<Vec<PoolEntry>, ErrorKind>;
	fn push_transaction(&self, tx: Transaction, fluff: Option<bool>) -> Result<(), ErrorKind>;
}

impl NodeRpc for Node {
	fn get_header(
		&self,
		height: Option<u64>,
		hash: Option<String>,
		commit: Option<String>,
	) -> Result<BlockHeaderPrintable, ErrorKind> {
		let mut parsed_hash: Option<Hash> = None;
		if let Some(hash) = hash {
			let vec = util::from_hex(hash)
				.map_err(|e| ErrorKind::Argument(format!("invalid block hash: {}", e)))?;
			parsed_hash = Some(Hash::from_vec(&vec));
		}
		Node::get_header(self, height, parsed_hash, commit).map_err(|e| e.kind().clone())
	}
	fn get_block(
		&self,
		height: Option<u64>,
		hash: Option<String>,
		commit: Option<String>,
	) -> Result<BlockPrintable, ErrorKind> {
		let mut parsed_hash: Option<Hash> = None;
		if let Some(hash) = hash {
			let vec = util::from_hex(hash)
				.map_err(|e| ErrorKind::Argument(format!("invalid block hash: {}", e)))?;
			parsed_hash = Some(Hash::from_vec(&vec));
		}
		Node::get_block(self, height, parsed_hash, commit).map_err(|e| e.kind().clone())
	}
	fn get_status(&self) -> Result<Status, ErrorKind> {
		Node::get_status(self).map_err(|e| e.kind().clone())
	}

	fn get_version(&self) -> Result<Version, ErrorKind> {
		Node::get_version(self).map_err(|e| e.kind().clone())
	}

	fn get_tip(&self) -> Result<Tip, ErrorKind> {
		Node::get_tip(self).map_err(|e| e.kind().clone())
	}

	fn get_kernel(
		&self,
		excess: String,
		min_height: Option<u64>,
		max_height: Option<u64>,
	) -> Result<LocatedTxKernel, ErrorKind> {
		Node::get_kernel(self, excess, min_height, max_height).map_err(|e| e.kind().clone())
	}

	fn get_outputs(
		&self,
		commits: Option<Vec<String>>,
		start_height: Option<u64>,
		end_height: Option<u64>,
		include_proof: Option<bool>,
		include_merkle_proof: Option<bool>,
	) -> Result<Vec<OutputPrintable>, ErrorKind> {
		Node::get_outputs(
			self,
			commits,
			start_height,
			end_height,
			include_proof,
			include_merkle_proof,
		)
		.map_err(|e| e.kind().clone())
	}

	fn get_unspent_outputs(
		&self,
		start_index: u64,
		max: u64,
		include_proof: Option<bool>,
	) -> Result<OutputListing, ErrorKind> {
		Node::get_unspent_outputs(self, start_index, max, include_proof)
			.map_err(|e| e.kind().clone())
	}

	fn validate_chain(&self) -> Result<(), ErrorKind> {
		Node::validate_chain(self).map_err(|e| e.kind().clone())
	}

	fn compact_chain(&self) -> Result<(), ErrorKind> {
		Node::compact_chain(self).map_err(|e| e.kind().clone())
	}

	fn get_peers(&self, addr: Option<SocketAddr>) -> Result<Vec<PeerData>, ErrorKind> {
		Node::get_peers(self, addr).map_err(|e| e.kind().clone())
	}

	fn get_connected_peers(&self) -> Result<Vec<PeerInfoDisplay>, ErrorKind> {
		Node::get_connected_peers(self).map_err(|e| e.kind().clone())
	}

	fn ban_peer(&self, addr: SocketAddr) -> Result<(), ErrorKind> {
		Node::ban_peer(self, addr).map_err(|e| e.kind().clone())
	}

	fn unban_peer(&self, addr: SocketAddr) -> Result<(), ErrorKind> {
		Node::unban_peer(self, addr).map_err(|e| e.kind().clone())
	}

	fn get_pool_size(&self) -> Result<usize, ErrorKind> {
		Node::get_pool_size(self).map_err(|e| e.kind().clone())
	}

	fn get_stempool_size(&self) -> Result<usize, ErrorKind> {
		Node::get_stempool_size(self).map_err(|e| e.kind().clone())
	}

	fn get_unconfirmed_transactions(&self) -> Result<Vec<PoolEntry>, ErrorKind> {
		Node::get_unconfirmed_transactions(self).map_err(|e| e.kind().clone())
	}
	fn push_transaction(&self, tx: Transaction, fluff: Option<bool>) -> Result<(), ErrorKind> {
		Node::push_transaction(self, tx, fluff).map_err(|e| e.kind().clone())
	}
}
