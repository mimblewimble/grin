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
use std::net::{SocketAddr, Shutdown};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use futures;
use futures::{Future, Stream};
use futures::future::{self, IntoFuture};
use futures_cpupool::CpuPool;
use tokio_core::net::{TcpListener, TcpStream};
use tokio_core::reactor;
use tokio_timer::Timer;

use core::core;
use core::core::hash::Hash;
use core::core::target::Difficulty;
use handshake::Handshake;
use peer::Peer;
use peers::Peers;
use store::PeerStore;
use types::*;
use util::LOGGER;

/// A no-op network adapter used for testing.
pub struct DummyAdapter {}

impl ChainAdapter for DummyAdapter {
	fn total_difficulty(&self) -> Difficulty {
		Difficulty::one()
	}
	fn total_height(&self) -> u64 {
		0
	}
	fn transaction_received(&self, _tx: core::Transaction) {}
	fn block_received(&self, _b: core::Block, _addr: SocketAddr) -> bool { true }
	fn compact_block_received(&self, _cb: core::CompactBlock, _addr: SocketAddr) -> bool { true }
	fn header_received(&self, _bh: core::BlockHeader, _addr: SocketAddr) -> bool { true }
	fn headers_received(&self, _bh: Vec<core::BlockHeader>, _addr:SocketAddr) {}
	fn locate_headers(&self, _loc: Vec<Hash>) -> Vec<core::BlockHeader> {
		vec![]
	}
	fn get_block(&self, _: Hash) -> Option<core::Block> {
		None
	}
}

impl NetAdapter for DummyAdapter {
	fn find_peer_addrs(&self, _: Capabilities) -> Vec<SocketAddr> {
		vec![]
	}
	fn peer_addrs_received(&self, _: Vec<SocketAddr>) {}
	fn peer_difficulty(&self, _: SocketAddr, _: Difficulty, _:u64) {}
}

/// P2P server implementation, handling bootstrapping to find and connect to
/// peers, receiving connections from other peers and keep track of all of them.
pub struct Server {
	config: P2PConfig,
	capabilities: Capabilities,
	handshake: Arc<Handshake>,
	pub peers: Peers,
	pool: CpuPool,
	stop: RefCell<Option<futures::sync::oneshot::Sender<()>>>,
}

unsafe impl Sync for Server {}
unsafe impl Send for Server {}

// TODO TLS
impl Server {
	/// Creates a new idle p2p server with no peers
	pub fn new(
		db_root: String,
		capab: Capabilities,
		config: P2PConfig,
		adapter: Arc<ChainAdapter>,
		genesis: Hash,
		pool: CpuPool,
	) -> Result<Server, Error> {
		Ok(Server {
			config: config.clone(),
			capabilities: capab,
			handshake: Arc::new(Handshake::new(genesis, config.clone())),
			peers: Peers::new(PeerStore::new(db_root)?, adapter, config.clone()),
			pool: pool,
			stop: RefCell::new(None),
		})
	}

	/// Starts the p2p server. Opens a TCP port to allow incoming
	/// connections and starts the bootstrapping process to find peers.
	pub fn start(&self, h: reactor::Handle) -> Box<Future<Item = (), Error = Error>> {
		let addr = SocketAddr::new(self.config.host, self.config.port);
		let socket = TcpListener::bind(&addr, &h.clone()).unwrap();
		warn!(LOGGER, "P2P server started on {}", addr);

		let handshake = self.handshake.clone();
		let peers = self.peers.clone();
		let capab = self.capabilities.clone();
		let pool = self.pool.clone();

		// main peer acceptance future handling handshake
		let hp = h.clone();
		let peers_listen = socket.incoming().map_err(From::from).map(move |(conn, _)| {

			// aaaand.. reclone for the internal closures
			let peers = peers.clone();
			let peers2 = peers.clone();
			let handshake = handshake.clone();
			let hp = hp.clone();
			let pool = pool.clone();

			future::ok(conn).and_then(move |conn| {
				// Refuse connection from banned peers
				if let Ok(peer_addr) = conn.peer_addr() {
					if peers.is_banned(peer_addr) {
						debug!(LOGGER, "Peer {} banned, refusing connection.", peer_addr);
						if let Err(e) = conn.shutdown(Shutdown::Both) {
							debug!(LOGGER, "Error shutting down conn: {:?}", e);
						}
						return Err(Error::Banned)
					}
				}
				Ok(conn)
			}).and_then(move |conn| {
				let total_diff = peers2.total_difficulty();

				// accept the peer and add it to the server map
				let accept = Peer::accept(
					conn,
					capab,
					total_diff,
					&handshake.clone(),
					Arc::new(peers2.clone()),
				);
				let added = add_to_peers(peers2, accept);

				// wire in a future to timeout the accept after 5 secs
				let timed_peer = with_timeout(Box::new(added), &hp);

				// run the main peer protocol
				timed_peer.and_then(move |(conn, peer)| {
					let peer = peer.read().unwrap();
					peer.run(conn, pool)
				})
			})
		});

		// spawn each peer future to its own task
		let hs = h.clone();
		let server = peers_listen.for_each(move |peer| {
			hs.spawn(peer.then(|res| {
				match res {
					Err(e) => info!(LOGGER, "Client error: {:?}", e),
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


		// timer to regularly check on our peers by pinging them
		let peers_inner = self.peers.clone();
		let peers_timer = Timer::default()
			.interval(Duration::new(20, 0))
			.fold((), move |_, _| {
				let total_diff = peers_inner.total_difficulty();
				let total_height = peers_inner.total_height();
				peers_inner.check_all(total_diff, total_height);
				Ok(())
			});

		Box::new(
			server
				.select(stop_rx.map_err(|_| Error::ConnectionClose))
				.then(|res| match res {
					Ok((_, _)) => Ok(()),
					Err((e, _)) => Err(e),
				})
				.select(peers_timer.map_err(|_| Error::Timeout))
				.then(|res| match res {
					Ok((_, _)) => Ok(()),
					Err((e, _)) => Err(e),
				}),
		)
	}

	/// Asks the server to connect to a new peer.
	pub fn connect_peer(
		&self,
		addr: SocketAddr,
		h: reactor::Handle,
	) -> Box<Future<Item = Option<Arc<RwLock<Peer>>>, Error = Error>> {

		if Peer::is_denied(self.config.clone(), addr) {
			debug!(LOGGER, "Peer {} denied, not connecting.", addr);
			return Box::new(future::err(Error::ConnectionClose));
		}

		if let Some(p) = self.peers.get_connected_peer(&addr) {
			// if we're already connected to the addr, just return the peer
			debug!(LOGGER, "connect_peer: already connected {}", addr);
			return Box::new(future::ok(Some(p)));
		}

		debug!(LOGGER, "connect_peer: connecting to {}", addr);

		// cloneapalooza
		let peers = self.peers.clone();
		let handshake = self.handshake.clone();
		let capab = self.capabilities.clone();
		let pool = self.pool.clone();

		let self_addr = SocketAddr::new(self.config.host, self.config.port);

		let timer = Timer::default();
		let socket_connect = timer.timeout(
			TcpStream::connect(&addr, &h),
			Duration::from_secs(5),
		).map_err(|e| {
			debug!(LOGGER, "connect_peer: socket connect error - {:?}", e);
			Error::Connection(e)
		});

		let h2 = h.clone();
		let request = socket_connect
			.and_then(move |socket| {
				let total_diff = peers.total_difficulty();

				// connect to the peer and add it to the server map, wiring it a timeout for
				// the handshake
				let connect = Peer::connect(
					socket,
					capab,
					total_diff,
					self_addr,
					handshake.clone(),
					Arc::new(peers.clone()),
				);
				let added = add_to_peers(peers, connect);
				with_timeout(Box::new(added), &h)
			})
			.and_then(move |(socket, peer)| {
				let peer_inner = peer.read().unwrap();
				h2.spawn(peer_inner.run(socket, pool).map_err(|e| {
					error!(LOGGER, "Peer error: {:?}", e);
					()
				}));
				Ok(Some(peer.clone()))
			});
		Box::new(request)
	}

	/// Stops the server. Disconnect from all peers at the same time.
	pub fn stop(self) {
		info!(LOGGER, "calling stop on server");
		self.peers.stop();
		self.stop.into_inner().unwrap().send(()).unwrap();
	}
}

// Adds the peer built by the provided future in the peers map
fn add_to_peers<A>(peers: Peers, peer_fut: A)
	-> Box<Future<Item = Result<(TcpStream, Arc<RwLock<Peer>>), ()>, Error = Error>>
	where A: IntoFuture<Item = (TcpStream, Peer), Error = Error> + 'static {

	let peer_add = peer_fut.into_future().map(move |(conn, peer)| {
		let apeer = peers.add_connected(peer);
		Ok((conn, apeer))
	});
	Box::new(peer_add)
}

// Adds a timeout to a future
fn with_timeout<T: 'static>(
	fut: Box<Future<Item = Result<T, ()>, Error = Error>>,
	h: &reactor::Handle,
) -> Box<Future<Item = T, Error = Error>> {
	let timeout = reactor::Timeout::new(Duration::from_secs(5), h).unwrap();
	let timed = fut.select(timeout.map(Err).from_err())
		.then(|res| match res {
			Ok((Ok(inner), _timeout)) => {
				Ok(inner)
			},
			Ok((Err(inner), _accept)) => {
				debug!(LOGGER, "with_timeout: ok, timeout. nested={:?}", inner);
				Err(Error::Timeout)
			},
			Err((e, _other)) => {
				debug!(LOGGER, "with_timeout: err. {:?}", e);
				Err(e)
			},
		});
	Box::new(timed)
}
