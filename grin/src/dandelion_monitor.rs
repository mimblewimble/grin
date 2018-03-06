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
use time::{self, now_utc};
use util::LOGGER;

use pool::TransactionPool;
use pool::PoolConfig;
use pool::TxSource;
use pool::BlockChain;

/// A process to monitor transactions in the stempool.
/// Periodically read the stempool and test if the embargo timer is expired.
/// In that case the transaction will be sent in fluff phase (to multiple peers) instead of
/// sending only to the peer relay
pub fn monitor_transactions<T>(
	config: PoolConfig,
	tx_pool: Arc<RwLock<TransactionPool<T>>>,
	stop: Arc<AtomicBool>,
) where
	T: BlockChain + Send + Sync + 'static,
{
	let _ = thread::Builder::new()
		.name("dandelion".to_string())
		.spawn(move || {
			let stem_transactions = tx_pool.write().unwrap().stem_transactions.clone();
			let time_stem_transactions = tx_pool.write().unwrap().time_stem_transactions.clone();

			let mut prev = time::now_utc() - time::Duration::seconds(60);
			loop {
				let current_time = time::now_utc();

				if current_time - prev > time::Duration::seconds(20) {
					for tx_hash in stem_transactions.keys() {
						let time_transaction = time_stem_transactions.get(tx_hash).unwrap();
						let interval = now_utc().to_timespec().sec - time_transaction;
						// Unban peer
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
								Ok(()) => info!(
									LOGGER,
									"Fluffing transaction after embargo timer expired."
								),
								Err(e) => debug!(LOGGER, "error - {:?}", e),
							};
							// Remove from tx pool
							tx_pool.write().unwrap().remove_from_stempool(tx_hash);
						}
					}
					prev = current_time;
				}

				thread::sleep(Duration::from_secs(1));

				if stop.load(Ordering::Relaxed) {
					break;
				}
			}
		});
}
