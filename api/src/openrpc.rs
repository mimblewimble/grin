// Copyright 2026 The Grin Developers
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

//! OpenRPC service discovery (`rpc.discover`) for the v2 JSON-RPC APIs.

use easy_jsonrpc_mw::{Error, Handler, Params, Value};

/// OpenRPC document for the owner API (`/v2/owner`).
pub const OWNER_SPEC: &str = include_str!("../openrpc/owner.json");
/// OpenRPC document for the foreign API (`/v2/foreign`).
pub const FOREIGN_SPEC: &str = include_str!("../openrpc/foreign.json");

/// Wraps an RPC handler and answers `rpc.discover` with an OpenRPC document.
/// The rpc macro rejects `rpc.`-prefixed method names, so this is done outside the trait.
pub struct Discoverable<'a, H: ?Sized> {
	inner: &'a H,
	spec: &'static str,
}

impl<'a, H: ?Sized> Discoverable<'a, H> {
	/// Serve `spec` for `rpc.discover` and pass every other call to `inner`.
	pub fn new(inner: &'a H, spec: &'static str) -> Self {
		Discoverable { inner, spec }
	}
}

impl<'a, H: Handler + ?Sized> Handler for Discoverable<'a, H> {
	fn handle(&self, method: &str, params: Params) -> Result<Value, Error> {
		if method == "rpc.discover" {
			return serde_json::from_str(self.spec).map_err(|_| Error::internal_error());
		}
		self.inner.handle(method, params)
	}
}

#[cfg(test)]
mod test {
	use super::*;
	use crate::owner::Owner;
	use crate::owner_rpc::OwnerRpc;
	use easy_jsonrpc_mw::MaybeReply;
	use regex::Regex;
	use serde_json::json;
	use std::sync::Weak;

	fn call(handler: &dyn Handler, req: Value) -> Value {
		match handler.handle_request(req) {
			MaybeReply::Reply(r) => r,
			MaybeReply::DontReply => panic!("no reply"),
		}
	}

	fn owner() -> Owner {
		Owner::new(Weak::new(), Weak::new(), Weak::new())
	}

	#[test]
	fn discover_returns_spec() {
		let api = owner();
		let wrapped = Discoverable::new(&api as &dyn OwnerRpc, OWNER_SPEC);
		let res = call(
			&wrapped,
			json!({"jsonrpc": "2.0", "method": "rpc.discover", "id": 1}),
		);
		let spec: Value = serde_json::from_str(OWNER_SPEC).unwrap();
		assert_eq!(res["result"], spec);
	}

	#[test]
	fn discover_in_batch_and_delegation() {
		let api = owner();
		let wrapped = Discoverable::new(&api as &dyn OwnerRpc, OWNER_SPEC);
		let res = call(
			&wrapped,
			json!([
				{"jsonrpc": "2.0", "method": "rpc.discover", "id": 1},
				{"jsonrpc": "2.0", "method": "get_status", "params": [], "id": 2},
				{"jsonrpc": "2.0", "method": "no_such_method", "id": 3}
			]),
		);
		let by_id = |id: u64| {
			res.as_array()
				.unwrap()
				.iter()
				.find(|r| r["id"] == id)
				.unwrap()
				.clone()
		};
		assert_eq!(by_id(1)["result"]["openrpc"], "1.2.6");
		// No chain behind the Weak refs, so the call reaches Owner and fails there.
		assert!(by_id(2)["result"]["Err"].is_object());
		assert_eq!(by_id(3)["error"]["code"], -32601);
	}

	/// (method, [(param, is_option)]) for each fn in `trait_name`, parsed from its source.
	fn trait_methods(src: &str, trait_name: &str) -> Vec<(String, Vec<(String, bool)>)> {
		let start = src.find(&format!("pub trait {}", trait_name)).unwrap();
		let body = &src[start..];
		let body = &body[..body.find("\n}\n").unwrap()];
		let body = Regex::new(r"(?s)/\*\*.*?\*/")
			.unwrap()
			.replace_all(body, "");
		let fns = Regex::new(r"(?s)fn\s+(\w+)\s*\((.*?)\)\s*->").unwrap();
		let arg = Regex::new(r"(\w+)\s*:\s*(\S.*)").unwrap();
		fns.captures_iter(&body)
			.map(|c| {
				let args = c[2]
					.split(',')
					.filter_map(|a| arg.captures(a.trim()))
					.map(|a| (a[1].to_string(), a[2].trim().starts_with("Option<")))
					.collect();
				(c[1].to_string(), args)
			})
			.collect()
	}

	fn spec_methods(spec: &str) -> Vec<(String, Vec<(String, bool)>)> {
		let spec: Value = serde_json::from_str(spec).unwrap();
		spec["methods"]
			.as_array()
			.unwrap()
			.iter()
			.map(|m| {
				let params = m["params"]
					.as_array()
					.unwrap()
					.iter()
					.map(|p| {
						let nullable = p["schema"]["oneOf"]
							.as_array()
							.is_some_and(|alts| alts.contains(&json!({"type": "null"})));
						(p["name"].as_str().unwrap().to_string(), nullable)
					})
					.collect();
				(m["name"].as_str().unwrap().to_string(), params)
			})
			.collect()
	}

	#[test]
	fn owner_spec_matches_trait() {
		assert_eq!(
			spec_methods(OWNER_SPEC),
			trait_methods(include_str!("owner_rpc.rs"), "OwnerRpc")
		);
	}

	#[test]
	fn foreign_spec_matches_trait() {
		assert_eq!(
			spec_methods(FOREIGN_SPEC),
			trait_methods(include_str!("foreign_rpc.rs"), "ForeignRpc")
		);
	}
}
