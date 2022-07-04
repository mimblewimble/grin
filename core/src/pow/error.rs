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

//! Cuckatoo specific errors

/// Cuckatoo solver or validation error
#[derive(Debug, thiserror::Error)]
/// Libwallet error types
pub enum Error {
	/// Verification error
	#[error("Verification Error: {0}")]
	Verification(String),
	/// IO Error
	#[error("IO Error")]
	IOError {
		/// Io Error Convert
		#[from]
		source: std::io::Error,
	},
	/// Unexpected Edge Error
	#[error("Edge Addition Error")]
	EdgeAddition,
	/// Path Error
	#[error("Path Error")]
	Path,
	/// Invalid cycle
	#[error("Invalid Cycle length: {0}")]
	InvalidCycle(usize),
	/// No Cycle
	#[error("No Cycle")]
	NoCycle,
	/// No Solution
	#[error("No Solution")]
	NoSolution,
}
