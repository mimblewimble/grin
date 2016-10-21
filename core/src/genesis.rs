//! Definition of the genesis block. Placeholder for now.

use time;

use core;

use tiny_keccak::Keccak;

// Genesis block definition. It has no rewards, no inputs, no outputs, no
// fees and a height of zero.
pub fn genesis() -> core::Block {
	let mut sha3 = Keccak::new_sha3_256();
	let mut empty_h = [0; 32];
	sha3.update(&[]);
	sha3.finalize(&mut empty_h);

	core::Block {
		header: core::BlockHeader {
			height: 0,
			previous: core::ZERO_HASH,
			timestamp: time::Tm {
				tm_year: 1997,
				tm_mon: 7,
				tm_mday: 4,
				..time::empty_tm()
			},
			td: 0,
			utxo_merkle: core::Hash::from_vec(empty_h.to_vec()),
			tx_merkle: core::Hash::from_vec(empty_h.to_vec()),
			total_fees: 0,
			nonce: 0,
			pow: core::Proof::zero(), // TODO get actual PoW solution
		},
		inputs: vec![],
		outputs: vec![],
		proofs: vec![],
	}
}
