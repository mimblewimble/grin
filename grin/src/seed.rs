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

//! Mining plugin manager, using the cuckoo-miner crate to provide
//! a mining worker implementation
//!

use std::net::SocketAddr;
use std::str::{self, FromStr};
use std::sync::Arc;
use std::time;

use cpupool;
use futures::{self, future, Future, Stream};
use futures::sync::mpsc;
use hyper;
use tokio_core::reactor;
use tokio_timer::Timer;

use p2p;
use util::LOGGER;

const PEER_MAX_COUNT: u32 = 25;
const PEER_PREFERRED_COUNT: u32 = 8;
const SEEDS_URL: &'static str = "http://grin-tech.org/seeds.txt";

pub struct Seeder {
	p2p: Arc<p2p::Server>,

	capabilities: p2p::Capabilities,
}

impl Seeder {
	pub fn new(
		capabilities: p2p::Capabilities,
		p2p: Arc<p2p::Server>,
	) -> Seeder {
		Seeder {
			p2p: p2p,
			capabilities: capabilities,
		}
	}

	pub fn connect_and_monitor(
		&self,
		h: reactor::Handle,
		seed_list: Box<Future<Item = Vec<SocketAddr>, Error = String>>,
	) {
		// open a channel with a listener that connects every peer address sent below
		// max peer count
		let (tx, rx) = futures::sync::mpsc::unbounded();
		h.spawn(self.listen_for_addrs(h.clone(), rx));

		// check seeds and start monitoring connections
		let seeder = self.connect_to_seeds(tx.clone(), seed_list)
			.join(self.monitor_peers(tx.clone()));

		h.spawn(seeder.map(|_| ()).map_err(|e| {
			error!(LOGGER, "Seeding or peer monitoring error: {}", e);
			()
		}));
	}

	fn monitor_peers(
		&self,
		tx: mpsc::UnboundedSender<SocketAddr>,
	) -> Box<Future<Item = (), Error = String>> {
		let p2p_server = self.p2p.clone();
		let capabilities = self.capabilities.clone();

		// now spawn a new future to regularly check if we need to acquire more peers
		// and if so, gets them from db
		let mon_loop = Timer::default()
			.interval(time::Duration::from_secs(30))
			.for_each(move |_| {
				let total_count = p2p_server.all_peers().len();
				debug!(
					LOGGER,
					"monitor_peers: {} most_work_peers, {} connected, {} total known",
					p2p_server.most_work_peers().len(),
					p2p_server.connected_peers().len(),
					total_count,
				);

				let mut healthy_count = 0;
				let mut banned_count = 0;
				let mut defunct_count = 0;
				for x in p2p_server.all_peers() {
					if x.flags == p2p::State::Healthy { healthy_count += 1 }
					else if x.flags == p2p::State::Banned { banned_count += 1 }
					else if x.flags == p2p::State::Defunct { defunct_count += 1 };
				}

				debug!(
					LOGGER,
					"monitor_peers: all {} = {} healthy + {} banned + {} defunct",
					total_count,
					healthy_count,
					banned_count,
					defunct_count,
				);

				// maintenance step first, clean up p2p server peers
				{
					p2p_server.clean_peers(PEER_PREFERRED_COUNT as usize);
				}

				// not enough peers, getting more from db
				if p2p_server.peer_count() < PEER_PREFERRED_COUNT {
					// loop over connected peers
					// ask them for their list of peers
					for p in p2p_server.connected_peers() {
						if let Ok(p) = p.try_read() {
							debug!(
								LOGGER,
								"monitor_peers: asking {} for more peers",
								p.info.addr,
							);
							let _ = p.send_peer_request(capabilities);
						} else {
							warn!(
								LOGGER,
								"monitor_peers: failed to get read lock on peer",
							);
						}
					}

					// find some peers from our db
					// and queue them up for a connection attempt
					let peers = p2p_server.find_peers(
						p2p::State::Healthy,
						p2p::UNKNOWN,
						100,
					);
					for p in peers {
						debug!(
							LOGGER,
							"monitor_peers: queue to soon try {}",
							p.addr,
						);
						tx.unbounded_send(p.addr).unwrap();
					}
				}
				Ok(())
			})
			.map_err(|e| e.to_string());

		Box::new(mon_loop)
	}

	// Check if we have any pre-existing peer in db. If so, start with those,
	// otherwise use the seeds provided.
	fn connect_to_seeds(
		&self,
		tx: mpsc::UnboundedSender<SocketAddr>,
		seed_list: Box<Future<Item = Vec<SocketAddr>, Error = String>>,
	) -> Box<Future<Item = (), Error = String>> {

		// a thread pool is required so we don't block the event loop with a
		// db query
		let thread_pool = cpupool::CpuPool::new(1);
		let p2p_server = self.p2p.clone();
		let seeder = thread_pool
			.spawn_fn(move || {
				// check if we have some peers in db
				let peers = p2p_server.find_peers(
					p2p::State::Healthy,
					p2p::FULL_HIST,
					100,
				);
				Ok(peers)
			})
			.and_then(|peers| {
				// if so, get their addresses, otherwise use our seeds
				if peers.len() > 3 {
					Box::new(future::ok(peers.iter().map(|p| p.addr).collect::<Vec<_>>()))
				} else {
					seed_list
				}
			})
			.and_then(move |peer_addrs| {
				if peer_addrs.len() == 0 {
					warn!(LOGGER, "No seeds were retrieved.");
				}

				// connect to this first set of addresses
				for addr in peer_addrs {
					tx.unbounded_send(addr).unwrap();
				}

				Ok(())
			});
		Box::new(seeder)
	}

	/// Builds a future to continuously listen on a channel receiver for new
	/// addresses to and initiate a connection if the max peer count isn't
	/// exceeded. A request for more peers is also automatically sent after
	/// connection.
	fn listen_for_addrs(
		&self,
		h: reactor::Handle,
		rx: mpsc::UnboundedReceiver<SocketAddr>,
	) -> Box<Future<Item = (), Error = ()>> {
		let capab = self.capabilities;
		let p2p_server = self.p2p.clone();

		let listener = rx.for_each(move |peer_addr| {
			debug!(LOGGER, "New peer address to connect to: {}.", peer_addr);
			let inner_h = h.clone();
			if p2p_server.peer_count() < PEER_MAX_COUNT {
				h.spawn(
					connect_and_req(
						capab,
						p2p_server.clone(),
						inner_h,
						peer_addr,
					)
				)
			};
			Box::new(future::ok(()))
		});
		Box::new(listener)
	}
}

/// Extract the list of seeds from a pre-defined text file available through
/// http. Easy method until we have a set of DNS names we can rely on.
pub fn web_seeds(h: reactor::Handle) -> Box<Future<Item = Vec<SocketAddr>, Error = String>> {
	let url = hyper::Uri::from_str(&SEEDS_URL).unwrap();
	let seeds = future::ok(()).and_then(move |_| {
		let client = hyper::Client::new(&h);
		debug!(LOGGER, "Retrieving seed nodes from {}", &SEEDS_URL);

		// http get, filtering out non 200 results
		client
			.get(url)
			.map_err(|e| e.to_string())
			.and_then(|res| {
				if res.status() != hyper::Ok {
					return Err(format!("Gist request failed: {}", res.status()));
				}
				Ok(res)
			})
			.and_then(|res| {
				// collect all chunks and split around whitespace to get a list of SocketAddr
				res.body()
					.collect()
					.map_err(|e| e.to_string())
					.and_then(|chunks| {
						let res = chunks.iter().fold("".to_string(), |acc, ref chunk| {
							acc + str::from_utf8(&chunk[..]).unwrap()
						});
						let addrs = res.split_whitespace()
							.map(|s| s.parse().unwrap())
							.collect::<Vec<_>>();
						debug!(LOGGER, "Retrieved seed addresses: {:?}", addrs);
						Ok(addrs)
					})
			})
	});
	Box::new(seeds)
}

/// Convenience function when the seed list is immediately known. Mostly used
/// for tests.
pub fn predefined_seeds(
	addrs_str: Vec<String>,
) -> Box<Future<Item = Vec<SocketAddr>, Error = String>> {
	let seeds = future::ok(()).and_then(move |_| {
		Ok(
			addrs_str
				.iter()
				.map(|s| s.parse().unwrap())
				.collect::<Vec<_>>(),
		)
	});
	Box::new(seeds)
}

fn connect_and_req(
	capab: p2p::Capabilities,
	p2p: Arc<p2p::Server>,
	h: reactor::Handle,
	addr: SocketAddr,
) -> Box<Future<Item = (), Error = ()>> {
	let connect_peer = p2p.connect_peer(addr, h);
	let p2p_server = p2p.clone();
	let fut = connect_peer.then(move |p| {
		match p {
			Ok(Some(p)) => {
				debug!(LOGGER, "connect_and_req: ok. attempting send_peer_request");
				if let Ok(p) = p.try_read() {
					let _ = p.send_peer_request(capab);
				}
			},
			Ok(None) => {
				debug!(LOGGER, "connect_and_req: ok but none inner (what does this mean?), {}", addr);
			},
			Err(e) => {
				debug!(LOGGER, "connect_and_req: {} is Defunct; {:?}", addr, e);
				let _ = p2p_server.update_state(addr, p2p::State::Defunct);
			},
		}
		Ok(())
	});
	Box::new(fut)
}
