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

//! Simple implementation of the siphash 2-4 hashing function from
//! Jean-Philippe Aumasson and Daniel J. Bernstein.

/// Implements siphash 2-4 specialized for a 4 u64 array key and a u64 nonce
pub fn siphash24(v: [u64; 4], nonce: u64) -> u64 {
	let mut v0 = v[0];
	let mut v1 = v[1];
	let mut v2 = v[2];
	let mut v3 = v[3] ^ nonce;

	// macro for left rotation
	macro_rules! rotl {
    ($num:ident, $shift:expr) => {
      $num = ($num << $shift) | ($num >> (64 - $shift));
    }
  }

	// macro for a single siphash round
	macro_rules! round {
    () => {
      v0 = v0.wrapping_add(v1);
      v2 = v2.wrapping_add(v3);
      rotl!(v1, 13);
      rotl!(v3, 16);
      v1 ^= v0;
      v3 ^= v2;
      rotl!(v0, 32);
      v2 = v2.wrapping_add(v1);
      v0 = v0.wrapping_add(v3);
      rotl!(v1, 17);
      rotl!(v3, 21);
      v1 ^= v2;
      v3 ^= v0;
      rotl!(v2, 32);
    }
  }

	// 2 rounds
	round!();
	round!();

	v0 ^= nonce;
	v2 ^= 0xff;

	// and then 4 rounds, hence siphash 2-4
	round!();
	round!();
	round!();
	round!();

	return v0 ^ v1 ^ v2 ^ v3;
}

#[cfg(test)]
mod test {
	use super::*;

	/// Some test vectors hoisted from the Java implementation (adjusted from
	/// the fact that the Java impl uses a long, aka a signed 64 bits number).
	#[test]
	fn hash_some() {
		assert_eq!(siphash24([1, 2, 3, 4], 10), 928382149599306901);
		assert_eq!(siphash24([1, 2, 3, 4], 111), 10524991083049122233);
		assert_eq!(siphash24([9, 7, 6, 7], 12), 1305683875471634734);
		assert_eq!(siphash24([9, 7, 6, 7], 10), 11589833042187638814);
	}
}
