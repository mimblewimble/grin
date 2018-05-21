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
				//

				for entry in tx_pool.read().unwrap().stempool.entries.clone() {
					let interval = now_utc().to_timespec().sec - entry.tx_at;
					if interval > config.dandelion_embargo {
						match tx_pool
							.write()
							.unwrap()
							.add_to_pool(entry.src, entry.tx, false)
						{
							Ok(()) => info!(LOGGER, "Fluffing tx after embargo timer expired."),
							Err(e) => debug!(LOGGER, "error - {:?}", e),
						};
					}
				}

				thread::sleep(Duration::from_secs(1));

				if stop.load(Ordering::Relaxed) {
					break;
				}
			}
		});
}
