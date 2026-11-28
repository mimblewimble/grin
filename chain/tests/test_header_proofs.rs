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

mod chain_test_helper;

use self::chain_test_helper::{clean_output_dir, init_chain, mine_chain};
use grin_chain::Error;
use grin_core::core::hash::{Hashed, ZERO_HASH};
use grin_core::core::header_proof::{self, Error as ProofError, Level, MAX_ITEMS};
use grin_core::core::pmmr::{ReadablePMMR, ReadonlyPMMR, PMMR};
use grin_core::core::BlockHeader;
use grin_core::pow::{Difficulty, Proof, ProofOfWork};
use grin_core::{consensus, genesis, global};
use std::time::Instant;

fn refused(
	chain: &grin_chain::Chain,
	anchor: grin_core::core::hash::Hash,
	heights: &[u64],
	context: usize,
) -> ProofError {
	match chain.header_proofs(anchor, heights, context) {
		Err(Error::HeaderProof(e)) => e,
		other => panic!("expected a header proof error, got {:?}", other),
	}
}

#[test]
fn header_proofs_from_a_mined_chain() {
	let dir = ".grin.header_proofs";
	clean_output_dir(dir);
	let chain = mine_chain(dir, 30);
	let anchor = chain.get_header_by_height(25).unwrap();

	// On AutomatedTesting, version 5 (HF4) starts at height 12.
	let heights = [3, 11, 12, 13, 20, 24];
	let proofs = chain.header_proofs(anchor.hash(), &heights, 2).unwrap();
	assert_eq!(proofs.anchor_height, 25);
	assert_eq!(
		header_proof::verify(&anchor, &proofs).unwrap(),
		vec![
			Level::Pow,
			Level::Pow,
			Level::Exact,
			Level::Exact,
			Level::Exact,
			Level::Exact
		]
	);
	for item in &proofs.items {
		let height = item.header.height;
		assert_eq!(item.context.len(), 2);
		assert_eq!(
			item.context[0],
			chain.get_header_by_height(height - 1).unwrap()
		);
		assert_eq!(
			item.context[1],
			chain.get_header_by_height(height - 2).unwrap()
		);
	}

	let one = chain.header_proofs(anchor.hash(), &heights, 1).unwrap();
	assert_eq!(
		header_proof::verify(&anchor, &one).unwrap(),
		vec![Level::Pow; heights.len()]
	);
	let none = chain.header_proofs(anchor.hash(), &heights, 0).unwrap();
	assert_eq!(
		header_proof::verify(&anchor, &none).unwrap(),
		vec![Level::Solution; heights.len()]
	);

	// Proofs for one anchor do not verify against another.
	let other = chain.get_header_by_height(26).unwrap();
	assert_eq!(
		header_proof::verify(&other, &proofs),
		Err(ProofError::AnchorMismatch)
	);
	clean_output_dir(dir);
}

#[test]
fn header_proof_requests_are_refused_with_a_reason() {
	let dir = ".grin.header_proofs_refused";
	clean_output_dir(dir);
	let chain = mine_chain(dir, 20);
	let anchor = chain.get_header_by_height(15).unwrap().hash();

	assert!(matches!(
		refused(&chain, ZERO_HASH, &[1], 0),
		ProofError::UnknownAnchor(_)
	));
	assert_eq!(
		refused(&chain, anchor, &[15], 0),
		ProofError::NotBelowAnchor(15, 15)
	);
	assert_eq!(
		refused(&chain, anchor, &[5, 4], 0),
		ProofError::NotIncreasing(5, 4)
	);
	assert_eq!(
		refused(&chain, anchor, &[1], 2),
		ProofError::NoRoomForContext(1, 2)
	);
	let many: Vec<u64> = (0..=MAX_ITEMS as u64).collect();
	assert_eq!(
		refused(&chain, anchor, &many, 0),
		ProofError::ItemCount(MAX_ITEMS + 1)
	);
	assert_eq!(
		refused(&chain, anchor, &[10], 61),
		ProofError::ContextTooLarge(61)
	);

	// A header the node knows but that is not on its main header chain.
	let mut side = chain.get_header_by_height(10).unwrap();
	side.pow.proof.nonces[0] += 1;
	{
		let store = chain.store();
		let mut batch = store.batch().unwrap();
		batch.save_block_header(&side).unwrap();
		batch.commit().unwrap();
	}
	assert!(matches!(
		refused(&chain, side.hash(), &[3], 0),
		ProofError::AnchorNotOnMainChain(_)
	));
	clean_output_dir(dir);
}

/// Times proofs on a header chain of `GRIN_HEADER_PROOF_BENCH_HEADERS`
/// synthetic headers (default 100 000) in a real chain store. The headers
/// carry no valid PoW, so only building is timed, not verification.
///
/// `cargo test -p grin_chain --release --test test_header_proofs -- --ignored --nocapture`
#[test]
#[ignore]
fn bench_header_proofs() {
	let count: u64 = std::env::var("GRIN_HEADER_PROOF_BENCH_HEADERS")
		.ok()
		.and_then(|v| v.parse().ok())
		.unwrap_or(100_000);
	let dir = ".grin.header_proofs_bench";
	clean_output_dir(dir);
	global::set_local_chain_type(global::ChainTypes::AutomatedTesting);
	let chain = init_chain(dir, genesis::genesis_dev());

	let built = Instant::now();
	let handle = chain.header_pmmr();
	let mut prev = chain.genesis();
	let store = chain.store();
	let mut batch = store.batch().unwrap();
	for height in 1..count {
		let mut handle = handle.write();
		let prev_root = ReadonlyPMMR::at(&handle.backend, handle.size)
			.root()
			.unwrap();
		let header = BlockHeader {
			height,
			version: consensus::header_version(height),
			timestamp: prev.timestamp + chrono::Duration::seconds(60),
			prev_hash: prev.hash(),
			prev_root,
			pow: ProofOfWork {
				total_difficulty: prev.total_difficulty() + Difficulty::from_num(1),
				// Distinct nonces that fit the edge bits, so hashes differ.
				proof: Proof {
					edge_bits: 32,
					nonces: (0..global::proofsize() as u64)
						.map(|i| height * 64 + i)
						.collect(),
				},
				..Default::default()
			},
			..Default::default()
		};
		batch
			.save_block_header(&header)
			.unwrap_or_else(|e| panic!("saving header {}: {}", height, e));
		let size = handle.size;
		let mut mmr = PMMR::at(&mut handle.backend, size);
		mmr.push(&header).unwrap();
		handle.size = mmr.size;
		prev = header;
		if height % 50 == 0 {
			batch.commit().unwrap();
			batch = store.batch().unwrap();
		}
	}
	batch.commit().unwrap();
	println!("built {} headers in {:?}", count, built.elapsed());

	let anchor = prev.hash();
	let step = (count - 10) / 64;
	let heights: Vec<u64> = (0..64).map(|i| 2 + i * step).collect();
	for round in 0..5 {
		let at = Instant::now();
		let proofs = chain.header_proofs(anchor, &heights, 2).unwrap();
		println!(
			"round {}: 64 heights, context 2, {} headers: {:?}",
			round,
			proofs.items.len() * 3,
			at.elapsed()
		);
	}
	clean_output_dir(dir);
}
