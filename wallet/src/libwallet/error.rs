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
use std::io;

use failure::{Backtrace, Context, Fail};

use core;
use core::core::transaction;
use keychain;
use libtx;

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
	#[fail(
		display = "Fee exceeds amount: sender amount {}, recipient fee {}",
		sender_amount,
		recipient_fee
	)]
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

	/// API Error
	#[fail(display = "Client Callback Error: {}", _0)]
	ClientCallback(&'static str),

	/// Secp Error
	#[fail(display = "Secp error")]
	Secp,

	/// Callback implementation error conversion
	#[fail(display = "Trait Implementation error")]
	CallbackImpl(&'static str),

	/// Wallet backend error
	#[fail(display = "Wallet store error")]
	Backend(String),

	/// Callback implementation error conversion
	#[fail(display = "Restore Error")]
	Restore,

	/// An error in the format of the JSON structures exchanged by the wallet
	#[fail(display = "JSON format error")]
	Format,

	/// Other serialization errors
	#[fail(display = "Ser/Deserialization error")]
	Deser(core::ser::Error),

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

	/// Transaction doesn't exist
	#[fail(display = "Transaction {} doesn't exist", _0)]
	TransactionDoesntExist(u32),

	/// Transaction already rolled back
	#[fail(display = "Transaction {} cannot be cancelled", _0)]
	TransactionNotCancellable(u32),

	/// Cancellation error
	#[fail(display = "Cancellation Error: {}", _0)]
	TransactionCancellationError(&'static str),

	/// Cancellation error
	#[fail(display = "Tx dump Error: {}", _0)]
	TransactionDumpError(&'static str),

	/// Attempt to repost a transaction that's already confirmed
	#[fail(display = "Transaction already confirmed error")]
	TransactionAlreadyConfirmed,

	/// Attempt to repost a transaction that's not completed and stored
	#[fail(display = "Transaction building not completed: {}", _0)]
	TransactionBuildingNotCompleted(u32),

	/// Invalid BIP-32 Depth
	#[fail(display = "Invalid BIP32 Depth (must be 1 or greater)")]
	InvalidBIP32Depth,

	/// Attempt to add an account that exists
	#[fail(display = "Account Label '{}' already exists", _0)]
	AccountLabelAlreadyExists(String),

	/// Reference unknown account label
	#[fail(display = "Unknown Account Label '{}'", _0)]
	UnknownAccountLabel(String),

	/// Other
	#[fail(display = "Generic error: {}", _0)]
	GenericError(String),
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

impl From<io::Error> for Error {
	fn from(_error: io::Error) -> Error {
		Error {
			inner: Context::new(ErrorKind::IO),
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

impl From<core::ser::Error> for Error {
	fn from(error: core::ser::Error) -> Error {
		Error {
			inner: Context::new(ErrorKind::Deser(error)),
		}
	}
}
