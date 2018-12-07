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

use crate::util::RwLock;
use chrono::prelude::Utc;
use rand::{thread_rng, Rng};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::core::core::hash::Hashed;
use crate::core::core::transaction;
use crate::core::core::verifier_cache::VerifierCache;
use crate::pool::{DandelionConfig, PoolEntryState, PoolError, TransactionPool, TxSource};

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
	verifier_cache: Arc<RwLock<dyn VerifierCache>>,
	stop: Arc<AtomicBool>,
) {
	debug!("Started Dandelion transaction monitor.");

	let _ = thread::Builder::new()
		.name("dandelion".to_string())
		.spawn(move || {
			loop {
				if stop.load(Ordering::Relaxed) {
					break;
				}

				// This is the patience timer, we loop every n secs.
				let patience_secs = dandelion_config.patience_secs.unwrap();
				thread::sleep(Duration::from_secs(patience_secs));

				// Step 1: find all "ToStem" entries in stempool from last run.
				// Aggregate them up to give a single (valid) aggregated tx and propagate it
				// to the next Dandelion relay along the stem.
				if process_stem_phase(tx_pool.clone(), verifier_cache.clone()).is_err() {
					error!("dand_mon: Problem with stem phase.");
				}

				// Step 2: find all "ToFluff" entries in stempool from last run.
				// Aggregate them up to give a single (valid) aggregated tx and (re)add it
				// to our pool with stem=false (which will then broadcast it).
				if process_fluff_phase(tx_pool.clone(), verifier_cache.clone()).is_err() {
					error!("dand_mon: Problem with fluff phase.");
				}

				// Step 3: now find all "Fresh" entries in stempool since last run.
				// Coin flip for each (90/10) and label them as either "ToStem" or "ToFluff".
				// We will process these in the next run (waiting patience secs).
				if process_fresh_entries(dandelion_config.clone(), tx_pool.clone()).is_err() {
					error!("dand_mon: Problem processing fresh pool entries.");
				}

				// Step 4: now find all expired entries based on embargo timer.
				if process_expired_entries(dandelion_config.clone(), tx_pool.clone()).is_err() {
					error!("dand_mon: Problem processing fresh pool entries.");
				}
			}
		});
}

fn process_stem_phase(
	tx_pool: Arc<RwLock<TransactionPool>>,
	verifier_cache: Arc<RwLock<dyn VerifierCache>>,
) -> Result<(), PoolError> {
	let mut tx_pool = tx_pool.write();

	let header = tx_pool.chain_head()?;

	let stem_txs = tx_pool
		.stempool
		.get_transactions_in_state(PoolEntryState::ToStem);

	if stem_txs.is_empty() {
		return Ok(());
	}

	let txpool_tx = tx_pool.txpool.aggregate_transaction()?;

	let stem_txs = tx_pool
		.stempool
		.select_valid_transactions(stem_txs, txpool_tx, &header)?;
	tx_pool
		.stempool
		.transition_to_state(&stem_txs, PoolEntryState::Stemmed);

	if stem_txs.len() > 0 {
		debug!("dand_mon: Found {} txs for stemming.", stem_txs.len());

		let agg_tx = transaction::aggregate(stem_txs)?;
		agg_tx.validate(verifier_cache.clone())?;

		let res = tx_pool.adapter.stem_tx_accepted(&agg_tx);
		if res.is_err() {
			debug!("dand_mon: Unable to propagate stem tx. No relay, fluffing instead.");

			let src = TxSource {
				debug_name: "no_relay".to_string(),
				identifier: "?.?.?.?".to_string(),
			};

			tx_pool.add_to_pool(src, agg_tx, false, &header)?;
		}
	}
	Ok(())
}

fn process_fluff_phase(
	tx_pool: Arc<RwLock<TransactionPool>>,
	verifier_cache: Arc<RwLock<dyn VerifierCache>>,
) -> Result<(), PoolError> {
	let mut tx_pool = tx_pool.write();

	let header = tx_pool.chain_head()?;

	let stem_txs = tx_pool
		.stempool
		.get_transactions_in_state(PoolEntryState::ToFluff);

	if stem_txs.is_empty() {
		return Ok(());
	}

	let txpool_tx = tx_pool.txpool.aggregate_transaction()?;

	let stem_txs = tx_pool
		.stempool
		.select_valid_transactions(stem_txs, txpool_tx, &header)?;
	tx_pool
		.stempool
		.transition_to_state(&stem_txs, PoolEntryState::Fluffed);

	if stem_txs.len() > 0 {
		debug!("dand_mon: Found {} txs for fluffing.", stem_txs.len());

		let agg_tx = transaction::aggregate(stem_txs)?;
		agg_tx.validate(verifier_cache.clone())?;

		let src = TxSource {
			debug_name: "fluff".to_string(),
			identifier: "?.?.?.?".to_string(),
		};

		tx_pool.add_to_pool(src, agg_tx, false, &header)?;
	}
	Ok(())
}

fn process_fresh_entries(
	dandelion_config: DandelionConfig,
	tx_pool: Arc<RwLock<TransactionPool>>,
) -> Result<(), PoolError> {
	let mut tx_pool = tx_pool.write();

	let mut rng = thread_rng();

	let fresh_entries = &mut tx_pool
		.stempool
		.entries
		.iter_mut()
		.filter(|x| x.state == PoolEntryState::Fresh)
		.collect::<Vec<_>>();

	if fresh_entries.len() > 0 {
		debug!(
			"dand_mon: Found {} fresh entries in stempool.",
			fresh_entries.len()
		);

		for x in &mut fresh_entries.iter_mut() {
			let random = rng.gen_range(0, 101);
			if random <= dandelion_config.stem_probability.unwrap() {
				x.state = PoolEntryState::ToStem;
			} else {
				x.state = PoolEntryState::ToFluff;
			}
		}
	}
	Ok(())
}

fn process_expired_entries(
	dandelion_config: DandelionConfig,
	tx_pool: Arc<RwLock<TransactionPool>>,
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

	if expired_entries.len() > 0 {
		debug!("dand_mon: Found {} expired txs.", expired_entries.len());

		{
			let mut tx_pool = tx_pool.write();
			let header = tx_pool.chain_head()?;

			for entry in expired_entries {
				let src = TxSource {
					debug_name: "embargo_expired".to_string(),
					identifier: "?.?.?.?".to_string(),
				};
				match tx_pool.add_to_pool(src, entry.tx, false, &header) {
					Ok(_) => debug!("dand_mon: embargo expired, fluffed tx successfully."),
					Err(e) => debug!("dand_mon: Failed to fluff expired tx - {:?}", e),
				};
			}
		}
	}
	Ok(())
}
