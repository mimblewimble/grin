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

//! Error types for libwallet
use keychain;
use libtx;
use std::fmt::{self, Display};

use core::core::transaction;
use failure::{Backtrace, Context, Fail};

/// Error definition
#[derive(Debug, Fail)]
pub struct Error {
	inner: Context<ErrorKind>,
}

/// Wallet errors, mostly wrappers around underlying crypto or I/O errors.
#[derive(Clone, Eq, PartialEq, Debug, Fail)]
pub enum ErrorKind {
	/// Not enough funds
	#[fail(display = "Not enough funds. Required: {}, Available: {}", needed, available)]
	NotEnoughFunds {
		/// available funds
		available: u64,
		/// Needed funds
		needed: u64,
	},

	/// Fee dispute
	#[fail(display = "Fee dispute: sender fee {}, recipient fee {}", sender_fee, recipient_fee)]
	FeeDispute {
		/// sender fee
		sender_fee: u64,
		/// recipient fee
		recipient_fee: u64,
	},

	/// Fee Exceeds amount
	#[fail(display = "Fee exceeds amount: sender amount {}, recipient fee {}", sender_amount,
	       recipient_fee)]
	FeeExceedsAmount {
		/// sender amount
		sender_amount: u64,
		/// recipient fee
		recipient_fee: u64,
	},

	/// LibTX Error
	#[fail(display = "LibTx Error")]
	LibTX(libtx::ErrorKind),

	/// Keychain error
	#[fail(display = "Keychain error")]
	Keychain(keychain::Error),

	/// Transaction Error
	#[fail(display = "Transaction error")]
	Transaction(transaction::Error),

	/// Secp Error
	#[fail(display = "Secp error")]
	Secp,

	/// Callback implementation error conversion
	#[fail(display = "Trait Implementation error")]
	CallbackImpl(&'static str),

	/// Callback implementation error conversion
	#[fail(display = "Restore Error")]
	Restore,

	/// An error in the format of the JSON structures exchanged by the wallet
	#[fail(display = "JSON format error")]
	Format,

	/// IO Error
	#[fail(display = "I/O error")]
	IO,

	/// Error when contacting a node through its API
	#[fail(display = "Node API error")]
	Node,

	/// Error contacting wallet API
	#[fail(display = "Wallet Communication Error: {}", _0)]
	WalletComms(String),

	/// Error originating from hyper.
	#[fail(display = "Hyper error")]
	Hyper,

	/// Error originating from hyper uri parsing.
	#[fail(display = "Uri parsing error")]
	Uri,

	/// Signature error
	#[fail(display = "Signature error")]
	Signature(&'static str),

	/// Attempt to use duplicate transaction id in separate transactions
	#[fail(display = "Duplicate transaction ID error")]
	DuplicateTransactionId,

	/// Wallet seed already exists
	#[fail(display = "Wallet seed exists error")]
	WalletSeedExists,

	/// Wallet seed doesn't exist
	#[fail(display = "Wallet seed doesn't exist error")]
	WalletSeedDoesntExist,

	/// Other
	#[fail(display = "Generic error: {}", _0)]
	GenericError(&'static str),
}

impl Display for Error {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		Display::fmt(&self.inner, f)
	}
}

impl Error {
	/// get kind
	pub fn kind(&self) -> ErrorKind {
		self.inner.get_context().clone()
	}
	/// get cause
	pub fn cause(&self) -> Option<&Fail> {
		self.inner.cause()
	}
	/// get backtrace
	pub fn backtrace(&self) -> Option<&Backtrace> {
		self.inner.backtrace()
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

impl From<keychain::Error> for Error {
	fn from(error: keychain::Error) -> Error {
		Error {
			inner: Context::new(ErrorKind::Keychain(error)),
		}
	}
}

impl From<libtx::Error> for Error {
	fn from(error: libtx::Error) -> Error {
		Error {
			inner: Context::new(ErrorKind::LibTX(error.kind())),
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
