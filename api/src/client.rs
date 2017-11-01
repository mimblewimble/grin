// Copyright 2016 The Grin Developers
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

//! High level JSON/HTTP client API

use hyper;
use hyper::client::Response;
use hyper::status::{StatusClass, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json;

use rest::Error;

/// Helper function to easily issue a HTTP GET request against a given URL that
/// returns a JSON object. Handles request building, JSON deserialization and
/// response code checking.
pub fn get<'a, T>(url: &'a str) -> Result<T, Error>
where
	for<'de> T: Deserialize<'de>,
{
	let client = hyper::Client::new();
	let res = check_error(client.get(url).send())?;
	serde_json::from_reader(res).map_err(|e| {
		Error::Internal(format!("Server returned invalid JSON: {}", e))
	})
}

/// Helper function to easily issue a HTTP POST request with the provided JSON
/// object as body on a given URL that returns a JSON object. Handles request
/// building, JSON serialization and deserialization, and response code
/// checking.
pub fn post<'a, IN>(url: &'a str, input: &IN) -> Result<(), Error>
where
	IN: Serialize,
{
	let in_json = serde_json::to_string(input).map_err(|e| {
		Error::Internal(format!("Could not serialize data to JSON: {}", e))
	})?;
	let client = hyper::Client::new();
	let _res = check_error(client.post(url).body(&mut in_json.as_bytes()).send())?;
	Ok(())
}

// convert hyper error and check for non success response codes
fn check_error(res: hyper::Result<Response>) -> Result<Response, Error> {
	if let Err(e) = res {
		return Err(Error::Internal(format!("Error during request: {}", e)));
	}
	let response = res.unwrap();
	match response.status.class() {
		StatusClass::Success => Ok(response),
		StatusClass::ServerError => Err(Error::Internal(format!("Server error."))),
		StatusClass::ClientError => if response.status == StatusCode::NotFound {
			Err(Error::NotFound)
		} else {
			Err(Error::Argument(format!("Argument error")))
		},
		_ => Err(Error::Internal(format!("Unrecognized error."))),
	}
}
