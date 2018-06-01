// Copyright 2018 The Grin Developers
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

use std::io::Read;
use std::net::SocketAddr;
use std::net::ToSocketAddrs;
use std::str;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;
use time::{self, now_utc};

use hyper;

use p2p;
use pool::DandelionConfig;
use util::LOGGER;

const SEEDS_URL: &'static str = "http://grin-tech.org/seeds.txt";
// DNS Seeds with contact email associated
const DNS_SEEDS: &'static [&'static str] = &[
	"seed.grin-tech.org",      // igno.peverell@protonmail.com
	"seed.grin.lesceller.com", // q.lesceller@gmail.com
	"grin-seed.owncrypto.de",  // roll@yourowncryp.to
];

pub fn connect_and_monitor(
	p2p_server: Arc<p2p::Server>,
	capabilities: p2p::Capabilities,
	dandelion_config: DandelionConfig,
	seed_list: Box<Fn() -> Vec<SocketAddr> + Send>,
	stop: Arc<AtomicBool>,
) {
	let _ = thread::Builder::new()
		.name("seed".to_string())
		.spawn(move || {
			let peers = p2p_server.peers.clone();

			// open a channel with a listener that connects every peer address sent below
			// max peer count
			let (tx, rx) = mpsc::channel();

			// check seeds first
			connect_to_seeds(peers.clone(), tx.clone(), seed_list);

			let mut prev = time::now_utc() - time::Duration::seconds(60);
			loop {
				let current_time = time::now_utc();

				if current_time - prev > time::Duration::seconds(20) {
					// try to connect to any address sent to the channel
					listen_for_addrs(peers.clone(), p2p_server.clone(), capabilities, &rx);

					// monitor additional peers if we need to add more
					monitor_peers(
						peers.clone(),
						p2p_server.config.clone(),
						capabilities,
						tx.clone(),
					);

					update_dandelion_relay(peers.clone(), dandelion_config.clone());

					prev = current_time;
				}

				thread::sleep(Duration::from_secs(1));

				if stop.load(Ordering::Relaxed) {
					break;
				}
			}
		});
}

fn monitor_peers(
	peers: Arc<p2p::Peers>,
	config: p2p::P2PConfig,
	capabilities: p2p::Capabilities,
	tx: mpsc::Sender<SocketAddr>,
) {
	// regularly check if we need to acquire more peers  and if so, gets
	// them from db
	let total_count = peers.all_peers().len();
	let mut healthy_count = 0;
	let mut banned_count = 0;
	let mut defunct_count = 0;
	for x in peers.all_peers() {
		match x.flags {
			p2p::State::Banned => {
				let interval = now_utc().to_timespec().sec - x.last_banned;
				// Unban peer
				if interval >= config.ban_window() {
					peers.unban_peer(&x.addr);
					debug!(
						LOGGER,
						"monitor_peers: unbanned {} after {} seconds", x.addr, interval
					);
				} else {
					banned_count += 1;
				}
			}
			p2p::State::Healthy => healthy_count += 1,
			p2p::State::Defunct => defunct_count += 1,
		}
	}

	debug!(
		LOGGER,
		"monitor_peers: {} connected ({} most_work). \
		 all {} = {} healthy + {} banned + {} defunct",
		peers.connected_peers().len(),
		peers.most_work_peers().len(),
		total_count,
		healthy_count,
		banned_count,
		defunct_count,
	);

	// maintenance step first, clean up p2p server peers
	peers.clean_peers(config.peer_max_count() as usize);

	// not enough peers, getting more from db
	if peers.peer_count() >= config.peer_min_preferred_count() {
		return;
	}

	// loop over connected peers
	// ask them for their list of peers
	for p in peers.connected_peers() {
		if let Ok(p) = p.try_read() {
			debug!(LOGGER, "monitor_peers: ask {} for more peers", p.info.addr);
			let _ = p.send_peer_request(capabilities);
		} else {
			warn!(LOGGER, "monitor_peers: failed to get read lock on peer");
		}
	}

	// find some peers from our db
	// and queue them up for a connection attempt
	let peers = peers.find_peers(p2p::State::Healthy, p2p::Capabilities::UNKNOWN, 100);
	for p in peers {
		debug!(LOGGER, "monitor_peers: queue to soon try {}", p.addr);
		tx.send(p.addr).unwrap();
	}
}

fn update_dandelion_relay(peers: Arc<p2p::Peers>, dandelion_config: DandelionConfig) {
	// Dandelion Relay Updater
	let dandelion_relay = peers.get_dandelion_relay();
	if dandelion_relay.is_empty() {
		debug!(LOGGER, "monitor_peers: no dandelion relay updating");
		peers.update_dandelion_relay();
	} else {
		for last_added in dandelion_relay.keys() {
			let dandelion_interval = now_utc().to_timespec().sec - last_added;
			if dandelion_interval >= dandelion_config.relay_secs.unwrap() as i64 {
				debug!(LOGGER, "monitor_peers: updating expired dandelion relay");
				peers.update_dandelion_relay();
			}
		}
	}
}

// Check if we have any pre-existing peer in db. If so, start with those,
// otherwise use the seeds provided.
fn connect_to_seeds(
	peers: Arc<p2p::Peers>,
	tx: mpsc::Sender<SocketAddr>,
	seed_list: Box<Fn() -> Vec<SocketAddr>>,
) {
	// check if we have some peers in db
	let peers = peers.find_peers(p2p::State::Healthy, p2p::Capabilities::FULL_HIST, 100);

	// if so, get their addresses, otherwise use our seeds
	let peer_addrs = if peers.len() > 3 {
		peers.iter().map(|p| p.addr).collect::<Vec<_>>()
	} else {
		seed_list()
	};

	if peer_addrs.len() == 0 {
		warn!(LOGGER, "No seeds were retrieved.");
	}

	// connect to this first set of addresses
	for addr in peer_addrs {
		tx.send(addr).unwrap();
	}
}

/// Regularly poll a channel receiver for new addresses and initiate a
/// connection if the max peer count isn't exceeded. A request for more
/// peers is also automatically sent after connection.
fn listen_for_addrs(
	peers: Arc<p2p::Peers>,
	p2p: Arc<p2p::Server>,
	capab: p2p::Capabilities,
	rx: &mpsc::Receiver<SocketAddr>,
) {
	let pc = peers.peer_count();
	for addr in rx.try_iter() {
		if pc < p2p.config.peer_max_count() {
			let peers_c = peers.clone();
			let p2p_c = p2p.clone();
			let _ = thread::Builder::new()
				.name("peer_connect".to_string())
				.spawn(move || {
					let connect_peer = p2p_c.connect(&addr);
					match connect_peer {
						Ok(p) => {
							trace!(LOGGER, "connect_and_req: ok. attempting send_peer_request");
							if let Ok(p) = p.try_read() {
								let _ = p.send_peer_request(capab);
							}
						}
						Err(e) => {
							debug!(LOGGER, "connect_and_req: {} is Defunct; {:?}", addr, e);
							let _ = peers_c.update_state(addr, p2p::State::Defunct);
						}
					}
				});
		}
	}
}

pub fn dns_seeds() -> Box<Fn() -> Vec<SocketAddr> + Send> {
	Box::new(|| {
		let mut addresses: Vec<SocketAddr> = vec![];
		for dns_seed in DNS_SEEDS {
			let temp_addresses = addresses.clone();
			debug!(LOGGER, "Retrieving seed nodes from dns {}", dns_seed);
			match (dns_seed.to_owned(), 0).to_socket_addrs() {
				Ok(addrs) => addresses.append(
					&mut (addrs
						.map(|mut addr| {
							addr.set_port(13414);
							addr
						})
						.filter(|addr| !temp_addresses.contains(addr))
						.collect()),
				),
				Err(e) => debug!(
					LOGGER,
					"Failed to resolve seed {:?} got error {:?}", dns_seed, e
				),
			}
		}
		addresses
	})
}

/// Extract the list of seeds from a pre-defined text file available through
/// http. Easy method until we have a set of DNS names we can rely on.
pub fn web_seeds() -> Box<Fn() -> Vec<SocketAddr> + Send> {
	Box::new(|| {
		let client = hyper::Client::new();
		debug!(LOGGER, "Retrieving seed nodes from {}", &SEEDS_URL);

		// http get, filtering out non 200 results
		let mut res = client
			.get(SEEDS_URL)
			.send()
			.expect("Failed to resolve seeds.");
		if res.status != hyper::Ok {
			panic!("Failed to resolve seeds, got status {}.", res.status);
		}
		let mut buf = vec![];
		res.read_to_end(&mut buf)
			.expect("Could not read seed list.");

		let text = str::from_utf8(&buf[..]).expect("Corrupted seed list.");
		let addrs = text.split_whitespace()
			.map(|s| s.parse().unwrap())
			.collect::<Vec<_>>();
		debug!(LOGGER, "Retrieved seed addresses: {:?}", addrs);
		addrs
	})
}

/// Convenience function when the seed list is immediately known. Mostly used
/// for tests.
pub fn predefined_seeds(addrs_str: Vec<String>) -> Box<Fn() -> Vec<SocketAddr> + Send> {
	Box::new(move || {
		addrs_str
			.iter()
			.map(|s| s.parse().unwrap())
			.collect::<Vec<_>>()
	})
}
