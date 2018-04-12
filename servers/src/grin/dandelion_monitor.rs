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

use std::thread;
use std::time::Duration;
use std::sync::{Arc, RwLock};
use std::sync::atomic::{AtomicBool, Ordering};
use time::now_utc;
use util::LOGGER;

use pool::TransactionPool;
use pool::PoolConfig;
use pool::TxSource;
use pool::BlockChain;

/// A process to monitor transactions in the stempool.
/// With Dandelion, transaction can be broadcasted in stem or fluff phase.
/// When sent in stem phase, the transaction is relayed to only node: the dandelion relay. In
/// order to maintain reliability a timer is started for each transaction sent in stem phase.
/// This function will monitor the stempool and test if the timer is expired for each transaction.
/// In that case the transaction will be sent in fluff phase (to multiple peers) instead of
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
				let stem_transactions = tx_pool.read().unwrap().stem_transactions.clone();
				let time_stem_transactions = tx_pool.read().unwrap().time_stem_transactions.clone();

				for tx_hash in stem_transactions.keys() {
					let time_transaction = time_stem_transactions.get(tx_hash).unwrap();
					let interval = now_utc().to_timespec().sec - time_transaction;

					if interval >= config.dandelion_embargo {
						let source = TxSource {
							debug_name: "dandelion-monitor".to_string(),
							identifier: "?.?.?.?".to_string(),
						};
						let stem_transaction = stem_transactions.get(tx_hash).unwrap();
						let res = tx_pool.write().unwrap().add_to_memory_pool(
							source,
							*stem_transaction.clone(),
							false,
						);

						match res {
							Ok(()) => {
								info!(LOGGER, "Fluffing transaction after embargo timer expired.")
							}
							Err(e) => debug!(LOGGER, "error - {:?}", e),
						};
						// Remove from tx pool
						tx_pool.write().unwrap().remove_from_stempool(tx_hash);
					}
				}

				thread::sleep(Duration::from_secs(1));

				if stop.load(Ordering::Relaxed) {
					break;
				}
			}
		});
}
