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

use chrono::prelude::Utc;
use rand::{thread_rng, Rng};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::common::adapters::DandelionAdapter;
use crate::core::core::hash::Hashed;
use crate::core::core::transaction;
use crate::core::core::verifier_cache::VerifierCache;
use crate::pool::{DandelionConfig, PoolError, TransactionPool, TxSource};
use crate::util::{Mutex, RwLock, StopState};

/// A process to monitor transactions in the stempool.
/// With Dandelion, transaction can be broadcasted in stem or fluff phase.
/// When sent in stem phase, the transaction is relayed to only node: the
/// dandelion relay. In order to maintain reliability a timer is started for
/// each transaction sent in stem phase. This function will monitor the
/// stempool and test if the timer is expired for each transaction. In that case
/// the transaction will be sent in fluff phase (to multiple peers) instead of
/// sending only to the peer relay.
pub fn monitor_transactions(
	dandelion_config: DandelionConfig,
	tx_pool: Arc<RwLock<TransactionPool>>,
	adapter: Arc<DandelionAdapter>,
	verifier_cache: Arc<RwLock<dyn VerifierCache>>,
	stop_state: Arc<Mutex<StopState>>,
) {
	debug!("Started Dandelion transaction monitor.");

	let _ = thread::Builder::new()
		.name("dandelion".to_string())
		.spawn(move || {
			loop {
				if stop_state.lock().is_stopped() {
					break;
				}

				// TODO - may be preferable to loop more often and check for expired patience time?
				// This is the patience timer, we loop every n secs.
				let patience_secs = dandelion_config.patience_secs.unwrap();
				thread::sleep(Duration::from_secs(patience_secs));

				// Our adapter hooks us into the current Dandelion "epoch".
				// From this we can determine if we should fluff txs in stempool.
				if adapter.is_expired() {
					adapter.next_epoch();
				}

				// Vastly simplified -
				// check if we are is_stem() via the adapter (current epoch)
				// * if we are stem then do nothing (nothing to aggregate here)
				// * if fluff then aggregate and add to txpool

				if !adapter.is_stem() {
					if process_fluff_phase(&tx_pool, &verifier_cache).is_err() {
						error!("dand_mon: Problem processing fresh pool entries.");
					}
				}

				// Now find all expired entries based on embargo timer.
				if process_expired_entries(&dandelion_config, &tx_pool).is_err() {
					error!("dand_mon: Problem processing fresh pool entries.");
				}
			}
		});
}

fn process_fluff_phase(
	tx_pool: &Arc<RwLock<TransactionPool>>,
	verifier_cache: &Arc<RwLock<dyn VerifierCache>>,
) -> Result<(), PoolError> {
	let mut tx_pool = tx_pool.write();

	let stem_txs = tx_pool.stempool.all_transactions();
	if stem_txs.is_empty() {
		return Ok(());
	}

	// Get the aggregate tx representing the entire txpool.
	let txpool_tx = tx_pool.txpool.all_transactions_aggregate()?;

	let header = tx_pool.chain_head()?;
	let stem_txs =
		tx_pool
			.stempool
			.select_valid_transactions(&stem_txs, txpool_tx.as_ref(), &header)?;

	if stem_txs.is_empty() {
		return Ok(());
	}

	debug!("dand_mon: Found {} txs for fluffing.", stem_txs.len());

	let agg_tx = transaction::aggregate(&stem_txs)?;
	agg_tx.validate(verifier_cache.clone())?;

	let src = TxSource {
		debug_name: "fluff".to_string(),
		identifier: "?.?.?.?".to_string(),
	};

	tx_pool.add_to_pool(src, agg_tx, false, &header)?;
	Ok(())
}

fn process_expired_entries(
	dandelion_config: &DandelionConfig,
	tx_pool: &Arc<RwLock<TransactionPool>>,
) -> Result<(), PoolError> {
	let now = Utc::now().timestamp();
	let embargo_sec = dandelion_config.embargo_secs.unwrap() + thread_rng().gen_range(0, 31);
	let cutoff = now - embargo_sec as i64;

	let mut expired_entries = vec![];
	{
		let tx_pool = tx_pool.read();
		for entry in tx_pool
			.stempool
			.entries
			.iter()
			.filter(|x| x.tx_at.timestamp() < cutoff)
		{
			debug!("dand_mon: Embargo timer expired for {:?}", entry.tx.hash());
			expired_entries.push(entry.clone());
		}
	}

	if expired_entries.is_empty() {
		return Ok(());
	}

	debug!("dand_mon: Found {} expired txs.", expired_entries.len());

	let mut tx_pool = tx_pool.write();
	let header = tx_pool.chain_head()?;

	let src = TxSource {
		debug_name: "embargo_expired".to_string(),
		identifier: "?.?.?.?".to_string(),
	};

	for entry in expired_entries {
		match tx_pool.add_to_pool(src.clone(), entry.tx, false, &header) {
			Ok(_) => debug!("dand_mon: embargo expired, fluffed tx successfully."),
			Err(e) => debug!("dand_mon: Failed to fluff expired tx - {:?}", e),
		};
	}

	Ok(())
}
