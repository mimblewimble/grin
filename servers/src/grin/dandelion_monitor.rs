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

use rand;
use rand::Rng;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;
use time::now_utc;
use util::LOGGER;

use pool::BlockChain;
use pool::PoolConfig;
use pool::TransactionPool;
use pool::TxSource;

/// A process to monitor transactions in the stempool.
/// With Dandelion, transaction can be broadcasted in stem or fluff phase.
/// When sent in stem phase, the transaction is relayed to only node: the
/// dandelion relay. In order to maintain reliability a timer is started for
/// each transaction sent in stem phase. This function will monitor the
/// stempool and test if the timer is expired for each transaction. In that case
/// the transaction will be sent in fluff phase (to multiple peers) instead of
/// sending only to the peer relay.
pub fn monitor_transactions<T>(
	config: PoolConfig,
	tx_pool: Arc<RwLock<TransactionPool<T>>>,
	stop: Arc<AtomicBool>,
) where
	T: BlockChain + Send + Sync + 'static,
{
	debug!(LOGGER, "Started Dandelion transaction monitor");
	let _ = thread::Builder::new()
		.name("dandelion".to_string())
		.spawn(move || {
			loop {
				let tx_pool = tx_pool.clone();

				let now = now_utc().to_timespec().sec;

				let mut fresh_entries = vec![];
				let mut fluff_stempool = false;
				{
					let mut tx_pool = tx_pool.write().unwrap();
					let mut rng = rand::thread_rng();

					for mut entry in tx_pool.stempool.entries.iter_mut() {
						//
						// TODO patience timer config
						//
						let cutoff = now - 10;

						if entry.fresh && entry.tx_at.sec < cutoff {
							entry.fresh = false;
							let random = rng.gen_range(0, 101);
							if random <= config.dandelion_probability {
								debug!(
									LOGGER,
									"dand_mon: Not fluffing stempool, will propagate to Dandelion relay."
								);
								fresh_entries.push(entry.clone());
							} else {
								fluff_stempool = true;
								break;
							}
						}
					}
				}

				if fluff_stempool {
					let mut tx_pool = tx_pool.write().unwrap();
					if tx_pool.fluff_stempool().is_err() {
						error!(LOGGER, "Failed to fluff stempool.");
					}
				} else {
					let mut tx_pool = tx_pool.write().unwrap();
					for x in fresh_entries {
						// TODO - maybe adapter needs a has_dandelion_relay() so we can
						// conditionally propagate or aggregate and fluff here?
						let res = tx_pool.adapter.stem_tx_accepted(&x.tx);
						if res.is_err() {
							debug!(LOGGER, "Could not propagate stem tx, fluffing stempool.");
							if tx_pool.fluff_stempool().is_err() {
								error!(LOGGER, "Failed to fluff stempool.");
							}
						}
					}
				}

				// Randomize the cutoff time based on Dandelion embargo cofiguration.
				// Anything older than this gets "fluffed" as a fallback.
				let embargo_sec = config.dandelion_embargo + rand::thread_rng().gen_range(0, 31);
				let cutoff = now - embargo_sec;

				let mut expired_entries = vec![];
				{
					let tx_pool = tx_pool.read().unwrap();
					for entry in tx_pool
						.stempool
						.entries
						.iter()
						.filter(|x| x.tx_at.sec < cutoff)
					{
						debug!(LOGGER, "dand_mon: Fluffing tx after embargo timer expired.");
						expired_entries.push(entry.clone());
					}
				}

				{
					let mut tx_pool = tx_pool.write().unwrap();
					for entry in expired_entries {
						match tx_pool.add_to_pool(entry.src, entry.tx, false) {
							Ok(()) => debug!(LOGGER, "Fluffed tx successfully."),
							Err(e) => debug!(LOGGER, "error - {:?}", e),
						};
					}
				}

				thread::sleep(Duration::from_secs(10));

				if stop.load(Ordering::Relaxed) {
					break;
				}
			}
		});
}
