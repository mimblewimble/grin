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

//! Simple implementation of the siphash 2-4 hashing function from
//! Jean-Philippe Aumasson and Daniel J. Bernstein.

/// Implements siphash 2-4 specialized for a 4 u64 array key and a u64 nonce
pub fn siphash24(v: &[u64; 4], nonce: u64) -> u64 {
	let mut v0 = v[0];
	let mut v1 = v[1];
	let mut v2 = v[2];
	let mut v3 = v[3] ^ nonce;

	// macro for left rotation
	macro_rules! rotl {
		($num:ident, $shift:expr) => {
			$num = ($num << $shift) | ($num >> (64 - $shift));
		};
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
		};
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

	v0 ^ v1 ^ v2 ^ v3
}

/// Computes a block of siphash hashes by repeatedly hashing an initial offset
/// to obtain `block_size` hashes.
pub fn siphash_serial(block_size: usize, v: &[u64; 4], offset: u64) -> Vec<u64> {
	// initial state before we start inserting hashes
	let mut state = v.to_vec();
	state[0] = state[0] ^ offset;

	// iteratively compute additional hashes from past state
	for round in 0..block_size {
		let l = state.len();
		let prev = [state[l-4], state[l-3], state[l-2], state[l-1]];
		state.push(siphash24(&prev, round as u64));
	}

	// remove initial state and xor each state with previous
	state.pop(); state.pop(); state.pop(); state.pop(); 
	let last_state = state[state.len()-1];
	for n in 0..block_size-1 {
		state[n] = state[n+1] ^ last_state;
	}
	state[block_size - 1] = last_state ^ 0;
	return state;
}

#[cfg(test)]
mod test {
	use super::*;

	/// Some test vectors hoisted from the Java implementation (adjusted from
	/// the fact that the Java impl uses a long, aka a signed 64 bits number).
	#[test]
	fn hash_some() {
		assert_eq!(siphash24(&[1, 2, 3, 4], 10), 928382149599306901);
		assert_eq!(siphash24(&[1, 2, 3, 4], 111), 10524991083049122233);
		assert_eq!(siphash24(&[9, 7, 6, 7], 12), 1305683875471634734);
		assert_eq!(siphash24(&[9, 7, 6, 7], 10), 11589833042187638814);
	}

	#[test]
	fn test_siphash_serial() {
		let seed = [0; 4];
		let state = siphash_serial(64, &seed, 10);
		assert_eq!(state.len(), 64);
	}
}
