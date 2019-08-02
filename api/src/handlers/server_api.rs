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

use super::utils::w;
use crate::chain::{Chain, SyncState, SyncStatus};
use crate::p2p;
use crate::rest::*;
use crate::router::{Handler, ResponseFuture};
use crate::types::*;
use crate::web::*;
use hyper::{Body, Request, StatusCode};
use std::sync::Weak;

// RESTful index of available api endpoints
// GET /v1/
pub struct IndexHandler {
	pub list: Vec<String>,
}

impl IndexHandler {}

impl Handler for IndexHandler {
	fn get(&self, _req: Request<Body>) -> ResponseFuture {
		json_response_pretty(&self.list)
	}
}

pub struct KernelDownloadHandler {
	pub peers: Weak<p2p::Peers>,
}

impl Handler for KernelDownloadHandler {
	fn post(&self, _req: Request<Body>) -> ResponseFuture {
		if let Some(peer) = w_fut!(&self.peers).most_work_peer() {
			match peer.send_kernel_data_request() {
				Ok(_) => response(StatusCode::OK, "{}"),
				Err(e) => response(
					StatusCode::INTERNAL_SERVER_ERROR,
					format!("requesting kernel data from peer failed: {:?}", e),
				),
			}
		} else {
			response(
				StatusCode::INTERNAL_SERVER_ERROR,
				format!("requesting kernel data from peer failed (no peers)"),
			)
		}
	}
}

/// Status handler. Post a summary of the server status
/// GET /v1/status
pub struct StatusHandler {
	pub chain: Weak<Chain>,
	pub peers: Weak<p2p::Peers>,
	pub sync_state: Weak<SyncState>,
}

impl StatusHandler {
	fn get_status(&self) -> Result<Status, Error> {
		let head = w(&self.chain)?
			.head()
			.map_err(|e| ErrorKind::Internal(format!("can't get head: {}", e)))?;
		let sync_status = w(&self.sync_state)?.status();
		Ok(Status::from_tip_and_peers(
			head,
			w(&self.peers)?.peer_count(),
			sync_status_to_api_string(sync_status),
		))
	}
}

impl Handler for StatusHandler {
	fn get(&self, _req: Request<Body>) -> ResponseFuture {
		result_to_response(self.get_status())
	}
}

/// Convert a SyncStatus to correspond sync_status API string
fn sync_status_to_api_string(sync_status: SyncStatus) -> String {
	match sync_status {
		SyncStatus::Initial => "initial".to_string(),
		SyncStatus::NoSync => "no_sync".to_string(),
		SyncStatus::AwaitingPeers(_) => "awaiting_peers".to_string(),
		SyncStatus::HeaderSync {
			current_height,
			highest_height,
		} => format!("header_sync {}/{}", current_height, highest_height),
		SyncStatus::TxHashsetDownload {
			start_time: _,
			prev_update_time: _,
			update_time: _,
			prev_downloaded_size: _,
			downloaded_size,
			total_size,
		} => format!("txhashset_download {}/{}", downloaded_size, total_size),
		SyncStatus::TxHashsetSetup => "txhashset_setup".to_string(),
		SyncStatus::TxHashsetValidation {
			kernels,
			kernel_total,
			rproofs,
			rproof_total,
		} => format!(
			"txhashset_validation, kernels {}/{}, rangeproofs {}/{}",
			kernels, kernel_total, rproofs, rproof_total
		),
		SyncStatus::TxHashsetSave => "txhashset_save".to_string(),
		SyncStatus::TxHashsetDone => "txhashset_done".to_string(),
		SyncStatus::BodySync {
			current_height,
			highest_height,
		} => format!("body_sync {}/{}", current_height, highest_height),
		SyncStatus::Shutdown => "shutdown".to_string(),
		// any other status is considered syncing (should be unreachable)
		_ => "syncing".to_string(),
	}
}
