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

//! High level JSON/HTTP client API

use failure::{Fail, ResultExt};
use hyper;
use hyper::client::Response;
use hyper::status::{StatusClass, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json;
use std::io::Read;

use rest::{Error, ErrorKind};

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
		e.context(ErrorKind::Internal(
			"Server returned invalid JSON".to_owned(),
		)).into()
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
	let in_json = serde_json::to_string(input).context(ErrorKind::Internal(
		"Could not serialize data to JSON".to_owned(),
	))?;
	let client = hyper::Client::new();
	let _res = check_error(client.post(url).body(&mut in_json.as_bytes()).send())?;
	Ok(())
}

// convert hyper error and check for non success response codes
fn check_error(res: hyper::Result<Response>) -> Result<Response, Error> {
	if let Err(e) = res {
		return Err(
			e.context(ErrorKind::Internal("Error during request".to_owned()))
				.into(),
		);
	}
	let mut response = res.unwrap();
	match response.status.class() {
		StatusClass::Success => Ok(response),
		StatusClass::ServerError => Err(ErrorKind::Internal(format!(
			"Server error: {}",
			err_msg(&mut response)
		)))?,
		StatusClass::ClientError => if response.status == StatusCode::NotFound {
			Err(ErrorKind::NotFound)?
		} else {
			Err(ErrorKind::Argument(format!(
				"Argument error: {}",
				err_msg(&mut response)
			)))?
		},
		_ => Err(ErrorKind::Internal(format!("Unrecognized error.")))?,
	}
}

fn err_msg(resp: &mut Response) -> String {
	let mut msg = String::new();
	if let Err(_) = resp.read_to_string(&mut msg) {
		"<no message>".to_owned()
	} else {
		msg
	}
}
