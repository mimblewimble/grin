// Copyright 2019 The Grin Developers
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

//! JSON-RPC Stub generation for the Node API

use crate::node::Node;
use crate::rest::ErrorKind;
use crate::types::Status;

/// Public definition used to generate Node jsonrpc api.
/// * When running `grin` with defaults, the V2 api is available at
/// `localhost:3413/v2`
/// * The endpoint only supports POST operations, with the json-rpc request as the body
#[easy_jsonrpc_mw::rpc]
pub trait NodeRpc: Sync + Send {
	fn get_status(&self) -> Result<Status, ErrorKind>;
}

impl NodeRpc for Node {
	fn get_status(&self) -> Result<Status, ErrorKind> {
		Node::get_status(self).map_err(|e| e.kind().clone())
	}
}
