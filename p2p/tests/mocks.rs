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

//! Mocks for testing

extern crate grin_core as core;
extern crate grin_p2p as p2p;
extern crate env_logger;
extern crate futures;
extern crate tokio_core;

use std::cell::RefCell;
use std::net::SocketAddr;
use std::ops::Deref;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use std::io;

use futures::{Future, Stream};
use futures::future::{self, IntoFuture};
use tokio_core::net::{TcpListener, TcpStream};
use tokio_core::reactor;

use core::core::hash::Hash;
use core::core::target::Difficulty;
use core::core::*;

use p2p::handshake::Handshake;
use p2p::{Peer, PeerInfo, NetAdapter, Error, Capabilities};


/// A no-op network adapter used for testing.
pub struct DummyAdapter {}
impl NetAdapter for DummyAdapter {
	fn total_difficulty(&self) -> Difficulty {
		Difficulty::one()
	}
	fn transaction_received(&self, tx: Transaction) -> Result<(), Error> { Ok(()) }
	fn block_received(&self, b: Block) -> Result<(), Error> { Ok(()) }
	fn headers_received(&self, bh: Vec<BlockHeader>) -> Result<(), Error> { Ok(()) }
	fn locate_headers(&self, locator: Vec<Hash>) -> Option<Vec<BlockHeader>> {
		None
	}
	fn get_block(&self, h: Hash) -> Option<Block> {
		None
	}
	fn find_peer_addrs(&self, capab: Capabilities) -> Option<Vec<SocketAddr>> {
		None
	}
	fn peer_addrs_received(&self, peer_addrs: Vec<SocketAddr>) -> Result<(), Error> { Ok(())}
	fn peer_connected(&self, pi: &PeerInfo) {}
}

// A no-op network adapter that always rejects a payload
pub struct RejectingAdapter {}
impl NetAdapter for RejectingAdapter {
	fn total_difficulty(&self) -> Difficulty {
		Difficulty::one()
	}
	fn transaction_received(&self, tx: Transaction) -> Result<(), Error> { Err(Error::Invalid) }
	fn block_received(&self, b: Block) -> Result<(), Error> { Err(Error::Invalid) }
	fn headers_received(&self, bh: Vec<BlockHeader>) -> Result<(), Error> { Err(Error::Invalid) }
	fn locate_headers(&self, locator: Vec<Hash>) -> Option<Vec<BlockHeader>> {
		None
	}
	fn get_block(&self, h: Hash) -> Option<Block> {
		None
	}
	fn find_peer_addrs(&self, capab: Capabilities) -> Option<Vec<SocketAddr>> {
		None
	}
	fn peer_addrs_received(&self, peer_addrs: Vec<SocketAddr>) -> Result<(), Error> { Err(Error::Invalid)}
	fn peer_connected(&self, pi: &PeerInfo) {}
}
