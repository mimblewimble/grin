// Copyright 2021 The Grin Developers
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
use std::time::{Duration, Instant};

use crate::common::adapters::DandelionAdapter;
use crate::core::core::hash::Hashed;
use crate::core::core::transaction;
use crate::pool::{BlockChain, DandelionConfig, Pool, PoolEntry, PoolError, TxSource};
use crate::util::StopState;
use crate::ServerTxPool;

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
	tx_pool: ServerTxPool,
	adapter: Arc<dyn DandelionAdapter>,
	stop_state: Arc<StopState>,
) -> std::io::Result<thread::JoinHandle<()>> {
	debug!("Started Dandelion transaction monitor.");

	thread::Builder::new()
		.name("dandelion".to_string())
		.spawn(move || {
			let run_interval = Duration::from_secs(10);
			let mut last_run = Instant::now()
				.checked_sub(Duration::from_secs(20))
				.unwrap_or_else(Instant::now);
			loop {
				// Halt Dandelion monitor if we have been notified that we are stopping.
				if stop_state.is_stopped() {
					break;
				}

				if last_run.elapsed() > run_interval {
					if !adapter.is_stem() {
						let _ = process_fluff_phase(&dandelion_config, &tx_pool, &adapter).map_err(
							|e| {
								error!("dand_mon: Problem processing fluff phase. {:?}", e);
							},
						);
					}

					// Now find all expired entries based on embargo timer.
					let _ = process_expired_entries(&dandelion_config, &tx_pool).map_err(|e| {
						error!("dand_mon: Problem processing expired entries. {:?}", e);
					});

					// Handle the tx above *before* we transition to next epoch.
					// This gives us an opportunity to do the final "fluff" before we start
					// stemming on the subsequent epoch.
					if adapter.is_expired() {
						adapter.next_epoch();
					}
					last_run = Instant::now();
				}

				// Monitor loops every 10s, but check stop flag every second.
				thread::sleep(Duration::from_secs(1));
			}
		})
}

// Query the pool for transactions older than the cutoff.
// Used for both periodic fluffing and handling expired embargo timer.
fn select_txs_cutoff<B>(pool: &Pool<B>, cutoff_secs: u16) -> Vec<PoolEntry>
where
	B: BlockChain,
{
	let cutoff = Utc::now().timestamp() - cutoff_secs as i64;
	pool.entries
		.iter()
		.filter(|x| x.tx_at.timestamp() < cutoff)
		.cloned()
		.collect()
}

fn process_fluff_phase(
	dandelion_config: &DandelionConfig,
	tx_pool: &ServerTxPool,
	adapter: &Arc<dyn DandelionAdapter>,
) -> Result<(), PoolError> {
	// Take a write lock on the txpool for the duration of this processing.
	let mut tx_pool = tx_pool.write();

	let all_entries = tx_pool.stempool.entries.clone();
	if all_entries.is_empty() {
		return Ok(());
	}

	let cutoff_secs = dandelion_config.aggregation_secs;
	let cutoff_entries = select_txs_cutoff(&tx_pool.stempool, cutoff_secs);

	// If epoch is expired, fluff *all* outstanding entries in stempool.
	// If *any* entry older than aggregation_secs (30s) then fluff *all* entries.
	// Otherwise we are done for now and we can give txs more time to aggregate.
	if !adapter.is_expired() && cutoff_entries.is_empty() {
		return Ok(());
	}

	let header = tx_pool.chain_head()?;

	let fluffable_txs = {
		let txpool_tx = tx_pool.txpool.all_transactions_aggregate(None)?;
		let txs: Vec<_> = all_entries.into_iter().map(|x| x.tx).collect();
		tx_pool.stempool.validate_raw_txs(
			&txs,
			txpool_tx,
			&header,
			transaction::Weighting::NoLimit,
		)?
	};

	debug!(
		"dand_mon: Found {} txs in local stempool to fluff",
		fluffable_txs.len()
	);

	let agg_tx = transaction::aggregate(&fluffable_txs)?;
	agg_tx.validate(transaction::Weighting::AsTransaction)?;

	tx_pool.add_to_pool(TxSource::Fluff, agg_tx, false, &header)?;
	Ok(())
}

fn process_expired_entries(
	dandelion_config: &DandelionConfig,
	tx_pool: &ServerTxPool,
) -> Result<(), PoolError> {
	// Take a write lock on the txpool for the duration of this processing.
	let mut tx_pool = tx_pool.write();

	let embargo_secs = dandelion_config.embargo_secs + thread_rng().gen_range(0, 31);
	let expired_entries = select_txs_cutoff(&tx_pool.stempool, embargo_secs);

	if expired_entries.is_empty() {
		return Ok(());
	}

	debug!("dand_mon: Found {} expired txs.", expired_entries.len());

	let header = tx_pool.chain_head()?;

	for entry in expired_entries {
		let txhash = entry.tx.hash();
		match tx_pool.add_to_pool(TxSource::EmbargoExpired, entry.tx, false, &header) {
			Ok(_) => info!(
				"dand_mon: embargo expired for {}, fluffed successfully.",
				txhash
			),
			Err(e) => warn!("dand_mon: failed to fluff expired tx {}, {:?}", txhash, e),
		};
	}
	Ok(())
}
