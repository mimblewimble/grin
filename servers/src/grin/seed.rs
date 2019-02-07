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

//! Seeds a server with initial peers on first start and keep monitoring
//! peer counts to connect to more if neeed. Seedin strategy is
//! configurable with either no peers, a user-defined list or a preset
//! list of DNS records (the default).

use chrono::prelude::{DateTime, Utc};
use chrono::{Duration, MIN_DATE};
use rand::{thread_rng, Rng};
use std::collections::HashMap;
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::{mpsc, Arc};
use std::{cmp, str, thread, time};

use crate::core::global;
use crate::p2p;
use crate::p2p::ChainAdapter;
use crate::pool::DandelionConfig;
use crate::util::{Mutex, StopState};

// DNS Seeds with contact email associated
const MAINNET_DNS_SEEDS: &'static [&'static str] = &[
	"mainnet.seed.grin-tech.org",      // igno.peverell@protonmail.com
	"mainnet.seed.grin.icu",           // gary.peverell@protonmail.com
	"mainnet.seed.713.mw",             // jasper@713.mw
	"mainnet.seed.grin.lesceller.com", // q.lesceller@gmail.com
	"mainnet.seed.grin.prokapi.com",   // hendi@prokapi.com
	"grinseed.yeastplume.org",         // yeastplume@protonmail.com
];
const FLOONET_DNS_SEEDS: &'static [&'static str] = &[
	"floonet.seed.grin-tech.org",      // igno.peverell@protonmail.com
	"floonet.seed.grin.icu",           // gary.peverell@protonmail.com
	"floonet.seed.713.mw",             // jasper@713.mw
	"floonet.seed.grin.lesceller.com", // q.lesceller@gmail.com
	"floonet.seed.grin.prokapi.com",   // hendi@prokapi.com
];

pub fn connect_and_monitor(
	p2p_server: Arc<p2p::Server>,
	capabilities: p2p::Capabilities,
	dandelion_config: DandelionConfig,
	seed_list: Box<dyn Fn() -> Vec<SocketAddr> + Send>,
	preferred_peers: Option<Vec<SocketAddr>>,
	stop_state: Arc<Mutex<StopState>>,
) {
	let _ = thread::Builder::new()
		.name("seed".to_string())
		.spawn(move || {
			let peers = p2p_server.peers.clone();

			// open a channel with a listener that connects every peer address sent below
			// max peer count
			let (tx, rx) = mpsc::channel();

			// check seeds first
			connect_to_seeds_and_preferred_peers(
				peers.clone(),
				tx.clone(),
				seed_list,
				preferred_peers.clone(),
			);

			let mut prev = MIN_DATE.and_hms(0, 0, 0);
			let mut prev_expire_check = MIN_DATE.and_hms(0, 0, 0);
			let mut prev_ping = Utc::now();
			let mut start_attempt = 0;

			let mut connecting_history: HashMap<SocketAddr, DateTime<Utc>> = HashMap::new();

			loop {
				if stop_state.lock().is_stopped() {
					break;
				}

				// Pause egress peer connection request. Only for tests.
				if stop_state.lock().is_paused() {
					thread::sleep(time::Duration::from_secs(1));
					continue;
				}

				// Check for and remove expired peers from the storage
				if Utc::now() - prev_expire_check > Duration::hours(1) {
					peers.remove_expired();

					prev_expire_check = Utc::now();
				}

				// make several attempts to get peers as quick as possible
				// with exponential backoff
				if Utc::now() - prev > Duration::seconds(cmp::min(20, 1 << start_attempt)) {
					// try to connect to any address sent to the channel
					listen_for_addrs(
						peers.clone(),
						p2p_server.clone(),
						capabilities,
						&rx,
						&mut connecting_history,
					);

					// monitor additional peers if we need to add more
					monitor_peers(
						peers.clone(),
						p2p_server.config.clone(),
						tx.clone(),
						preferred_peers.clone(),
					);

					update_dandelion_relay(peers.clone(), dandelion_config.clone());

					prev = Utc::now();
					start_attempt = cmp::min(6, start_attempt + 1);
				}

				// Ping connected peers on every 10s to monitor peers.
				if Utc::now() - prev_ping > Duration::seconds(10) {
					let total_diff = peers.total_difficulty();
					let total_height = peers.total_height();
					peers.check_all(total_diff, total_height);
					prev_ping = Utc::now();
				}

				thread::sleep(time::Duration::from_secs(1));
			}
		});
}

fn monitor_peers(
	peers: Arc<p2p::Peers>,
	config: p2p::P2PConfig,
	tx: mpsc::Sender<SocketAddr>,
	preferred_peers_list: Option<Vec<SocketAddr>>,
) {
	// regularly check if we need to acquire more peers  and if so, gets
	// them from db
	let total_count = peers.all_peers().len();
	let mut healthy_count = 0;
	let mut banned_count = 0;
	let mut defuncts = vec![];

	for x in peers.all_peers() {
		match x.flags {
			p2p::State::Banned => {
				let interval = Utc::now().timestamp() - x.last_banned;
				// Unban peer
				if interval >= config.ban_window() {
					peers.unban_peer(&x.addr);
					debug!(
						"monitor_peers: unbanned {} after {} seconds",
						x.addr, interval
					);
				} else {
					banned_count += 1;
				}
			}
			p2p::State::Healthy => healthy_count += 1,
			p2p::State::Defunct => defuncts.push(x),
		}
	}

	debug!(
		"monitor_peers: on {}:{}, {} connected ({} most_work). \
		 all {} = {} healthy + {} banned + {} defunct",
		config.host,
		config.port,
		peers.peer_count(),
		peers.most_work_peers().len(),
		total_count,
		healthy_count,
		banned_count,
		defuncts.len(),
	);

	// maintenance step first, clean up p2p server peers
	peers.clean_peers(config.peer_max_count() as usize);

	if peers.healthy_peers_mix() {
		return;
	}

	// loop over connected peers
	// ask them for their list of peers
	let mut connected_peers: Vec<SocketAddr> = vec![];
	for p in peers.connected_peers() {
		trace!(
			"monitor_peers: {}:{} ask {} for more peers",
			config.host,
			config.port,
			p.info.addr,
		);
		let _ = p.send_peer_request(p2p::Capabilities::PEER_LIST);
		connected_peers.push(p.info.addr)
	}

	// Attempt to connect to preferred peers if there is some
	match preferred_peers_list {
		Some(preferred_peers) => {
			for p in preferred_peers {
				if !connected_peers.is_empty() {
					if !connected_peers.contains(&p) {
						tx.send(p).unwrap();
					}
				} else {
					tx.send(p).unwrap();
				}
			}
		}
		None => debug!("monitor_peers: no preferred peers"),
	}

	// take a random defunct peer and mark it healthy: over a long period any
	// peer will see another as defunct eventually, gives us a chance to retry
	if defuncts.len() > 0 {
		thread_rng().shuffle(&mut defuncts);
		let _ = peers.update_state(defuncts[0].addr, p2p::State::Healthy);
	}

	// find some peers from our db
	// and queue them up for a connection attempt
	let new_peers = peers.find_peers(
		p2p::State::Healthy,
		p2p::Capabilities::UNKNOWN,
		config.peer_max_count() as usize,
	);

	for p in new_peers.iter().filter(|p| !peers.is_known(&p.addr)) {
		trace!(
			"monitor_peers: on {}:{}, queue to soon try {}",
			config.host,
			config.port,
			p.addr,
		);
		tx.send(p.addr).unwrap();
	}
}

fn update_dandelion_relay(peers: Arc<p2p::Peers>, dandelion_config: DandelionConfig) {
	// Dandelion Relay Updater
	let dandelion_relay = peers.get_dandelion_relay();
	if dandelion_relay.is_empty() {
		debug!("monitor_peers: no dandelion relay updating");
		peers.update_dandelion_relay();
	} else {
		for last_added in dandelion_relay.keys() {
			let dandelion_interval = Utc::now().timestamp() - last_added;
			if dandelion_interval >= dandelion_config.relay_secs.unwrap() as i64 {
				debug!("monitor_peers: updating expired dandelion relay");
				peers.update_dandelion_relay();
			}
		}
	}
}

// Check if we have any pre-existing peer in db. If so, start with those,
// otherwise use the seeds provided.
fn connect_to_seeds_and_preferred_peers(
	peers: Arc<p2p::Peers>,
	tx: mpsc::Sender<SocketAddr>,
	seed_list: Box<dyn Fn() -> Vec<SocketAddr>>,
	peers_preferred_list: Option<Vec<SocketAddr>>,
) {
	// check if we have some peers in db
	// look for peers that are able to give us other peers (via PEER_LIST capability)
	let peers = peers.find_peers(p2p::State::Healthy, p2p::Capabilities::PEER_LIST, 100);

	// if so, get their addresses, otherwise use our seeds
	let mut peer_addrs = if peers.len() > 3 {
		peers.iter().map(|p| p.addr).collect::<Vec<_>>()
	} else {
		seed_list()
	};

	// If we have preferred peers add them to the connection
	match peers_preferred_list {
		Some(mut peers_preferred) => peer_addrs.append(&mut peers_preferred),
		None => trace!("No preferred peers"),
	};

	if peer_addrs.len() == 0 {
		warn!("No seeds were retrieved.");
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
	connecting_history: &mut HashMap<SocketAddr, DateTime<Utc>>,
) {
	// Pull everything currently on the queue off the queue.
	// Does not block so addrs may be empty.
	// We will take(max_peers) from this later but we want to drain the rx queue
	// here to prevent it backing up.
	let addrs: Vec<SocketAddr> = rx.try_iter().collect();

	// If we have a healthy number of outbound peers then we are done here.
	if peers.healthy_peers_mix() {
		return;
	}

	// Try to connect to (up to max peers) peer addresses.
	// Note: We drained the rx queue earlier to keep it under control.
	// Even if there are many addresses to try we will only try a bounded number of them.
	let connect_min_interval = 30;
	for addr in addrs.into_iter().take(p2p.config.peer_max_count() as usize) {
		// ignore the duplicate connecting to same peer within 30 seconds
		let now = Utc::now();
		if let Some(last_connect_time) = connecting_history.get(&addr) {
			if *last_connect_time + Duration::seconds(connect_min_interval) > now {
				debug!(
					"peer_connect: ignore a duplicate request to {}. previous connecting time: {}",
					addr,
					last_connect_time.format("%H:%M:%S%.3f").to_string(),
				);
				continue;
			} else {
				*connecting_history.get_mut(&addr).unwrap() = now;
			}
		}
		connecting_history.insert(addr, now);

		let peers_c = peers.clone();
		let p2p_c = p2p.clone();
		let _ = thread::Builder::new()
			.name("peer_connect".to_string())
			.spawn(move || match p2p_c.connect(&addr) {
				Ok(p) => {
					let _ = p.send_peer_request(capab);
					let _ = peers_c.update_state(addr, p2p::State::Healthy);
				}
				Err(_) => {
					let _ = peers_c.update_state(addr, p2p::State::Defunct);
				}
			});
	}

	// shrink the connecting history.
	// put a threshold here to avoid frequent shrinking in every call
	if connecting_history.len() > 100 {
		let now = Utc::now();
		let old: Vec<_> = connecting_history
			.iter()
			.filter(|&(_, t)| *t + Duration::seconds(connect_min_interval) < now)
			.map(|(s, _)| s.clone())
			.collect();
		for addr in old {
			connecting_history.remove(&addr);
		}
	}
}

pub fn dns_seeds() -> Box<dyn Fn() -> Vec<SocketAddr> + Send> {
	Box::new(|| {
		let mut addresses: Vec<SocketAddr> = vec![];
		let net_seeds = if global::is_floonet() {
			FLOONET_DNS_SEEDS
		} else {
			MAINNET_DNS_SEEDS
		};
		for dns_seed in net_seeds {
			let temp_addresses = addresses.clone();
			debug!("Retrieving seed nodes from dns {}", dns_seed);
			match (dns_seed.to_owned(), 0).to_socket_addrs() {
				Ok(addrs) => addresses.append(
					&mut (addrs
						.map(|mut addr| {
							addr.set_port(if global::is_floonet() { 13414 } else { 3414 });
							addr
						})
						.filter(|addr| !temp_addresses.contains(addr))
						.collect()),
				),
				Err(e) => debug!("Failed to resolve seed {:?} got error {:?}", dns_seed, e),
			}
		}
		debug!("Retrieved seed addresses: {:?}", addresses);
		addresses
	})
}

/// Convenience function when the seed list is immediately known. Mostly used
/// for tests.
pub fn predefined_seeds(addrs_str: Vec<String>) -> Box<dyn Fn() -> Vec<SocketAddr> + Send> {
	Box::new(move || {
		addrs_str
			.iter()
			.map(|s| s.parse().unwrap())
			.collect::<Vec<_>>()
	})
}

/// Convenience function when the seed list is immediately known. Mostly used
/// for tests.
pub fn preferred_peers(addrs_str: Vec<String>) -> Option<Vec<SocketAddr>> {
	if addrs_str.is_empty() {
		None
	} else {
		Some(
			addrs_str
				.iter()
				.map(|s| s.parse().unwrap())
				.collect::<Vec<_>>(),
		)
	}
}
