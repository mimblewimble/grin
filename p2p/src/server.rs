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

//! Grin server implementation, accepts incoming connections and connects to
//! other peers in the network.

use std::cell::RefCell;
use std::net::SocketAddr;
use std::ops::Deref;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use futures;
use futures::{Future, Stream};
use tokio_core::net::{TcpListener, TcpStream};
use tokio_core::reactor;

use core::core;
use core::ser::Error;
use handshake::Handshake;
use peer::Peer;
use types::*;

pub struct DummyAdapter {}
impl NetAdapter for DummyAdapter {
	fn transaction_received(&self, tx: core::Transaction) {}
	fn block_received(&self, b: core::Block) {}
}

/// P2P server implementation, handling bootstrapping to find and connect to
/// peers, receiving connections from other peers and keep track of all of them.
pub struct Server {
	config: P2PConfig,
	peers: Arc<RwLock<Vec<Arc<Peer>>>>,
	stop: RefCell<Option<futures::sync::oneshot::Sender<()>>>,
}

unsafe impl Sync for Server {}
unsafe impl Send for Server {}

// TODO TLS
impl Server {
	/// Creates a new idle p2p server with no peers
	pub fn new(config: P2PConfig) -> Server {
		Server {
			config: config,
			peers: Arc::new(RwLock::new(Vec::new())),
			stop: RefCell::new(None),
		}
	}

	/// Starts the p2p server. Opens a TCP port to allow incoming
	/// connections and starts the bootstrapping process to find peers.
	pub fn start(&self, h: reactor::Handle) -> Box<Future<Item = (), Error = Error>> {
		let addr = SocketAddr::new(self.config.host, self.config.port);
		let socket = TcpListener::bind(&addr, &h.clone()).unwrap();
		warn!("P2P server started on {}", addr);

		let hs = Arc::new(Handshake::new());
		let peers = self.peers.clone();

		// main peer acceptance future handling handshake
		let hp = h.clone();
		let peers = socket.incoming().map_err(|e| Error::IOErr(e)).map(move |(conn, addr)| {
			let peers = peers.clone();
			// accept the peer and add it to the server map
			let peer_accept = Peer::accept(conn, &hs.clone()).map(move |(conn, peer)| {
				let apeer = Arc::new(peer);
				let mut peers = peers.write().unwrap();
				peers.push(apeer.clone());
				Ok((conn, apeer))
			});

			// wire in a future to timeout the accept after 5 secs
			let timeout = reactor::Timeout::new(Duration::new(5, 0), &hp).unwrap();
			let timed_peer = peer_accept.select(timeout.map(Err).map_err(|e| Error::IOErr(e)))
				.then(|res| {
					match res {
						Ok((Ok((conn, p)), _timeout)) => Ok((conn, p)),
						Ok((_, _accept)) => Err(Error::TooLargeReadErr),
						Err((e, _other)) => Err(e),
					}
				});

			// run the main peer protocol
			timed_peer.and_then(|(conn, peer)| peer.clone().run(conn, &DummyAdapter {}))
		});

		// spawn each peer future to its own task
		let hs = h.clone();
		let server = peers.for_each(move |peer| {
			hs.spawn(peer.then(|res| {
				match res {
					Err(e) => info!("Client error: {}", e),
					_ => {}
				}
				futures::finished(())
			}));
			Ok(())
		});

		// setup the stopping oneshot on the server and join it with the peer future
		let (stop, stop_rx) = futures::sync::oneshot::channel();
		{
			let mut stop_mut = self.stop.borrow_mut();
			*stop_mut = Some(stop);
		}
		Box::new(server.select(stop_rx.map_err(|_| Error::CorruptedData)).then(|res| {
			match res {
				Ok((_, _)) => Ok(()),
				Err((e, _)) => Err(e),
			}
		}))
	}

	pub fn connect_peer(&self,
	                    addr: SocketAddr,
	                    h: reactor::Handle)
	                    -> Box<Future<Item = (), Error = Error>> {
		let socket = TcpStream::connect(&addr, &h).map_err(|e| Error::IOErr(e));
		let peers = self.peers.clone();
		let request = socket.and_then(move |socket| {
				let peers = peers.clone();
				Peer::connect(socket, &Handshake::new()).map(move |(conn, peer)| {
					let apeer = Arc::new(peer);
					let mut peers = peers.write().unwrap();
					peers.push(apeer.clone());
					(conn, apeer)
				})
			})
			.and_then(|(socket, peer)| peer.run(socket, &DummyAdapter {}));
		Box::new(request)
	}

	/// Stops the server. Disconnect from all peers at the same time.
	pub fn stop(self) {
		let peers = self.peers.write().unwrap();
		for p in peers.deref() {
			p.stop();
		}
		self.stop.into_inner().unwrap().complete(());
	}
}
