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

//! libtx specific errors
use failure::{Backtrace, Context, Fail};
use std::fmt::{self, Display};

use crate::core::{committed, transaction};
use crate::keychain;
use crate::util::secp;

/// Lib tx error definition
#[derive(Debug)]
pub struct Error {
	inner: Context<ErrorKind>,
}

#[derive(Clone, Debug, Eq, Fail, PartialEq)]
/// Libwallet error types
pub enum ErrorKind {
	/// SECP error
	#[fail(display = "Secp Error")]
	Secp(secp::Error),
	/// Keychain error
	#[fail(display = "Keychain Error")]
	Keychain(keychain::Error),
	/// Transaction error
	#[fail(display = "Transaction Error")]
	Transaction(transaction::Error),
	/// Signature error
	#[fail(display = "Signature Error")]
	Signature(String),
	/// Rangeproof error
	#[fail(display = "Rangeproof Error")]
	RangeProof(String),
	/// Fee error
	#[fail(display = "Fee Error")]
	Fee(String),
	/// Error from summing commitments via committed trait.
	#[fail(display = "Committed Error")]
	Committed(committed::Error),
}

impl Fail for Error {
	fn cause(&self) -> Option<&dyn Fail> {
		self.inner.cause()
	}

	fn backtrace(&self) -> Option<&Backtrace> {
		self.inner.backtrace()
	}
}

impl Display for Error {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		Display::fmt(&self.inner, f)
	}
}

impl Error {
	/// Return errorkind
	pub fn kind(&self) -> ErrorKind {
		self.inner.get_context().clone()
	}
}

impl From<ErrorKind> for Error {
	fn from(kind: ErrorKind) -> Error {
		Error {
			inner: Context::new(kind),
		}
	}
}

impl From<Context<ErrorKind>> for Error {
	fn from(inner: Context<ErrorKind>) -> Error {
		Error { inner: inner }
	}
}

impl From<secp::Error> for Error {
	fn from(error: secp::Error) -> Error {
		Error {
			inner: Context::new(ErrorKind::Secp(error)),
		}
	}
}

impl From<committed::Error> for Error {
	fn from(error: committed::Error) -> Error {
		Error {
			inner: Context::new(ErrorKind::Committed(error)),
		}
	}
}

impl From<keychain::Error> for Error {
	fn from(error: keychain::Error) -> Error {
		Error {
			inner: Context::new(ErrorKind::Keychain(error)),
		}
	}
}

impl From<transaction::Error> for Error {
	fn from(error: transaction::Error) -> Error {
		Error {
			inner: Context::new(ErrorKind::Transaction(error)),
		}
	}
}
