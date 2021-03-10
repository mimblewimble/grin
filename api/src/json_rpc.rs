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
// Derived from https://github.com/apoelstra/rust-jsonrpc

//! JSON RPC Client functionality
use std::{error, fmt};

use serde::{Deserialize, Serialize};

/// Builds a request
pub fn build_request<'a, 'b>(name: &'a str, params: &'b serde_json::Value) -> Request<'a, 'b> {
	Request {
		method: name,
		params: params,
		id: From::from(1),
		jsonrpc: Some("2.0"),
	}
}

#[derive(Debug, Clone, PartialEq, Serialize)]
/// A JSONRPC request object
pub struct Request<'a, 'b> {
	/// The name of the RPC call
	pub method: &'a str,
	/// Parameters to the RPC call
	pub params: &'b serde_json::Value,
	/// Identifier for this Request, which should appear in the response
	pub id: serde_json::Value,
	/// jsonrpc field, MUST be "2.0"
	pub jsonrpc: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
/// A JSONRPC response object
pub struct Response {
	/// A result if there is one, or null
	pub result: Option<serde_json::Value>,
	/// An error if there is one, or null
	pub error: Option<RpcError>,
	/// Identifier for this Request, which should match that of the request
	pub id: serde_json::Value,
	/// jsonrpc field, MUST be "2.0"
	pub jsonrpc: Option<String>,
}

impl Response {
	/// Extract the result from a response
	pub fn result<T: serde::de::DeserializeOwned>(&self) -> Result<T, Error> {
		if let Some(ref e) = self.error {
			return Err(Error::Rpc(e.clone()));
		}

		let result = match self.result.clone() {
			Some(r) => serde_json::from_value(r["Ok"].clone()).map_err(Error::Json),
			None => serde_json::from_value(serde_json::Value::Null).map_err(Error::Json),
		}?;
		Ok(result)
	}

	/// Extract the result from a response, consuming the response
	pub fn into_result<T: serde::de::DeserializeOwned>(self) -> Result<T, Error> {
		if let Some(e) = self.error {
			return Err(Error::Rpc(e));
		}
		self.result()
	}

	/// Return the RPC error, if there was one, but do not check the result
	pub fn _check_error(self) -> Result<(), Error> {
		if let Some(e) = self.error {
			Err(Error::Rpc(e))
		} else {
			Ok(())
		}
	}

	/// Returns whether or not the `result` field is empty
	pub fn _is_none(&self) -> bool {
		self.result.is_none()
	}
}

/// A library error
#[derive(Debug)]
pub enum Error {
	/// Json error
	Json(serde_json::Error),
	/// Client error
	Hyper(hyper::error::Error),
	/// Error response
	Rpc(RpcError),
	/// Response to a request did not have the expected nonce
	_NonceMismatch,
	/// Response to a request had a jsonrpc field other than "2.0"
	_VersionMismatch,
	/// Batches can't be empty
	_EmptyBatch,
	/// Too many responses returned in batch
	_WrongBatchResponseSize,
	/// Batch response contained a duplicate ID
	_BatchDuplicateResponseId(serde_json::Value),
	/// Batch response contained an ID that didn't correspond to any request ID
	_WrongBatchResponseId(serde_json::Value),
}

impl From<serde_json::Error> for Error {
	fn from(e: serde_json::Error) -> Error {
		Error::Json(e)
	}
}

impl From<hyper::error::Error> for Error {
	fn from(e: hyper::error::Error) -> Error {
		Error::Hyper(e)
	}
}

impl From<RpcError> for Error {
	fn from(e: RpcError) -> Error {
		Error::Rpc(e)
	}
}

impl fmt::Display for Error {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		match *self {
			Error::Json(ref e) => write!(f, "JSON decode error: {}", e),
			Error::Hyper(ref e) => write!(f, "Hyper error: {}", e),
			Error::Rpc(ref r) => write!(f, "RPC error response: {:?}", r),
			Error::_BatchDuplicateResponseId(ref v) => {
				write!(f, "duplicate RPC batch response ID: {}", v)
			}
			Error::_WrongBatchResponseId(ref v) => write!(f, "wrong RPC batch response ID: {}", v),
			_ => write!(f, "{}", self),
		}
	}
}

impl std::error::Error for Error {
	fn description(&self) -> &str {
		match *self {
			Error::Json(_) => "JSON decode error",
			Error::Hyper(_) => "Hyper error",
			Error::Rpc(_) => "RPC error response",
			Error::_NonceMismatch => "Nonce of response did not match nonce of request",
			Error::_VersionMismatch => "`jsonrpc` field set to non-\"2.0\"",
			Error::_EmptyBatch => "batches can't be empty",
			Error::_WrongBatchResponseSize => "too many responses returned in batch",
			Error::_BatchDuplicateResponseId(_) => "batch response contained a duplicate ID",
			Error::_WrongBatchResponseId(_) => {
				"batch response contained an ID that didn't correspond to any request ID"
			}
		}
	}

	fn cause(&self) -> Option<&dyn error::Error> {
		match *self {
			Error::Json(ref e) => Some(e),
			Error::Hyper(ref e) => Some(e),
			_ => None,
		}
	}
}

/// Standard error responses, as described at at
/// http://www.jsonrpc.org/specification#error_object
///
/// # Documentation Copyright
/// Copyright (C) 2007-2010 by the JSON-RPC Working Group
///
/// This document and translations of it may be used to implement JSON-RPC, it
/// may be copied and furnished to others, and derivative works that comment
/// on or otherwise explain it or assist in its implementation may be prepared,
/// copied, published and distributed, in whole or in part, without restriction
/// of any kind, provided that the above copyright notice and this paragraph
/// are included on all such copies and derivative works. However, this document
/// itself may not be modified in any way.
///
/// The limited permissions granted above are perpetual and will not be revoked.
///
/// This document and the information contained herein is provided "AS IS" and
/// ALL WARRANTIES, EXPRESS OR IMPLIED are DISCLAIMED, INCLUDING BUT NOT LIMITED
/// TO ANY WARRANTY THAT THE USE OF THE INFORMATION HEREIN WILL NOT INFRINGE ANY
/// RIGHTS OR ANY IMPLIED WARRANTIES OF MERCHANTABILITY OR FITNESS FOR A
/// PARTICULAR PURPOSE.
///
#[allow(dead_code)]
#[derive(Debug)]
pub enum StandardError {
	/// Invalid JSON was received by the server.
	/// An error occurred on the server while parsing the JSON text.
	ParseError,
	/// The JSON sent is not a valid Request object.
	InvalidRequest,
	/// The method does not exist / is not available.
	MethodNotFound,
	/// Invalid method parameter(s).
	InvalidParams,
	/// Internal JSON-RPC error.
	InternalError,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
/// A JSONRPC error object
pub struct RpcError {
	/// The integer identifier of the error
	pub code: i32,
	/// A string describing the error
	pub message: String,
	/// Additional data specific to the error
	pub data: Option<serde_json::Value>,
}

/// Create a standard error responses
pub fn _standard_error(code: StandardError, data: Option<serde_json::Value>) -> RpcError {
	match code {
		StandardError::ParseError => RpcError {
			code: -32700,
			message: "Parse error".to_string(),
			data: data,
		},
		StandardError::InvalidRequest => RpcError {
			code: -32600,
			message: "Invalid Request".to_string(),
			data: data,
		},
		StandardError::MethodNotFound => RpcError {
			code: -32601,
			message: "Method not found".to_string(),
			data: data,
		},
		StandardError::InvalidParams => RpcError {
			code: -32602,
			message: "Invalid params".to_string(),
			data: data,
		},
		StandardError::InternalError => RpcError {
			code: -32603,
			message: "Internal error".to_string(),
			data: data,
		},
	}
}

/// Converts a Rust `Result` to a JSONRPC response object
pub fn _result_to_response(
	result: Result<serde_json::Value, RpcError>,
	id: serde_json::Value,
) -> Response {
	match result {
		Ok(data) => Response {
			result: Some(data),
			error: None,
			id: id,
			jsonrpc: Some(String::from("2.0")),
		},
		Err(err) => Response {
			result: None,
			error: Some(err),
			id: id,
			jsonrpc: Some(String::from("2.0")),
		},
	}
}
