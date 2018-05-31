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
use std::fmt::{self, Display};

use failure::{Backtrace, Context, Fail};

#[derive(Debug)]
pub struct Error {
	inner: Context<ErrorKind>,
}

/// Wallet errors, mostly wrappers around underlying crypto or I/O errors.
#[derive(Copy, Clone, Eq, PartialEq, Debug, Fail)]
pub enum ErrorKind {
	#[fail(display = "Not enough funds")]
	NotEnoughFunds(u64),

	#[fail(display = "Fee dispute: sender fee {}, recipient fee {}", sender_fee, recipient_fee)]
	FeeDispute { sender_fee: u64, recipient_fee: u64 },

	#[fail(
		display = "Fee exceeds amount: sender amount {}, recipient fee {}",
		sender_amount,
		recipient_fee
	)]
	FeeExceedsAmount {
		sender_amount: u64,
		recipient_fee: u64,
	},

	#[fail(display = "Keychain error")]
	Keychain,

	#[fail(display = "Transaction error")]
	Transaction,

	#[fail(display = "Secp error")]
	Secp,

	#[fail(display = "LibWallet error")]
	LibWalletError,

	#[fail(display = "Wallet data error: {}", _0)]
	FileWallet(&'static str),

	/// An error in the format of the JSON structures exchanged by the wallet
	#[fail(display = "JSON format error")]
	Format,

	#[fail(display = "I/O error")]
	IO,

	/// Error when contacting a node through its API
	#[fail(display = "Node API error")]
	Node,

	/// Error originating from hyper.
	#[fail(display = "Hyper error")]
	Hyper,

	/// Error originating from hyper uri parsing.
	#[fail(display = "Uri parsing error")]
	Uri,

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

	#[fail(display = "Generic error: {}", _0)]
	GenericError(&'static str),
}

impl Fail for Error {
	fn cause(&self) -> Option<&Fail> {
		self.inner.cause()
	}

	fn backtrace(&self) -> Option<&Backtrace> {
		self.inner.backtrace()
	}
}

impl Display for Error {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		Display::fmt(&self.inner, f)
	}
}

impl Error {
	pub fn kind(&self) -> ErrorKind {
		*self.inner.get_context()
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
