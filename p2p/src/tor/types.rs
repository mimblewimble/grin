// Copyright 2026 The Grin Developers
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

use ed25519_dalek::hazmat::ExpandedSecretKey;

#[derive(Debug, thiserror::Error)]
pub enum Error {
	/// Generic Tor error
	#[error("Tor Process Error: {0}")]
	Generic(String),
	/// Tor configuration error
	#[error("Tor Config Error: {0}")]
	Config(String),
	/// POST HTTP request error.
	#[error("Tor POST Error: {0}")]
	Post(PostError),
}

/// Failure from a POST HTTP request over Tor, classified by whether delivery may
/// have started.
#[derive(Debug, thiserror::Error)]
pub enum PostError {
	/// The failure happened before handing the request to Hyper.
	#[error("Not sent: {0}")]
	NotSent(String),
	/// Hyper started sending the request, so delivery cannot be ruled out.
	#[error("Possibly sent: {0}")]
	PossiblySent(String),
}

/// Convert expanded secret key to byte array (works for both external Tor process and Arti).
pub(crate) fn exp_sec_key_bytes(expanded_sk: ExpandedSecretKey) -> [u8; 64] {
	let mut sk_bytes = [0_u8; 64];
	sk_bytes[0..32].copy_from_slice(&expanded_sk.scalar.to_bytes());
	sk_bytes[32..64].copy_from_slice(&expanded_sk.hash_prefix);
	sk_bytes
}
