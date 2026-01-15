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

//! libtx specific errors
use crate::core::transaction;
use util::secp;

/// Lib tx error definition
#[derive(Clone, Debug, Eq, thiserror::Error, PartialEq, Serialize, Deserialize)]
/// Libwallet error types
pub enum Error {
	/// SECP error
	#[error("Secp Error")]
	Secp {
		/// SECP error
		#[from]
		source: secp::Error,
	},
	/// Keychain error
	#[error("Keychain Error")]
	Keychain {
		/// Keychain error
		#[from]
		source: keychain::Error,
	},
	/// Transaction error
	#[error("Transaction Error")]
	Transaction {
		/// Transaction error
		#[from]
		source: transaction::Error,
	},
	/// Signature error
	#[error("Signature Error")]
	Signature(String),
	/// Rangeproof error
	#[error("Rangeproof Error")]
	RangeProof(String),
	/// Other error
	#[error("Other Error")]
	Other(String),
}
