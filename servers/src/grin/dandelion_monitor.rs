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
use std::collections::HashMap;
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

				//
				// TODO - do we also want the patience timer to run here?
				// i.e. We do not immediately notify stem peer of new stempool tx,
				// we wait n secs before doing so?
				//

				let mut fresh_entries = vec![];
				let mut fluff_stempool = false;
				{
					let mut tx_pool = tx_pool.write().unwrap();
					let mut rng = rand::thread_rng();

					// TODO - also check the patience timer here for each tx.
					for mut entry in tx_pool.stempool.entries.iter_mut() {
						if entry.fresh {
							entry.fresh = false;
							let random = rng.gen_range(0, 101);
							if random <= config.dandelion_probability {
								info!(
									LOGGER,
									"Not fluffing stempool, will propagate to Dandelion relay."
								);
								fresh_entries.push(entry.clone());
							} else {
								info!(LOGGER, "Attempting to fluff stempool.");
								fluff_stempool = true;
								break;
							}
						}
					}
				}

				if fluff_stempool {
					let mut tx_pool = tx_pool.write().unwrap();
					if let Ok(agg_tx) = tx_pool.stempool.aggregate_transaction() {
						let src = TxSource {
							debug_name: "fluff".to_string(),
							identifier: "?.?.?.?".to_string(),
						};
						match tx_pool.add_to_pool(src, agg_tx, false) {
							Ok(()) => info!(
								LOGGER,
								"Aggregated stempool, adding aggregated tx to local txpool."
							),
							Err(e) => debug!(LOGGER, "Error - {:?}", e),
						};
					} else {
						error!(LOGGER, "Failed to aggregate stempool.");
					}
				} else {
					let tx_pool = tx_pool.read().unwrap();
					for x in fresh_entries {
						tx_pool.adapter.stem_tx_accepted(&x.tx);
					}
				}

				// Randomize the cutoff time based on Dandelion embargo cofiguration.
				// Anything older than this gets "fluffed" as a fallback.
				let now = now_utc().to_timespec().sec;
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
						info!(LOGGER, "Fluffing tx after embargo timer expired.");
						expired_entries.push(entry.clone());
					}
				}

				{
					let mut tx_pool = tx_pool.write().unwrap();
					for entry in expired_entries {
						match tx_pool.add_to_pool(entry.src, entry.tx, false) {
							Ok(()) => info!(LOGGER, "Fluffed tx successfully."),
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
