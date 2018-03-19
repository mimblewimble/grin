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

use futures::future;
use futures::{Future, Stream};

use hyper;
use hyper::{Client, Method, Request, StatusCode};
use hyper::header::{ContentLength, ContentType};

use serde::{Deserialize, Serialize};
use serde_json;

use tokio_core::reactor::Core;

use rest::Error;

/// Helper function to easily issue a HTTP GET request against a given URL that
/// returns a JSON object. Handles request building, JSON deserialization and
/// response code checking.
pub fn get<'a, T>(url: &'a str) -> Result<T, Error>
where
	for<'de> T: Deserialize<'de>,
{

    let uri = url.parse::<hyper::Uri>().unwrap();
    let mut core = Core::new().unwrap();
    let client = Client::new(&core.handle());
    let req_f = client.get(uri).map_err(|_| {Error::Internal(format!("Error in request."))});
    let work = req_f.and_then(|res| {
        let status_code = res.status();
        res.body().concat2()
        .map_err(|_| {Error::Internal(format!("Error in request."))})
        .and_then(move |body_chunks| {
            let s = String::from_utf8(body_chunks.to_vec()).unwrap();
            match status_code {
                StatusCode::Ok => future::ok::<_, Error>(s),
                StatusCode::BadRequest => future::err::<_, Error>(Error::Argument(format!("Server returned missing argument error: {}", s))),
                StatusCode::InternalServerError => future::err::<_, Error>(Error::Internal(format!("Server returned internal error: {}", s))),
                StatusCode::NotFound => future::err::<_, Error>(Error::NotFound),
                _ => future::err::<_, Error>(Error::Internal(format!("Server returned undefined internal error"))),
            }
        })
    });
    let json = core.run(work).unwrap();
    serde_json::from_slice(json.as_ref()).map_err(|e| {
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

    let json = serde_json::to_string(input).map_err(|e| {
            Error::Internal(format!("Could not serialize data to JSON: {}", e))
    })?;

    let uri = url.parse::<hyper::Uri>().unwrap();
    let mut req = Request::new(Method::Post, uri);
    req.headers_mut().set(ContentType::json());
    req.headers_mut().set(ContentLength(json.len() as u64));
    req.set_body(json);

    let mut core = Core::new().unwrap();
    let client = Client::new(&core.handle());
    let req_f = client.request(req).map_err(|_| {Error::Internal(format!("Error in request."))});
    let work = req_f.and_then(|res| {
        let status_code = res.status();        
        res.body().concat2()
        .map_err(|_| {Error::Internal(format!("Error in request."))})
        .and_then(move |body_chunks| {
            let s = String::from_utf8(body_chunks.to_vec()).unwrap();
            match status_code {
                StatusCode::Ok => future::ok::<_, Error>(s),
                StatusCode::BadRequest => future::err::<_, Error>(Error::Argument(format!("Server returned missing argument error: {}", s))),
                StatusCode::InternalServerError => future::err::<_, Error>(Error::Internal(format!("Server returned internal error: {}", s))),
                StatusCode::NotFound => future::err::<_, Error>(Error::NotFound),
                _ => future::err::<_, Error>(Error::Internal(format!("Server returned unknown internal error."))),
            }
        })
    });
    let _ = core.run(work).unwrap();
	Ok(())
}
