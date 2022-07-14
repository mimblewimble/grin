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

//! Library containing lower-level transaction building functions needed by
//! all wallets.

#![deny(non_upper_case_globals)]
#![deny(non_camel_case_types)]
#![deny(non_snake_case)]
#![deny(unused_mut)]
#![warn(missing_docs)]

pub mod aggsig;
pub mod build;
mod error;
pub mod proof;
pub mod reward;
pub mod secp_ser;

use crate::core::Transaction;
use crate::global::get_accept_fee_base;

pub use self::proof::ProofBuilder;
pub use crate::libtx::error::Error;

/// Transaction fee calculation given numbers of inputs, outputs, and kernels
pub fn tx_fee(input_len: usize, output_len: usize, kernel_len: usize) -> u64 {
	Transaction::weight_by_iok(input_len as u64, output_len as u64, kernel_len as u64)
		* get_accept_fee_base()
}

/// Transaction fee calculation given transaction
pub fn accept_fee(tx: Transaction) -> u64 {
	tx.accept_fee()
}
