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

//! Historical header MMR inclusion proofs (HEADER_PROOF_HIST, protocol v1).
//!
//! A header proof binds a full block header at height `H` to the header MMR
//! prefix committed to by `anchor.prev_root`, using the historical MMR size
//! `insertion_to_pmmr_index(anchor.height)`. The header MMR leaf is
//! `H(be64(pos) || packed PoW nonces)` (implicit in the PMMR hashing rules),
//! where `pos = insertion_to_pmmr_index(H)`.
//!
//! # Security caveats
//!
//! * A proof authenticates membership in the anchor's MMR only. It does NOT
//!   prove that the anchor is the highest-work chain; anchor selection and
//!   eclipse defense are a separate problem.
//! * PoW/difficulty validation of the returned headers is the client's job
//!   (post-HF4 WTEMA needs the surrounding headers).
//! * Header proofs are not UTXO/state proofs; PIBD/state proofs are still
//!   required for spend validation.
//! * Until the sampling/fraud analysis is done, trusted checkpoint mode must
//!   not be removed.

use std::collections::HashMap;

use crate::core::consensus;
use crate::core::core::hash::{Hash, Hashed};
use crate::core::core::merkle_proof::MerkleProof;
use crate::core::core::pmmr::{self, Backend, ReadablePMMR, ReadonlyPMMR};
use crate::core::core::{BlockHeader, HeaderVersion};
use crate::core::pow;
use crate::msg::{HeaderProofItem, HeaderProofs};
use crate::types::{MAX_HEADER_PROOF_ITEMS, MAX_HEADER_PROOF_PATH};

/// Historical header MMR size for the anchor at `height`.
///
/// This is the size of the header MMR prefix after the first `height` leaves,
/// which is exactly what the anchor header commits to in `prev_root`.
pub fn header_mmr_size(height: u64) -> u64 {
	pmmr::insertion_to_pmmr_index(height)
}

/// MMR leaf position for the header at `height`.
pub fn header_leaf_pos(height: u64) -> u64 {
	pmmr::insertion_to_pmmr_index(height)
}

/// Verifies one header proof item against the anchor `prev_root`.
///
/// This is the client-side (and fail-closed server-side) check:
/// `header.height` must match the requested height, the leaf position is
/// derived from the height, and the Merkle path must verify against `root`
/// with the historical `mmr_size`.
pub fn verify_header_proof_item(
	root: Hash,
	mmr_size: u64,
	height: u64,
	header: &BlockHeader,
	path: &[Hash],
) -> Result<(), String> {
	if header.height != height {
		return Err(format!(
			"header height {} does not match requested height {}",
			header.height, height
		));
	}
	if path.len() > MAX_HEADER_PROOF_PATH as usize {
		return Err(format!(
			"proof path length {} exceeds max {}",
			path.len(),
			MAX_HEADER_PROOF_PATH
		));
	}
	let pos = header_leaf_pos(height);
	if pos >= mmr_size {
		return Err(format!(
			"header leaf position {} outside historical MMR size {}",
			pos, mmr_size
		));
	}
	if mmr_size != pmmr::insertion_to_pmmr_index(pmmr::n_leaves(mmr_size)) {
		return Err(format!("invalid historical MMR size {}", mmr_size));
	}
	let proof = MerkleProof {
		mmr_size,
		path: path.to_vec(),
	};
	proof
		.verify(root, header, pos)
		.map_err(|_| format!("header proof root mismatch at height {}", height))
}

/// Verifies the MMR *membership* part of a full `HeaderProofs` response.
///
/// NOTE: the header MMR leaf commits only to `(leaf pos, packed PoW nonces)`.
/// This function therefore does NOT bind header fields such as `timestamp`,
/// `output_root`, MMR sizes or `total_difficulty`. Use
/// [`verify_header_proofs_with_pow`] before trusting those fields.
///
/// The caller remains responsible for deciding that the anchor itself is
/// acceptable (chain work / checkpoint policy).
pub fn verify_memberships(anchor: &BlockHeader, proofs: &HeaderProofs) -> Result<(), String> {
	if anchor.hash() != proofs.anchor {
		return Err("header proofs anchor does not match the anchor header".to_string());
	}
	let expected_size = header_mmr_size(anchor.height);
	if proofs.mmr_size != expected_size {
		return Err(format!(
			"header proofs mmr_size {} does not match anchor height {} (expected {})",
			proofs.mmr_size, anchor.height, expected_size
		));
	}
	if proofs.items.is_empty() || proofs.items.len() > MAX_HEADER_PROOF_ITEMS as usize {
		return Err(format!(
			"header proofs item count {} outside 1..={}",
			proofs.items.len(),
			MAX_HEADER_PROOF_ITEMS
		));
	}
	let mut prev_height: Option<u64> = None;
	for item in &proofs.items {
		let height = item.header.height;
		if height >= anchor.height {
			return Err(format!(
				"header proof height {} is not below anchor height {}",
				height, anchor.height
			));
		}
		if let Some(prev) = prev_height {
			if height <= prev {
				return Err(format!(
					"header proof heights are not strictly increasing ({} after {})",
					height, prev
				));
			}
		}
		prev_height = Some(height);
		verify_header_proof_item(
			anchor.prev_root,
			proofs.mmr_size,
			height,
			&item.header,
			&item.path,
		)?;
	}
	Ok(())
}

/// Verifies a full `HeaderProofs` response **with full header PoW + difficulty
/// binding**.
///
/// [`verify_memberships`] only proves that `(leaf pos, packed PoW nonces)` are
/// part of the anchor's header MMR. To safely accept the other header fields
/// (timestamp, output/range-proof/kernel roots, MMR sizes, total_difficulty)
/// the client must also validate each header's proof of work and the claimed
/// difficulty, because the PoW pre-image commits to those fields.
///
/// For **every sampled height `H`** this function requires the headers
/// `H-2, H-1, H` to be present in the response proof set and/or the explicit
/// `trusted_tail` (a client-supplied, trusted contiguous chain tail). It then
/// checks, for post-HF4 WTEMA rules (`H.version >= 5`):
/// * membership proofs ([`verify_memberships`]);
/// * header version is valid for its height;
/// * `H` carries an actual primary PoW solution (`pow::verify_size`), which
///   binds every pre-PoW header field including timestamp/output_root;
/// * `prev_hash` linkage `H -> H-1 -> H-2`; strictly increasing timestamps and
///   total_difficulty;
/// * the claimed difficulty `H.total_difficulty - (H-1).total_difficulty`
///   equals `consensus::next_wtema_difficulty(H, [H-1, H-2])` exactly (same
///   rule the chain enforces), and `H.pow.to_difficulty(H.height)` reaches it;
/// * `H.total_difficulty`/timestamp are below the anchor's.
///
/// **Pre-HF4 policy is explicit, not guessed:** a sampled header with
/// `version < 5` is rejected with an error saying that pre-HF4 headers need
/// the trusted checkpoint/window policy (DMA difficulty needs a 60-header
/// window and the dual-PoW rules). Callers that sample pre-HF4 heights must
/// validate them via that separate, explicit policy.
///
/// `trusted_tail` headers are trusted by the caller; only sampled `H` headers
/// get full PoW validation. Anchor selection/eclipse defense remains a
/// separate problem.
pub fn verify_header_proofs_with_pow(
	anchor: &BlockHeader,
	proofs: &HeaderProofs,
	trusted_tail: &[BlockHeader],
) -> Result<(), String> {
	verify_memberships(anchor, proofs)?;

	// Height -> header lookup for the triples: proof items take precedence over
	// trusted-tail headers at the same height (they are the fully validated
	// ones; consistency is enforced by prev_hash/ordering checks below).
	let mut by_height: HashMap<u64, &BlockHeader> = HashMap::new();
	for header in trusted_tail {
		by_height.insert(header.height, header);
	}
	for item in &proofs.items {
		by_height.insert(item.header.height, &item.header);
	}

	for item in &proofs.items {
		let header = &item.header;
		let height = header.height;

		if !consensus::valid_header_version(height, header.version) {
			return Err(format!(
				"invalid header version {} at height {}",
				header.version.0, height
			));
		}
		if header.version < HeaderVersion(5) {
			return Err(format!(
				"pre-HF4 header at height {} (version {}): WTEMA triple verification is not applied; use the explicit trusted checkpoint/window policy for pre-HF4 headers",
				height, header.version.0
			));
		}
		if height < 2 {
			return Err(format!(
				"sampled height {} cannot provide h-2/h-1 headers",
				height
			));
		}
		let prev1 = by_height.get(&(height - 1)).ok_or_else(|| {
			format!(
				"sampled height {} requires header h-1={} in the proof set or trusted tail",
				height,
				height - 1
			)
		})?;
		let prev2 = by_height.get(&(height - 2)).ok_or_else(|| {
			format!(
				"sampled height {} requires header h-2={} in the proof set or trusted tail",
				height,
				height - 2
			)
		})?;

		// Post-HF4 is primary Cuckoo only; a v>=5 header carrying secondary PoW
		// would have zero/incorrect consensus difficulty and must fail.
		if header.pow.is_secondary() {
			return Err(format!(
				"post-HF4 header at height {} carries secondary PoW",
				height
			));
		}
		if !header.pow.is_primary() {
			return Err(format!(
				"invalid PoW edge bits {} at height {}",
				header.pow.edge_bits(),
				height
			));
		}
		pow::verify_size(header).map_err(|e| {
			format!(
				"header PoW does not verify at height {} (header fields are not bound by the membership proof): {:?}",
				height, e
			)
		})?;

		// Triple ordering / chain linkage.
		if header.timestamp <= prev1.timestamp || prev1.timestamp <= prev2.timestamp {
			return Err(format!(
				"header timestamps are not strictly increasing for the h-2/h-1/h triple below height {}",
				height
			));
		}
		if header.total_difficulty() <= prev1.total_difficulty()
			|| prev1.total_difficulty() <= prev2.total_difficulty()
		{
			return Err(format!(
				"header total_difficulty is not strictly increasing for the h-2/h-1/h triple below height {}",
				height
			));
		}
		if header.prev_hash != prev1.hash() {
			return Err(format!(
				"header prev_hash linkage mismatch at height {} (h-1)",
				height
			));
		}
		if prev1.prev_hash != prev2.hash() {
			return Err(format!(
				"header prev_hash linkage mismatch at height {} (h-2 -> h-1)",
				height
			));
		}
		if header.timestamp >= anchor.timestamp {
			return Err(format!(
				"header timestamp at height {} is not before the anchor timestamp",
				height
			));
		}
		if header.total_difficulty() > anchor.total_difficulty() {
			return Err(format!(
				"header total_difficulty at height {} exceeds the anchor total_difficulty",
				height
			));
		}

		// Difficulty: the chain derives network difficulty of h-1 from h-2, then
		// requires h.total_difficulty - (h-1).total_difficulty to equal the
		// WTEMA target for h exactly, and the PoW to reach that target.
		let last_net_difficulty = prev1.total_difficulty() - prev2.total_difficulty();
		let last_info = consensus::HeaderDifficultyInfo::new(
			Some(prev1.hash()),
			prev1.timestamp.timestamp() as u64,
			last_net_difficulty,
			prev1.pow.secondary_scaling,
			prev1.pow.is_secondary(),
		);
		let prev_info = consensus::HeaderDifficultyInfo::new(
			Some(prev2.hash()),
			prev2.timestamp.timestamp() as u64,
			crate::core::pow::Difficulty::zero(),
			prev2.pow.secondary_scaling,
			prev2.pow.is_secondary(),
		);
		let expected = consensus::next_wtema_difficulty(height, vec![last_info, prev_info]);
		let claimed_difficulty = header.total_difficulty() - prev1.total_difficulty();
		if claimed_difficulty != expected.difficulty {
			return Err(format!(
				"claimed difficulty {} at height {} does not match expected WTEMA difficulty {}",
				claimed_difficulty, height, expected.difficulty
			));
		}
		if header.pow.to_difficulty(height) < claimed_difficulty {
			return Err(format!(
				"proof of work difficulty too low at height {}",
				height
			));
		}
	}
	Ok(())
}

/// Builds historical header MMR inclusion proofs for the provided headers.
///
/// `backend` must contain at least the header MMR prefix up to
/// `anchor.height` (a node's header MMR backend will typically contain the
/// current tip; only the historical size is used here, never the tip size).
/// `anchor` must be the header that committed to `prev_root`. `headers` must
/// be sorted by strictly increasing height and each height must be below
/// `anchor.height`.
///
/// Every proof is verified against `anchor.prev_root` before it is returned
/// (fail closed: any missing hash or mismatch is an error, never a peer fault).
pub fn build_header_proofs<B: Backend<BlockHeader>>(
	backend: &B,
	anchor: &BlockHeader,
	headers: &[BlockHeader],
) -> Result<HeaderProofs, String> {
	if headers.is_empty() || headers.len() > MAX_HEADER_PROOF_ITEMS as usize {
		return Err(format!(
			"header proof request count {} outside 1..={}",
			headers.len(),
			MAX_HEADER_PROOF_ITEMS
		));
	}
	let mmr_size = header_mmr_size(anchor.height);
	let readonly = ReadonlyPMMR::at(backend, mmr_size);
	let root = readonly
		.root()
		.map_err(|e| format!("failed to read historical header MMR root: {}", e))?;
	if root != anchor.prev_root {
		return Err(format!(
			"historical header MMR root {} does not match anchor prev_root {}",
			root, anchor.prev_root
		));
	}
	let mut items = Vec::with_capacity(headers.len());
	let mut prev_height: Option<u64> = None;
	for header in headers {
		let height = header.height;
		if height >= anchor.height {
			return Err(format!(
				"requested height {} is not below anchor height {}",
				height, anchor.height
			));
		}
		if let Some(prev) = prev_height {
			if height <= prev {
				return Err(format!(
					"requested heights are not strictly increasing ({} after {})",
					height, prev
				));
			}
		}
		prev_height = Some(height);
		let pos = header_leaf_pos(height);
		if pos >= mmr_size {
			return Err(format!(
				"header leaf position {} outside historical MMR size {}",
				pos, mmr_size
			));
		}
		let proof = readonly
			.merkle_proof(pos)
			.map_err(|e| format!("failed to build header proof at height {}: {}", height, e))?;
		proof.verify(root, header, pos).map_err(|_| {
			format!(
				"built header proof does not verify against prev_root at height {}",
				height
			)
		})?;
		items.push(HeaderProofItem {
			header: header.clone(),
			path: proof.path,
		});
	}
	Ok(HeaderProofs {
		anchor: anchor.hash(),
		mmr_size,
		items,
	})
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::core::consensus;
	use crate::core::core::hash::ZERO_HASH;
	use crate::core::core::pmmr::{VecBackend, PMMR};
	use crate::core::global;
	use crate::core::pow::{Difficulty, Proof, ProofOfWork};
	use chrono::{DateTime, Duration, Utc};

	fn init_test_chain() {
		global::set_local_chain_type(global::ChainTypes::AutomatedTesting);
	}

	fn test_header(height: u64, prev_root: Hash) -> BlockHeader {
		BlockHeader {
			height,
			version: consensus::header_version(height),
			prev_root,
			pow: ProofOfWork {
				proof: Proof::zero(global::proofsize()),
				..Default::default()
			},
			..Default::default()
		}
	}

	/// Builds a header MMR over `count` synthetic headers, returning the
	/// backend and the headers. Header `h` commits the root over the first `h`
	/// leaves in `prev_root`, mirroring the real chain invariant.
	fn build_header_mmr(count: u64) -> (VecBackend<BlockHeader>, Vec<BlockHeader>) {
		let mut backend = VecBackend::new();
		let mut headers = Vec::with_capacity(count as usize);
		let mut size = 0u64;
		for height in 0..count {
			let prev_root = ReadonlyPMMR::at(&backend, size)
				.root()
				.expect("historical root");
			let header = test_header(height, prev_root);
			let mut pmmr = PMMR::at(&mut backend, size);
			pmmr.push(&header).expect("push header");
			size = pmmr.size;
			headers.push(header);
		}
		(backend, headers)
	}

	#[test]
	fn historical_proofs_verify_against_prev_root() {
		init_test_chain();
		let (backend, headers) = build_header_mmr(32);
		let anchor = headers[24].clone();
		let heights = [0u64, 1, 2, 7, 8, 15, 16, 23];
		let selected: Vec<BlockHeader> = heights
			.iter()
			.map(|h| headers[*h as usize].clone())
			.collect();

		let proofs = build_header_proofs(&backend, &anchor, &selected).expect("build proofs");
		assert_eq!(proofs.anchor, anchor.hash());
		assert_eq!(proofs.mmr_size, header_mmr_size(anchor.height));
		assert_ne!(proofs.mmr_size, header_mmr_size(headers.len() as u64));
		assert_eq!(proofs.items.len(), heights.len());
		for (item, height) in proofs.items.iter().zip(heights.iter()) {
			assert_eq!(item.header.height, *height);
			assert!(item.path.len() <= MAX_HEADER_PROOF_PATH as usize);
		}
		verify_memberships(&anchor, &proofs).expect("verify proofs");
	}

	#[test]
	fn proofs_use_historical_size_not_tip_size() {
		init_test_chain();
		// Keep appending headers after the anchor; proofs for the anchor must
		// still be built against the historical size, not the tip size.
		let (backend, headers) = build_header_mmr(40);
		let anchor = headers[24].clone();
		let selected = vec![headers[0].clone(), headers[23].clone()];
		let proofs = build_header_proofs(&backend, &anchor, &selected).expect("build proofs");
		assert_eq!(proofs.mmr_size, header_mmr_size(24));

		// A tip-sized MMR reports a different root, so the request must fail
		// closed against the anchor prev_root.
		let tip_root = ReadonlyPMMR::at(&backend, header_mmr_size(40))
			.root()
			.expect("tip root");
		let mut wrong_anchor = anchor.clone();
		wrong_anchor.prev_root = tip_root;
		assert!(build_header_proofs(&backend, &wrong_anchor, &selected).is_err());
	}

	#[test]
	fn tampered_proof_rejected() {
		init_test_chain();
		let (backend, headers) = build_header_mmr(16);
		let anchor = headers[9].clone();
		let selected = vec![headers[3].clone()];
		let proofs = build_header_proofs(&backend, &anchor, &selected).expect("build proofs");

		// Wrong anchor root.
		let mut tampered_anchor = anchor.clone();
		tampered_anchor.prev_root = ZERO_HASH;
		assert!(verify_memberships(&tampered_anchor, &proofs).is_err());

		// Wrong path hash.
		let mut wrong_path = clone_proofs(&proofs);
		assert!(!wrong_path.items[0].path.is_empty());
		wrong_path.items[0].path[0] = ZERO_HASH;
		assert!(verify_memberships(&anchor, &wrong_path).is_err());

		// Wrong leaf header: the header MMR leaf commits to the packed PoW
		// nonces, so mutating a packed nonce must invalidate the proof.
		let mut wrong_leaf = clone_proofs(&proofs);
		assert!(!wrong_leaf.items[0].header.pow.proof.nonces.is_empty());
		wrong_leaf.items[0].header.pow.proof.nonces[0] += 1;
		assert!(verify_memberships(&anchor, &wrong_leaf).is_err());

		// Wrong historical size.
		let mut wrong_size = clone_proofs(&proofs);
		wrong_size.mmr_size += 1;
		assert!(verify_memberships(&anchor, &wrong_size).is_err());

		// Wrong anchor hash echo.
		let mut wrong_anchor_hash = clone_proofs(&proofs);
		wrong_anchor_hash.anchor = ZERO_HASH;
		assert!(verify_memberships(&anchor, &wrong_anchor_hash).is_err());
	}

	#[test]
	fn limits_and_ordering_enforced() {
		init_test_chain();
		let (backend, headers) = build_header_mmr(80);
		let anchor = headers[70].clone();

		// Empty request.
		assert!(build_header_proofs(&backend, &anchor, &[]).is_err());

		// More than MAX_HEADER_PROOF_ITEMS.
		let too_many: Vec<BlockHeader> = (0..=MAX_HEADER_PROOF_ITEMS as u64)
			.map(|h| headers[h as usize].clone())
			.collect();
		assert!(build_header_proofs(&backend, &anchor, &too_many).is_err());

		// Unsorted.
		let unsorted = vec![headers[5].clone(), headers[2].clone()];
		assert!(build_header_proofs(&backend, &anchor, &unsorted).is_err());

		// Duplicate.
		let duplicate = vec![headers[5].clone(), headers[5].clone()];
		assert!(build_header_proofs(&backend, &anchor, &duplicate).is_err());

		// Height equal to anchor height.
		let at_anchor = vec![headers[70].clone()];
		assert!(build_header_proofs(&backend, &anchor, &at_anchor).is_err());

		// Height above anchor height.
		let above_anchor = vec![headers[71].clone()];
		assert!(build_header_proofs(&backend, &anchor, &above_anchor).is_err());
	}

	/// Synthetic trusted-tail header: distinct packed nonces so header hashes
	/// differ; its PoW is not validated by the verifier (tail is trusted).
	fn tail_header(
		height: u64,
		timestamp: DateTime<Utc>,
		total_difficulty: u64,
		prev_hash: Hash,
	) -> BlockHeader {
		let mut nonces: Vec<u64> = (0..global::proofsize())
			.map(|i| i as u64 + height * 1000)
			.collect();
		nonces.sort_unstable();
		BlockHeader {
			height,
			version: consensus::header_version(height),
			timestamp,
			prev_hash,
			pow: ProofOfWork {
				total_difficulty: Difficulty::from_num(total_difficulty),
				proof: Proof {
					edge_bits: global::min_edge_bits(),
					nonces,
				},
				..Default::default()
			},
			..Default::default()
		}
	}

	/// Post-HF4 WTEMA fixture:
	/// * trusted tail heights 10 (td 1000) and 11 (td 1020, ts +60s);
	/// * sampled, actually mined header height 12 with total_difficulty 1040
	///   (= 1020 + expected WTEMA target 20);
	/// * anchor height 13 (td 2000) committing the 13-leaf header MMR prefix.
	fn wtema_fixture() -> (BlockHeader, HeaderProofs, Vec<BlockHeader>) {
		init_test_chain();
		let t0 = BlockHeader::default().timestamp;
		let h10 = tail_header(10, t0, 1000, ZERO_HASH);
		let h11 = tail_header(11, t0 + Duration::seconds(60), 1020, h10.hash());

		let mut backend = VecBackend::new();
		let mut size = 0u64;
		for _ in 0..12 {
			let mut pmmr = PMMR::at(&mut backend, size);
			pmmr.push(&BlockHeader::default()).expect("push header");
			size = pmmr.size;
		}
		let prev_root = ReadonlyPMMR::at(&backend, size).root().expect("root");

		let mut sampled = BlockHeader {
			height: 12,
			version: consensus::header_version(12),
			timestamp: t0 + Duration::seconds(120),
			prev_hash: h11.hash(),
			prev_root,
			pow: ProofOfWork {
				// h11 network difficulty 20 + WTEMA target for 60s block time = 20
				total_difficulty: Difficulty::from_num(1040),
				..Default::default()
			},
			..Default::default()
		};
		pow::pow_size(
			&mut sampled,
			Difficulty::from_num(20),
			global::proofsize(),
			global::min_edge_bits(),
		)
		.expect("mine sampled header");
		{
			let mut pmmr = PMMR::at(&mut backend, size);
			pmmr.push(&sampled).expect("push sampled header");
			size = pmmr.size;
		}
		let anchor_root = ReadonlyPMMR::at(&backend, size)
			.root()
			.expect("anchor root");
		let anchor = BlockHeader {
			height: 13,
			prev_root: anchor_root,
			timestamp: t0 + Duration::seconds(180),
			pow: ProofOfWork {
				total_difficulty: Difficulty::from_num(2000),
				..Default::default()
			},
			..Default::default()
		};
		let proofs =
			build_header_proofs(&backend, &anchor, &[sampled]).expect("build header proofs");
		(anchor, proofs, vec![h10, h11])
	}

	#[test]
	fn valid_wtema_triple_accepted() {
		let (anchor, proofs, tail) = wtema_fixture();
		verify_memberships(&anchor, &proofs).expect("membership");
		verify_header_proofs_with_pow(&anchor, &proofs, &tail)
			.expect("valid h-2/h-1/h triple with WTEMA difficulty must be accepted");
	}

	#[test]
	fn sampled_header_without_pair_rejected() {
		let (anchor, proofs, tail) = wtema_fixture();

		// No h-1/h-2 at all.
		let err = verify_header_proofs_with_pow(&anchor, &proofs, &[])
			.expect_err("missing pair must be rejected");
		assert!(
			err.contains("requires header h-1"),
			"unexpected error: {}",
			err
		);

		// Only h-1 present: h-2 still missing.
		let err = verify_header_proofs_with_pow(&anchor, &proofs, &[tail[1].clone()])
			.expect_err("missing h-2 must be rejected");
		assert!(
			err.contains("requires header h-2"),
			"unexpected error: {}",
			err
		);
	}

	#[test]
	fn altered_difficulty_rejected() {
		let (anchor, proofs, tail) = wtema_fixture();
		// Keep the h-2 -> h-1 linkage/hash valid (hashes commit packed nonces
		// only) but alter h-1's claimed total_difficulty: the network difficulty
		// changes, so the expected WTEMA target no longer matches H's claimed
		// difficulty.
		let mut tampered_tail = tail.clone();
		tampered_tail[1].pow.total_difficulty = Difficulty::from_num(1010);
		let err = verify_header_proofs_with_pow(&anchor, &proofs, &tampered_tail)
			.expect_err("altered difficulty must be rejected");
		assert!(
			err.contains("does not match expected WTEMA difficulty"),
			"unexpected error: {}",
			err
		);
	}

	#[test]
	fn tampered_sampled_header_membership_ok_pow_fails() {
		let (anchor, proofs, tail) = wtema_fixture();
		let mut tampered = clone_proofs(&proofs);
		// Keep the packed PoW nonces: the MMR leaf is unchanged, membership passes.
		tampered.items[0].header.timestamp += Duration::seconds(-30);
		tampered.items[0].header.output_root = Hash::from_vec(&[0xAB; 32]);
		verify_memberships(&anchor, &tampered)
			.expect("membership proof does not bind non-PoW header fields");
		// The PoW solution binds these fields, so with_pow must reject it.
		assert!(
			verify_header_proofs_with_pow(&anchor, &tampered, &tail).is_err(),
			"verify_header_proofs_with_pow must reject non-PoW field tampering"
		);
	}

	#[test]
	fn pre_hf4_sampled_header_requires_explicit_policy() {
		init_test_chain();
		let (backend, headers) = build_header_mmr(13);
		let anchor = headers[12].clone();
		let proofs = build_header_proofs(&backend, &anchor, &[headers[11].clone()])
			.expect("build header proofs");
		// Height 11 is pre-HF4 (version 4) on the AutomatedTesting chain.
		let err = verify_header_proofs_with_pow(
			&anchor,
			&proofs,
			&[headers[9].clone(), headers[10].clone()],
		)
		.expect_err("pre-HF4 sampled header must be rejected");
		assert!(err.contains("pre-HF4"), "unexpected error: {}", err);
	}

	fn clone_proofs(proofs: &HeaderProofs) -> HeaderProofs {
		HeaderProofs {
			anchor: proofs.anchor,
			mmr_size: proofs.mmr_size,
			items: proofs
				.items
				.iter()
				.map(|i| HeaderProofItem {
					header: i.header.clone(),
					path: i.path.clone(),
				})
				.collect(),
		}
	}
}
