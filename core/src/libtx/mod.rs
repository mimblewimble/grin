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
pub mod serialization;
pub mod slate;

use crate::consensus;
use crate::core::Transaction;

pub use crate::libtx::error::{Error, ErrorKind};

const DEFAULT_BASE_FEE: u64 = consensus::MILLI_GRIN;

/// Transaction fee calculation
pub fn tx_fee(
	input_len: usize,
	output_len: usize,
	kernel_len: usize,
	base_fee: Option<u64>,
) -> u64 {
	let use_base_fee = match base_fee {
		Some(bf) => bf,
		None => DEFAULT_BASE_FEE,
	};

	(Transaction::weight(input_len, output_len, kernel_len) as u64) * use_base_fee
}
