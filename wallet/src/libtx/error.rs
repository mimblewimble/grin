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

//! Wallet lib errors

use core::core::transaction;
use keychain::{self, extkey};
use util::secp;

#[derive(Fail, PartialEq, Clone, Debug)]
/// Libwallet error types
pub enum Error {
	/// SECP error
	#[fail(display = "Secp Error")]
	Secp(secp::Error),
	/// Keychain error
	#[fail(display = "Keychain Error")]
	Keychain(keychain::Error),
	/// Extended key error
	#[fail(display = "Extended Key Error")]
	ExtendedKey(extkey::Error),
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
}

impl From<secp::Error> for Error {
	fn from(e: secp::Error) -> Error {
		Error::Secp(e)
	}
}

impl From<extkey::Error> for Error {
	fn from(e: extkey::Error) -> Error {
		Error::ExtendedKey(e)
	}
}

impl From<keychain::Error> for Error {
	fn from(e: keychain::Error) -> Error {
		Error::Keychain(e)
	}
}

impl From<transaction::Error> for Error {
	fn from(e: transaction::Error) -> Error {
		Error::Transaction(e)
	}
}
