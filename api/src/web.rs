use futures::future::{err, ok};
use futures::{Future, Stream};
use hyper::{Body, Request, Response, StatusCode};
use rest::*;
use router::ResponseFuture;
use serde::{Deserialize, Serialize};
use serde_json;
use std::fmt::Debug;

/// Parse request body
pub fn parse_body<T>(req: Request<Body>) -> Box<Future<Item = T, Error = Error> + Send>
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
