use crate::rest::*;
use crate::router::ResponseFuture;
use bytes::Buf;
use futures::future::ok;
use hyper::body;
use hyper::{Body, Request, Response, StatusCode};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::Debug;
use url::form_urlencoded;

/// Parse request body
pub async fn parse_body<T>(req: Request<Body>) -> Result<T, Error>
where
	for<'de> T: Deserialize<'de> + Send + 'static,
{
	let raw = body::to_bytes(req.into_body())
		.await
		.map_err(|e| Error::RequestError(format!("Failed to read request: {}", e)))?;

	serde_json::from_reader(raw.bytes())
		.map_err(|e| Error::RequestError(format!("Invalid request body: {}", e)))
}

/// Convert Result to ResponseFuture
pub fn result_to_response<T>(res: Result<T, Error>) -> ResponseFuture
where
	T: Serialize,
{
	match res {
		Ok(s) => json_response_pretty(&s),
		Err(e) => match e {
			Error::Argument(msg) => response(StatusCode::BAD_REQUEST, msg.clone()),
			Error::RequestError(msg) => response(StatusCode::BAD_REQUEST, msg.clone()),
			Error::NotFound => response(StatusCode::NOT_FOUND, ""),
			Error::Internal(msg) => response(StatusCode::INTERNAL_SERVER_ERROR, msg.clone()),
			Error::ResponseError(msg) => response(StatusCode::INTERNAL_SERVER_ERROR, msg.clone()),
			// place holder
			Error::Router { .. } => response(StatusCode::INTERNAL_SERVER_ERROR, ""),
		},
	}
}

/// Utility to serialize a struct into JSON and produce a sensible Response
/// out of it.
pub fn json_response<T>(s: &T) -> ResponseFuture
where
	T: Serialize,
{
	match serde_json::to_string(s) {
		Ok(json) => response(StatusCode::OK, json),
		Err(_) => response(StatusCode::INTERNAL_SERVER_ERROR, ""),
	}
}

/// Pretty-printed version of json response as future
pub fn json_response_pretty<T>(s: &T) -> ResponseFuture
where
	T: Serialize,
{
	match serde_json::to_string_pretty(s) {
		Ok(json) => response(StatusCode::OK, json),
		Err(e) => response(
			StatusCode::INTERNAL_SERVER_ERROR,
			format!("can't create json response: {}", e),
		),
	}
}

/// Text response as HTTP response
pub fn just_response<T: Into<Body> + Debug>(status: StatusCode, text: T) -> Response<Body> {
	let mut resp = Response::new(text.into());
	*resp.status_mut() = status;
	resp
}

/// Text response as future
pub fn response<T: Into<Body> + Debug>(status: StatusCode, text: T) -> ResponseFuture {
	Box::pin(ok(just_response(status, text)))
}

pub struct QueryParams {
	params: HashMap<String, Vec<String>>,
}

impl QueryParams {
	pub fn process_multival_param<F>(&self, name: &str, mut f: F)
	where
		F: FnMut(&str),
	{
		if let Some(ids) = self.params.get(name) {
			for id in ids {
				for id in id.split(',') {
					f(id);
				}
			}
		}
	}

	pub fn get(&self, name: &str) -> Option<&String> {
		self.params.get(name).and_then(|v| v.first())
	}
}

impl From<&str> for QueryParams {
	fn from(query_string: &str) -> Self {
		let params = form_urlencoded::parse(query_string.as_bytes())
			.into_owned()
			.fold(HashMap::new(), |mut hm, (k, v)| {
				hm.entry(k).or_insert_with(|| vec![]).push(v);
				hm
			});
		QueryParams { params }
	}
}

impl From<Option<&str>> for QueryParams {
	fn from(query_string: Option<&str>) -> Self {
		match query_string {
			Some(query_string) => Self::from(query_string),
			None => QueryParams {
				params: HashMap::new(),
			},
		}
	}
}

impl From<Request<Body>> for QueryParams {
	fn from(req: Request<Body>) -> Self {
		Self::from(req.uri().query())
	}
}

#[macro_export]
macro_rules! right_path_element(
	($req: expr) =>(
		match $req.uri().path().trim_end_matches('/').rsplit('/').next() {
			None => return response(StatusCode::BAD_REQUEST, "invalid url"),
			Some(el) => el,
		}
	));

#[macro_export]
macro_rules! must_get_query(
	($req: expr) =>(
		match $req.uri().query() {
			Some(q) => q,
			None => return Err(Error::RequestError("no query string".to_owned())),
		}
	));

#[macro_export]
macro_rules! parse_param(
	($param: expr, $name: expr, $default: expr) =>(
	match $param.get($name) {
		None => $default,
		Some(val) =>  match val.parse() {
			Ok(val) => val,
			Err(_) => return Err(Error::RequestError(format!("invalid value of parameter {}", $name))),
		}
	}
	));

#[macro_export]
macro_rules! parse_param_no_err(
	($param: expr, $name: expr, $default: expr) =>(
	match $param.get($name) {
		None => $default,
		Some(val) =>  match val.parse() {
			Ok(val) => val,
			Err(_) => $default,
		}
	}
	));

#[macro_export]
macro_rules! w_fut(
	($p: expr) =>(
		match w($p) {
			Ok(p) => p,
			Err(_) => return response(StatusCode::INTERNAL_SERVER_ERROR, "weak reference upgrade failed" ),
		}
	));
