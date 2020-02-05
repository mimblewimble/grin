use crate::rest::*;
use crate::router::ResponseFuture;
use futures::future::{err, ok};
use futures::{Future, Stream};
use hyper::{Body, Request, Response, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json;
use std::collections::HashMap;
use std::fmt::Debug;
use url::form_urlencoded;

/// Parse request body
pub fn parse_body<T>(req: Request<Body>) -> Box<dyn Future<Item = T, Error = Error> + Send>
where
	for<'de> T: Deserialize<'de> + Send + 'static,
{
	Box::new(
		req.into_body()
			.concat2()
			.map_err(|e| ErrorKind::RequestError(format!("Failed to read request: {}", e)).into())
			.and_then(|body| match serde_json::from_reader(&body.to_vec()[..]) {
				Ok(obj) => ok(obj),
				Err(e) => {
					err(ErrorKind::RequestError(format!("Invalid request body: {}", e)).into())
				}
			}),
	)
}

/// Convert Result to ResponseFuture
pub fn result_to_response<T>(res: Result<T, Error>) -> ResponseFuture
where
	T: Serialize,
{
	match res {
		Ok(s) => json_response_pretty(&s),
		Err(e) => match e.kind() {
			ErrorKind::Argument(msg) => response(StatusCode::BAD_REQUEST, msg.clone()),
			ErrorKind::RequestError(msg) => response(StatusCode::BAD_REQUEST, msg.clone()),
			ErrorKind::NotFound => response(StatusCode::NOT_FOUND, ""),
			ErrorKind::Internal(msg) => response(StatusCode::INTERNAL_SERVER_ERROR, msg.clone()),
			ErrorKind::ResponseError(msg) => {
				response(StatusCode::INTERNAL_SERVER_ERROR, msg.clone())
			}
			// place holder
			ErrorKind::Router(_) => response(StatusCode::INTERNAL_SERVER_ERROR, ""),
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
	Box::new(ok(just_response(status, text)))
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
		};
	));

#[macro_export]
macro_rules! must_get_query(
	($req: expr) =>(
		match $req.uri().query() {
			Some(q) => q,
			None => return Err(ErrorKind::RequestError("no query string".to_owned()).into()),
		}
	));

#[macro_export]
macro_rules! parse_param(
	($param: expr, $name: expr, $default: expr) =>(
	match $param.get($name) {
		None => $default,
		Some(val) =>  match val.parse() {
			Ok(val) => val,
			Err(_) => return Err(ErrorKind::RequestError(format!("invalid value of parameter {}", $name)).into()),
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
