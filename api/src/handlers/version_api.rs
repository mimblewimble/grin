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

use crate::core::core::block::HeaderVersion;

use crate::rest::*;
use crate::router::{Handler, ResponseFuture};
use crate::types::Version;
use crate::web::*;
use hyper::{Body, Request};

const CRATE_VERSION: &'static str = env!("CARGO_PKG_VERSION");

/// Version handler. Get running node API version
/// GET /v1/version
pub struct VersionHandler;

impl VersionHandler {
	fn get_version(&self) -> Result<Version, Error> {
		Ok(Version {
			node_version: CRATE_VERSION.to_owned(),
			block_header_version: HeaderVersion::default().into(),
		})
	}
}

impl Handler for VersionHandler {
	fn get(&self, _req: Request<Body>) -> ResponseFuture {
		result_to_response(self.get_version())
	}
}
