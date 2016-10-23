// Copyright 2016 The Grin Developers
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

//! Binary stream serialization and deserialzation for core types from trusted
//! Write or Read implementations. Issues like starvation or too big sends are
//! expected to be handled upstream.

use time;

use std::io::{Write, Read};
use core::{self, hash};
use ser::*;

use secp::Signature;
use secp::key::SecretKey;
use secp::pedersen::{Commitment, RangeProof};

macro_rules! impl_slice_bytes {
  ($byteable: ty) => {
    impl AsFixedBytes for $byteable {
      fn as_fixed_bytes(&self) -> &[u8] {
        &self[..]
      }
    }
  }
}

impl_slice_bytes!(SecretKey);
impl_slice_bytes!(Signature);
impl_slice_bytes!(Commitment);
impl_slice_bytes!(Vec<u8>);

impl AsFixedBytes for hash::Hash {
	fn as_fixed_bytes(&self) -> &[u8] {
		self.to_slice()
	}
}

impl AsFixedBytes for RangeProof {
	fn as_fixed_bytes(&self) -> &[u8] {
		&self.bytes()
	}
}

