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

use super::utils::w;
use crate::chain::{Chain, SyncState, SyncStatus};
use crate::p2p;
use crate::rest::*;
use crate::router::{Handler, ResponseFuture};
use crate::types::*;
use crate::web::*;
use hyper::{Body, Request};
use serde_json::json;
use std::convert::TryInto;
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

/// Status handler. Post a summary of the server status
/// GET /v1/status
pub struct StatusHandler {
	pub chain: Weak<Chain>,
	pub peers: Weak<p2p::Peers>,
	pub sync_state: Weak<SyncState>,
}

impl StatusHandler {
	pub fn get_status(&self) -> Result<Status, Error> {
		let head = w(&self.chain)?
			.head()
			.map_err(|e| Error::Internal(format!("can't get head: {}", e)))?;
		let sync_status = w(&self.sync_state)?.status();
		let (api_sync_status, api_sync_info) = sync_status_to_api(sync_status);
		Ok(Status::from_tip_and_peers(
			head,
			w(&self.peers)?
				.iter()
				.connected()
				.count()
				.try_into()
				.unwrap(),
			api_sync_status,
			api_sync_info,
		))
	}
}

impl Handler for StatusHandler {
	fn get(&self, _req: Request<Body>) -> ResponseFuture {
		result_to_response(self.get_status())
	}
}

/// Convert a SyncStatus in a readable API representation
fn sync_status_to_api(sync_status: SyncStatus) -> (String, Option<serde_json::Value>) {
	match sync_status {
		SyncStatus::NoSync => ("no_sync".to_string(), None),
		SyncStatus::AwaitingPeers(_) => ("awaiting_peers".to_string(), None),
		SyncStatus::HeaderSync {
			sync_head,
			highest_height,
			..
		} => (
			"header_sync".to_string(),
			Some(json!({ "current_height": sync_head.height, "highest_height": highest_height })),
		),
		SyncStatus::TxHashsetDownload(stats) => (
			"txhashset_download".to_string(),
			Some(
				json!({ "downloaded_size": stats.downloaded_size, "total_size": stats.total_size }),
			),
		),
		SyncStatus::TxHashsetRangeProofsValidation {
			rproofs,
			rproofs_total,
		} => (
			"txhashset_rangeproofs_validation".to_string(),
			Some(json!({ "rproofs": rproofs, "rproofs_total": rproofs_total })),
		),
		SyncStatus::TxHashsetKernelsValidation {
			kernels,
			kernels_total,
		} => (
			"txhashset_kernels_validation".to_string(),
			Some(json!({ "kernels": kernels, "kernels_total": kernels_total })),
		),
		SyncStatus::BodySync {
			current_height,
			highest_height,
		} => (
			"body_sync".to_string(),
			Some(json!({ "current_height": current_height, "highest_height": highest_height })),
		),
		SyncStatus::Shutdown => ("shutdown".to_string(), None),
		// any other status is considered syncing (should be unreachable)
		_ => ("syncing".to_string(), None),
	}
}
