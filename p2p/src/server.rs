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
use std::collections::HashMap;
use std::net::{SocketAddr, Shutdown};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use futures;
use futures::{Future, Stream};
use futures::future::{self, IntoFuture};
use rand::{thread_rng, Rng};
use tokio_core::net::{TcpListener, TcpStream};
use tokio_core::reactor;
use tokio_timer::Timer;

use core::core;
use core::core::hash::Hash;
use core::core::target::Difficulty;
use handshake::Handshake;
use peer::Peer;
use store::{PeerStore, PeerData, State};
use types::*;
use util::LOGGER;

/// A no-op network adapter used for testing.
pub struct DummyAdapter {}
impl NetAdapter for DummyAdapter {
	fn total_difficulty(&self) -> Difficulty {
		Difficulty::one()
	}
	fn transaction_received(&self, _: core::Transaction) {}
	fn block_received(&self, _: core::Block, _: SocketAddr) {}
	fn headers_received(&self, _: Vec<core::BlockHeader>, _:SocketAddr) {}
	fn locate_headers(&self, _: Vec<Hash>) -> Vec<core::BlockHeader> {
		vec![]
	}
	fn get_block(&self, _: Hash) -> Option<core::Block> {
		None
	}
	fn find_peer_addrs(&self, _: Capabilities) -> Vec<SocketAddr> {
		vec![]
	}
	fn peer_addrs_received(&self, _: Vec<SocketAddr>) {}
	fn peer_connected(&self, _: &PeerInfo) {}
	fn peer_difficulty(&self, _: SocketAddr, _: Difficulty) {}
}

/// P2P server implementation, handling bootstrapping to find and connect to
/// peers, receiving connections from other peers and keep track of all of them.
pub struct Server {
	config: P2PConfig,
	capabilities: Capabilities,
	store: Arc<PeerStore>,
	peers: Arc<RwLock<HashMap<SocketAddr, Arc<RwLock<Peer>>>>>,
	handshake: Arc<Handshake>,
	adapter: Arc<NetAdapter>,
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
		adapter: Arc<NetAdapter>,
		genesis: Hash,
	) -> Result<Server, Error> {
		Ok(Server {
			config: config,
			capabilities: capab,
			store: Arc::new(PeerStore::new(db_root)?),
			peers: Arc::new(RwLock::new(HashMap::new())),
			handshake: Arc::new(Handshake::new(genesis)),
			adapter: adapter,
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
		let adapter = self.adapter.clone();
		let capab = self.capabilities.clone();
		let store = self.store.clone();

		// main peer acceptance future handling handshake
		let hp = h.clone();
		let peers_listen = socket.incoming().map_err(From::from).map(move |(conn, _)| {

			// aaaand.. reclone for the internal closures
			let adapter = adapter.clone();
			let store = store.clone();
			let peers = peers.clone();
			let handshake = handshake.clone();
			let hp = hp.clone();

			future::ok(conn).and_then(move |conn| {
				// Refuse banned peers connection
				if let Ok(peer_addr) = conn.peer_addr() {
					if let Ok(peer_data) = store.get_peer(peer_addr) {
						if peer_data.flags == State::Banned {
							debug!(LOGGER, "Peer {} banned, refusing connection.", peer_addr);
							if let Err(e) = conn.shutdown(Shutdown::Both) {
								debug!(LOGGER, "Error shutting down conn: {:?}", e);
							}
							return Err(Error::Banned)
						}
					}
				}
				Ok(conn)
			}).and_then(move |conn| {

				let peers = peers.clone();
				let total_diff = adapter.total_difficulty();

				// accept the peer and add it to the server map
				let accept = Peer::accept(
					conn,
					capab,
					total_diff,
					&handshake.clone(),
					adapter.clone(),
				);
				let added = add_to_peers(peers, adapter.clone(), accept);

				// wire in a future to timeout the accept after 5 secs
				let timed_peer = with_timeout(Box::new(added), &hp);

				// run the main peer protocol
				timed_peer.and_then(move |(conn, peer)| {
					let peer = peer.read().unwrap();
					peer.run(conn)
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
		let adapter = self.adapter.clone();
		let peers_inner = self.peers.clone();
		let peers_timer = Timer::default()
			.interval(Duration::new(20, 0))
			.fold((), move |_, _| {
				let total_diff = adapter.total_difficulty();
				check_peers(peers_inner.clone(), total_diff);
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
		if let Some(p) = self.get_peer(&addr) {
			// if we're already connected to the addr, just return the peer
			debug!(LOGGER, "connect_peer: already connected {}", addr);
			return Box::new(future::ok(Some(p)));
		}

		debug!(LOGGER, "connect_peer: connecting to {}", addr);

		// cloneapalooza
		let peers = self.peers.clone();
		let handshake = self.handshake.clone();
		let adapter = self.adapter.clone();
		let capab = self.capabilities.clone();
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
				let peers = peers.clone();
				let total_diff = adapter.clone().total_difficulty();

				// connect to the peer and add it to the server map, wiring it a timeout for
				// the handshake
				let connect = Peer::connect(
					socket,
					capab,
					total_diff,
					self_addr,
					handshake.clone(),
					adapter.clone(),
				);
				let added = add_to_peers(peers, adapter, connect);
				with_timeout(Box::new(added), &h)
			})
			.and_then(move |(socket, peer)| {
				let peer_inner = peer.read().unwrap();
				h2.spawn(peer_inner.run(socket).map_err(|e| {
					error!(LOGGER, "Peer error: {:?}", e);
					()
				}));
				Ok(Some(peer.clone()))
			});
		Box::new(request)
	}

	/// Check if the server already knows this peer (is already connected).
	pub fn is_known(&self, addr: &SocketAddr) -> bool {
		self.get_peer(addr).is_some()
	}

	pub fn connected_peers(&self) -> Vec<Arc<RwLock<Peer>>> {
		self.peers.read().unwrap().values().map(|p| p.clone()).collect()
	}

	/// Get a peer we're connected to by address.
	pub fn get_peer(&self, addr: &SocketAddr) -> Option<Arc<RwLock<Peer>>> {
		self.peers.read().unwrap().get(addr).map(|p| p.clone())
	}

	/// Have the server iterate over its peer list and prune all peers we have
	/// lost connection to or have been deemed problematic.
	/// Also avoid connected peer count getting too high.
	pub fn clean_peers(&self, desired_count: usize) {
		let mut rm = vec![];

		// build a list of peers to be cleaned up
		for peer in self.connected_peers() {
			let peer_inner = peer.read().unwrap();
			if peer_inner.is_banned() {
				debug!(LOGGER, "cleaning {:?}, peer banned", peer_inner.info.addr);
				rm.push(peer.clone());
			} else if !peer_inner.is_connected() {
				debug!(LOGGER, "cleaning {:?}, not connected", peer_inner.info.addr);
				rm.push(peer.clone());
			}
		}

		// now clean up peer map based on the list to remove
		{
			let mut peers = self.peers.write().unwrap();
			for p in rm.clone() {
				let p = p.read().unwrap();
				peers.remove(&p.info.addr);
			}
		}

		// ensure we do not have too many connected peers
		// really fighting with the double layer of rwlocks here...
		let excess_count = {
			let peer_count = self.peer_count().clone() as usize;
			if peer_count > desired_count {
				peer_count - desired_count
			} else {
				0
			}
		};

		// map peers to addrs in a block to bound how long we keep the read lock for
		let addrs = {
			self.connected_peers().iter().map(|x| {
				let p = x.read().unwrap();
				p.info.addr.clone()
			}).collect::<Vec<_>>()
		};

		// now remove them taking a short-lived write lock each time
		// maybe better to take write lock once and remove them all?
		for x in addrs
			.iter()
			.take(excess_count) {
				let mut peers = self.peers.write().unwrap();
				peers.remove(x);
			}
	}

	/// Return vec of all peers that currently have the most worked branch,
	/// showing the highest total difficulty.
	pub fn most_work_peers(&self) -> Vec<Arc<RwLock<Peer>>> {
		let peers = self.connected_peers();
		if peers.len() == 0 {
			return vec![];
		}

		let max_total_difficulty = peers
			.iter()
			.map(|x| {
				match x.try_read() {
					Ok(peer) => peer.info.total_difficulty.clone(),
					Err(_) => Difficulty::zero(),
				}
			})
			.max()
			.unwrap();

		let mut max_peers = peers
			.iter()
			.filter(|x| {
				match x.try_read() {
					Ok(peer) => {
						peer.info.total_difficulty == max_total_difficulty
					},
					Err(_) => false,
				}
			})
			.cloned()
			.collect::<Vec<_>>();

		thread_rng().shuffle(&mut max_peers);
		max_peers
	}

	/// Returns single random peer with the most worked branch, showing the highest total
	/// difficulty.
	pub fn most_work_peer(&self) -> Option<Arc<RwLock<Peer>>> {
		match self.most_work_peers().first() {
			Some(x) => Some(x.clone()),
			None => None
		}
	}

	/// Returns a random connected peer.
	pub fn random_peer(&self) -> Option<Arc<RwLock<Peer>>> {
		let peers = self.connected_peers();
		Some(thread_rng().choose(&peers).unwrap().clone())
	}

	/// Broadcasts the provided block to all our peers. A peer implementation
	/// may drop the broadcast request if it knows the remote peer already has
	/// the block.
	pub fn broadcast_block(&self, b: &core::Block) {
		let peers = self.connected_peers();
		let mut count = 0;
		for p in peers {
			let p = p.read().unwrap();
			if p.is_connected() {
				if let Err(e) = p.send_block(b) {
					debug!(LOGGER, "Error sending block to peer: {:?}", e);
				} else {
					count += 1;
				}
			}
		}
		debug!(LOGGER, "Broadcasted block {} to {} peers.", b.header.height, count);
	}

	/// Broadcasts the provided transaction to all our peers. A peer
	/// implementation may drop the broadcast request if it knows the
	/// remote peer already has the transaction.
	pub fn broadcast_transaction(&self, tx: &core::Transaction) {
		let peers = self.connected_peers();
		for p in peers {
			let p = p.read().unwrap();
			if p.is_connected() {
				if let Err(e) = p.send_transaction(tx) {
					debug!(LOGGER, "Error sending block to peer: {:?}", e);
				}
			}
		}
	}

	/// Number of peers we're currently connected to.
	pub fn peer_count(&self) -> u32 {
		self.connected_peers().len() as u32
	}

	/// Bans a peer, disconnecting it if we're currently connected
	pub fn ban_peer(&self, peer_addr: &SocketAddr) {
		if let Err(e) = self.update_state(peer_addr.clone(), State::Banned) {
			error!(LOGGER, "Couldn't ban {}: {:?}", peer_addr, e);
		}

		if let Some(peer) = self.get_peer(peer_addr) {
			debug!(LOGGER, "Banning peer {}", peer_addr);
			// setting peer status will get it removed at the next clean_peer
			let peer = peer.write().unwrap();
			peer.set_banned();
			peer.stop();
		}
	}

	/// Stops the server. Disconnect from all peers at the same time.
	pub fn stop(self) {
		info!(LOGGER, "calling stop on server");
		let peers = self.connected_peers();
		for peer in peers {
			let peer = peer.read().unwrap();
			peer.stop();
		}
		self.stop.into_inner().unwrap().send(()).unwrap();
	}

	/// All peer information we have in storage
	pub fn all_peers(&self) -> Vec<PeerData> {
		self.store.all_peers()
	}

	/// Find peers in store (not necessarily connected) and return their data
	pub fn find_peers(&self, state: State, cap: Capabilities, count: usize) -> Vec<PeerData> {
		self.store.find_peers(state, cap, count)
	}

	/// Whether we've already seen a peer with the provided address
	pub fn exists_peer(&self, peer_addr: SocketAddr) -> Result<bool, Error> {
		self.store.exists_peer(peer_addr).map_err(From::from)
	}

	/// Saves updated information about a peer
	pub fn save_peer(&self, p: &PeerData) -> Result<(), Error> {
		self.store.save_peer(p).map_err(From::from)
	}

	/// Updates the state of a peer in store
	pub fn update_state(&self, peer_addr: SocketAddr, new_state: State) -> Result<(), Error> {
		self.store.update_state(peer_addr, new_state).map_err(From::from)
	}
}

// Adds the peer built by the provided future in the peers map
fn add_to_peers<A>(
	peers: Arc<RwLock<HashMap<SocketAddr, Arc<RwLock<Peer>>>>>,
	adapter: Arc<NetAdapter>,
	peer_fut: A,
) -> Box<Future<Item = Result<(TcpStream, Arc<RwLock<Peer>>), ()>, Error = Error>>
where
	A: IntoFuture<Item = (TcpStream, Peer), Error = Error> + 'static,
{
	let peer_add = peer_fut.into_future().map(move |(conn, peer)| {
		adapter.peer_connected(&peer.info);
		let addr = peer.info.addr.clone();
		let apeer = Arc::new(RwLock::new(peer));
		{
			let mut peers = peers.write().unwrap();
			peers.insert(addr, apeer.clone());
		}
		Ok((conn, apeer))
	});
	Box::new(peer_add)
}

// Ping all our connected peers. Always automatically expects a pong back or
// disconnects. This acts as a liveness test.
fn check_peers(
	peers: Arc<RwLock<HashMap<SocketAddr, Arc<RwLock<Peer>>>>>,
	total_difficulty: Difficulty,
) {
	let peers_map = peers.read().unwrap();
	for p in peers_map.values() {
		let p = p.read().unwrap();
		if p.is_connected() {
			let _ = p.send_ping(total_difficulty.clone());
		}
	}
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
