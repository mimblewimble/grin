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

//! Basic status view definition

use chrono::prelude::Utc;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use std::borrow::Cow;

use crate::tui::app::App;

use crate::chain::{HeaderSyncMode, SyncStatus};

const NANO_TO_MILLIS: f64 = 1.0 / 1_000_000.0;

pub fn update_sync_status(sync_status: SyncStatus) -> Cow<'static, str> {
	match sync_status {
		SyncStatus::Initial => Cow::Borrowed("Initializing"),
		SyncStatus::NoSync => Cow::Borrowed("Running"),
		SyncStatus::AwaitingPeers(_) => Cow::Borrowed("Waiting for peers"),
		SyncStatus::HeaderSync {
			sync_head,
			sync_mode,
			highest_height,
			..
		} => {
			let percent = if highest_height == 0 {
				0
			} else {
				sync_head.height * 100 / highest_height
			};
			let sync_mode = match sync_mode {
				HeaderSyncMode::Legacy => "Legacy",
				HeaderSyncMode::Pihd => "PIHD",
			};
			Cow::Owned(format!(
				"Sync step 1/7: Downloading headers ({}) - {}%",
				sync_mode, percent
			))
		}
		SyncStatus::TxHashsetPibd {
			aborted: _,
			errored: _,
			completed_leaves,
			leaves_required,
			completed_to_height: _,
			required_height: _,
		} => {
			let percent = if completed_leaves == 0 {
				0
			} else {
				completed_leaves * 100 / leaves_required
			};
			Cow::Owned(format!(
				"Sync step 2/7: Downloading Tx state (PIBD) - {} / {} entries - {}%",
				completed_leaves, leaves_required, percent
			))
		}
		SyncStatus::TxHashsetDownload(stat) => {
			if stat.total_size > 0 {
				let percent = stat.downloaded_size * 100 / stat.total_size;
				let start = stat
					.prev_update_time
					.timestamp_nanos_opt()
					.unwrap_or_default();
				let fin = Utc::now().timestamp_nanos_opt().unwrap_or_default();
				let dur_ms = (fin - start) as f64 * NANO_TO_MILLIS;

				Cow::Owned(format!("Sync step 2/7: Downloading {}(MB) chain state for state sync: {}% at {:.1?}(kB/s)",
						stat.total_size / 1_000_000,
						percent,
						if dur_ms > 1.0f64 { stat.downloaded_size.saturating_sub(stat.prev_downloaded_size) as f64 / dur_ms as f64 } else { 0f64 },
				))
			} else {
				let start = stat.start_time.timestamp_millis();
				let fin = Utc::now().timestamp_millis();
				let dur_secs = (fin - start) / 1000;

				Cow::Owned(format!("Sync step 2/7: Downloading chain state for state sync. Waiting remote peer to start: {}s",
						dur_secs,
				))
			}
		}
		SyncStatus::TxHashsetSetup {
			headers,
			headers_total,
			kernel_pos,
			kernel_pos_total,
		} => {
			if headers.is_some() && headers_total.is_some() {
				let h = headers.unwrap();
				let ht = headers_total.unwrap();
				let percent = h * 100 / ht;
				Cow::Owned(format!(
					"Sync step 3/7: Preparing for validation (kernel history) - {}/{} - {}%",
					h, ht, percent
				))
			} else if kernel_pos.is_some() && kernel_pos_total.is_some() {
				let k = kernel_pos.unwrap();
				let kt = kernel_pos_total.unwrap();
				let percent = k * 100 / kt;
				Cow::Owned(format!(
					"Sync step 3/7: Preparing for validation (kernel position) - {}/{} - {}%",
					k, kt, percent
				))
			} else {
				Cow::Borrowed("Sync step 3/7: Preparing chain state for validation")
			}
		}
		SyncStatus::TxHashsetRangeProofsValidation {
			rproofs,
			rproofs_total,
		} => {
			let r_percent = if rproofs_total > 0 {
				(rproofs * 100) / rproofs_total
			} else {
				0
			};
			Cow::Owned(format!(
				"Sync step 4/7: Validating chain state - range proofs: {}%",
				r_percent
			))
		}
		SyncStatus::TxHashsetKernelsValidation {
			kernels,
			kernels_total,
		} => {
			let k_percent = if kernels_total > 0 {
				(kernels * 100) / kernels_total
			} else {
				0
			};
			Cow::Owned(format!(
				"Sync step 5/7: Validating chain state - kernels: {}%",
				k_percent
			))
		}
		SyncStatus::TxHashsetSave => {
			Cow::Borrowed("Sync step 6/7: Finalizing chain state for state sync")
		}
		SyncStatus::TxHashsetDone => {
			Cow::Borrowed("Sync step 6/7: Finalized chain state for state sync")
		}
		SyncStatus::BodySync {
			current_height,
			highest_height,
		} => {
			let percent = if highest_height == 0 {
				0
			} else {
				current_height * 100 / highest_height
			};
			Cow::Owned(format!("Sync step 7/7: Downloading blocks: {}%", percent))
		}
		SyncStatus::Shutdown => Cow::Borrowed("Shutting down, closing connections"),
	}
}

fn line(label: &str, value: impl Into<String>) -> Line<'static> {
	Line::from(format!("{:<30}{}", label, value.into()))
}

const SEPARATOR: &str = "--------------------------------------------------------------------";

/// Draw the basic status view
pub fn draw(f: &mut Frame, area: Rect, app: &App) {
	let mut lines: Vec<Line> = Vec::new();

	match &app.stats {
		None => {
			lines.push(line("Current Status:", "Starting"));
		}
		Some(stats) => {
			let sync_status = update_sync_status(stats.sync_status);
			lines.push(line("Uptime:", stats.uptime_format()));
			lines.push(line("Current Status:", sync_status.to_string()));
			lines.push(line("Connected Peers:", stats.peer_count.to_string()));
			lines.push(line("Disk Usage (GB):", stats.disk_usage_gb.clone()));
			lines.push(Line::from(SEPARATOR));
			lines.push(line(
				"Header Tip Hash:",
				format!("{}...", stats.header_stats.last_block_h),
			));
			lines.push(line(
				"Header Chain Height:",
				stats.header_stats.height.to_string(),
			));
			lines.push(line(
				"Header Cumulative Difficulty:",
				stats.header_stats.total_difficulty.to_string(),
			));
			lines.push(line(
				"Header Tip Timestamp:",
				stats.header_stats.latest_timestamp.to_string(),
			));
			lines.push(Line::from(SEPARATOR));
			lines.push(line(
				"Chain Tip Hash:",
				format!("{}...", stats.chain_stats.last_block_h),
			));
			lines.push(line("Chain Height:", stats.chain_stats.height.to_string()));
			lines.push(line(
				"Chain Cumulative Difficulty:",
				stats.chain_stats.total_difficulty.to_string(),
			));
			lines.push(line(
				"Chain Tip Timestamp:",
				stats.chain_stats.latest_timestamp.to_string(),
			));
			lines.push(Line::from(SEPARATOR));
			if let Some(tx_stats) = &stats.tx_stats {
				lines.push(line(
					"Transaction Pool Size:",
					format!("{} ({})", tx_stats.tx_pool_size, tx_stats.tx_pool_kernels),
				));
				lines.push(line(
					"Stem Pool Size:",
					format!(
						"{} ({})",
						tx_stats.stem_pool_size, tx_stats.stem_pool_kernels
					),
				));
			} else {
				lines.push(line("Transaction Pool Size:", "0 (0)"));
				lines.push(line("Stem Pool Size:", "0 (0)"));
			}
		}
	}

	let paragraph = Paragraph::new(lines);
	f.render_widget(paragraph, area);
}

#[test]
fn test_status_txhashset_kernels() {
	let status = SyncStatus::TxHashsetKernelsValidation {
		kernels: 201,
		kernels_total: 5000,
	};
	let basic_status = update_sync_status(status);
	assert!(basic_status.contains("4%"), "{}", basic_status);
}

#[test]
fn test_status_txhashset_rproofs() {
	let status = SyncStatus::TxHashsetRangeProofsValidation {
		rproofs: 643,
		rproofs_total: 1000,
	};
	let basic_status = update_sync_status(status);
	assert!(basic_status.contains("64%"), "{}", basic_status);
}
