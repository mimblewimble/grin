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

//! Historical header MMR inclusion proofs.
//!
//! Every header commits in `prev_root` to the header MMR over all headers
//! below it. A proof shows that a historical header is the leaf at its height
//! under the root committed to by a later *anchor* header. The node builds
//! proofs against the anchor's historical MMR size; the header MMR is never
//! pruned, so no rewind is involved.
//!
//! Each sampled header may come with up to [`MAX_CONTEXT`] headers directly
//! below it (`context`). They carry no Merkle path: `prev_hash` links them to
//! the sampled header, which is what difficulty checks need.
//!
//! # What a proof does and does not show
//!
//! * It shows membership in the anchor's header MMR. It does not show that the
//!   anchor is on the most-work chain; choosing the anchor is the client's job.
//! * The MMR leaf commits only to the PoW nonces. Every other header field is
//!   bound through the PoW pre-image, so [`verify`] checks the PoW solution of
//!   every sampled header.
//! * Sampling by height alone catches a forged chain with one large jump in
//!   `total_difficulty` only with probability of about 1/n. Callers that need
//!   FlyClient-style guarantees should sample by cumulative work.

use crate::consensus;
use crate::core::hash::{Hash, Hashed};
use crate::core::merkle_proof::MerkleProof;
use crate::core::pmmr::{self, Backend, ReadablePMMR, ReadonlyPMMR};
use crate::core::{BlockHeader, HeaderVersion};
use crate::pow::{self, Difficulty};

/// Maximum number of sampled heights in one request.
pub const MAX_ITEMS: usize = 64;
/// Maximum number of context headers per sampled height.
pub const MAX_CONTEXT: usize = 60;
/// Maximum number of headers (sampled and context) in one response.
pub const MAX_HEADERS: usize = 256;
/// Maximum Merkle path length accepted by the verifier.
pub const MAX_PATH: usize = 64;

/// Errors from building or verifying header proofs.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
	/// The request holds no heights or too many.
	#[error("request has {0} heights, expected 1 to {max}", max = MAX_ITEMS)]
	ItemCount(usize),
	/// Too many context headers per height.
	#[error("context of {0} headers is more than {max}", max = MAX_CONTEXT)]
	ContextTooLarge(usize),
	/// The response would hold too many headers.
	#[error("response would hold {0} headers, more than {max}", max = MAX_HEADERS)]
	TooManyHeaders(usize),
	/// Heights are not strictly increasing.
	#[error("heights are not strictly increasing ({1} after {0})")]
	NotIncreasing(u64, u64),
	/// A height is not below the anchor.
	#[error("height {0} is not below the anchor height {1}")]
	NotBelowAnchor(u64, u64),
	/// A height has fewer headers below it than the context asks for.
	#[error("height {0} has fewer than {1} headers below it")]
	NoRoomForContext(u64, usize),
	/// Items carry different numbers of context headers.
	#[error("items carry different numbers of context headers")]
	UnevenContext,
	/// The anchor is not known to the node.
	#[error("anchor {0} is not known")]
	UnknownAnchor(String),
	/// The anchor is known but not on the node's main header chain.
	#[error("anchor {0} is not on the main header chain")]
	AnchorNotOnMainChain(String),
	/// The historical header MMR root does not match the anchor.
	#[error("historical header MMR root does not match the anchor prev_root")]
	RootMismatch,
	/// The header MMR could not be read.
	#[error("cannot read the header MMR: {0}")]
	Mmr(String),
	/// The proofs name another anchor, anchor height or MMR size.
	#[error("proofs do not match the anchor header")]
	AnchorMismatch,
	/// A header is not the leaf at its height under the anchor.
	#[error("header at height {0} is not a leaf under the anchor")]
	NotMember(u64),
	/// A header sits at another height than requested.
	#[error("header at height {got} where height {want} was expected")]
	WrongHeight {
		/// Height that was expected.
		want: u64,
		/// Height the header carries.
		got: u64,
	},
	/// A Merkle path is too long.
	#[error("Merkle path of {0} hashes is longer than {max}", max = MAX_PATH)]
	PathTooLong(usize),
	/// A context header is not linked to the header above it.
	#[error("context header at height {0} is not linked by prev_hash")]
	BrokenContext(u64),
	/// The header version is not valid for its height.
	#[error("invalid header version at height {0}")]
	BadVersion(u64),
	/// The proof of work solution is invalid.
	#[error("invalid proof of work at height {0}")]
	BadPow(u64),
	/// Timestamps do not strictly increase.
	#[error("timestamps do not strictly increase at height {0}")]
	BadTimestamp(u64),
	/// Total difficulty does not strictly increase.
	#[error("total difficulty does not strictly increase at height {0}")]
	BadTotalDifficulty(u64),
	/// A header is not below the anchor in time or total difficulty.
	#[error("header at height {0} is not below the anchor in time or total difficulty")]
	AboveAnchor(u64),
	/// The proof of work does not reach the claimed difficulty.
	#[error("proof of work at height {0} does not reach the claimed difficulty")]
	DifficultyTooLow(u64),
	/// The claimed difficulty is not the consensus target.
	#[error("claimed difficulty at height {0} is not the consensus target")]
	WrongDifficulty(u64),
}

/// One sampled header with its Merkle path and context headers.
#[derive(Clone, Debug, PartialEq)]
pub struct HeaderProofItem {
	/// The sampled header.
	pub header: BlockHeader,
	/// Merkle path from the header's leaf to the anchor's `prev_root`.
	pub path: Vec<Hash>,
	/// Headers directly below `header`, nearest first: H-1, H-2, ...
	pub context: Vec<BlockHeader>,
}

/// Proofs for a set of sampled heights against one anchor.
#[derive(Clone, Debug, PartialEq)]
pub struct HeaderProofs {
	/// Hash of the anchor header.
	pub anchor: Hash,
	/// Height of the anchor header.
	pub anchor_height: u64,
	/// Size of the header MMR the anchor commits to.
	pub mmr_size: u64,
	/// One item per sampled height, by increasing height.
	pub items: Vec<HeaderProofItem>,
}

/// How much of a sampled header [`verify`] could check.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
	/// The header is the leaf at its height under the anchor and its proof of
	/// work solution is valid, which binds every header field. Its difficulty
	/// is not checked: there is no context header.
	Solution,
	/// Also: the proof of work reaches the claimed difficulty
	/// `total(H) - total(H-1)`. Needs at least one context header and works
	/// at any height.
	Pow,
	/// Also: the claimed difficulty equals the consensus target. Checked after
	/// HF4 (WTEMA) with at least two context headers. The pre-HF4 rule needs
	/// a 60-header window and dual PoW and is not checked here.
	Exact,
}

/// Size of the header MMR after the first `height` headers. This is the MMR
/// a header at `height` commits to in `prev_root`.
pub fn header_mmr_size(height: u64) -> u64 {
	pmmr::insertion_to_pmmr_index(height)
}

/// Checks a request against the limits and the anchor height.
pub fn check_request(anchor_height: u64, heights: &[u64], context: usize) -> Result<(), Error> {
	if heights.is_empty() || heights.len() > MAX_ITEMS {
		return Err(Error::ItemCount(heights.len()));
	}
	if context > MAX_CONTEXT {
		return Err(Error::ContextTooLarge(context));
	}
	let total = heights.len().saturating_mul(context.saturating_add(1));
	if total > MAX_HEADERS {
		return Err(Error::TooManyHeaders(total));
	}
	let mut prev: Option<u64> = None;
	for &height in heights {
		if let Some(prev) = prev {
			if height <= prev {
				return Err(Error::NotIncreasing(prev, height));
			}
		}
		if height >= anchor_height {
			return Err(Error::NotBelowAnchor(height, anchor_height));
		}
		if height < context as u64 {
			return Err(Error::NoRoomForContext(height, context));
		}
		prev = Some(height);
	}
	Ok(())
}

/// Builds proofs for `samples`: pairs of a header on the anchor's chain and
/// its context headers, nearest first.
///
/// `backend` must hold the header MMR at least up to the anchor; only the
/// anchor's historical size is read. The result is checked against
/// `anchor.prev_root` before it is returned.
pub fn build<B: Backend<BlockHeader>>(
	backend: &B,
	anchor: &BlockHeader,
	samples: Vec<(BlockHeader, Vec<BlockHeader>)>,
) -> Result<HeaderProofs, Error> {
	let heights: Vec<u64> = samples.iter().map(|(h, _)| h.height).collect();
	let context = samples.first().map(|(_, c)| c.len()).unwrap_or(0);
	if samples.iter().any(|(_, c)| c.len() != context) {
		return Err(Error::UnevenContext);
	}
	check_request(anchor.height, &heights, context)?;

	let mmr_size = header_mmr_size(anchor.height);
	let readonly = ReadonlyPMMR::at(backend, mmr_size);
	let root = readonly.root().map_err(Error::Mmr)?;
	if root != anchor.prev_root {
		return Err(Error::RootMismatch);
	}
	let mut items = Vec::with_capacity(samples.len());
	for (header, context) in samples {
		let pos = header_mmr_size(header.height);
		let proof = readonly.merkle_proof(pos).map_err(Error::Mmr)?;
		proof
			.verify(root, &header, pos)
			.map_err(|_| Error::NotMember(header.height))?;
		check_context(&header, &context)?;
		items.push(HeaderProofItem {
			header,
			path: proof.path,
			context,
		});
	}
	Ok(HeaderProofs {
		anchor: anchor.hash(),
		anchor_height: anchor.height,
		mmr_size,
		items,
	})
}

/// Verifies `proofs` against `anchor`, a header the caller has chosen and
/// trusts to be on the chain it wants to check. Returns the [`Level`] reached
/// by each item, in order.
pub fn verify(anchor: &BlockHeader, proofs: &HeaderProofs) -> Result<Vec<Level>, Error> {
	if proofs.anchor != anchor.hash()
		|| proofs.anchor_height != anchor.height
		|| proofs.mmr_size != header_mmr_size(anchor.height)
	{
		return Err(Error::AnchorMismatch);
	}
	let heights: Vec<u64> = proofs.items.iter().map(|i| i.header.height).collect();
	let context = proofs.items.first().map(|i| i.context.len()).unwrap_or(0);
	if proofs.items.iter().any(|i| i.context.len() != context) {
		return Err(Error::UnevenContext);
	}
	check_request(anchor.height, &heights, context)?;
	proofs
		.items
		.iter()
		.map(|item| verify_item(anchor, proofs.mmr_size, item))
		.collect()
}

fn verify_item(
	anchor: &BlockHeader,
	mmr_size: u64,
	item: &HeaderProofItem,
) -> Result<Level, Error> {
	let header = &item.header;
	let height = header.height;

	if item.path.len() > MAX_PATH {
		return Err(Error::PathTooLong(item.path.len()));
	}
	let pos = header_mmr_size(height);
	if pos >= mmr_size {
		return Err(Error::NotMember(height));
	}
	let proof = MerkleProof {
		mmr_size,
		path: item.path.clone(),
	};
	proof
		.verify(anchor.prev_root, header, pos)
		.map_err(|_| Error::NotMember(height))?;

	if !consensus::valid_header_version(height, header.version) {
		return Err(Error::BadVersion(height));
	}
	if header.version >= HeaderVersion(5) && !header.pow.is_primary() {
		return Err(Error::BadPow(height));
	}
	pow::verify_size(header).map_err(|_| Error::BadPow(height))?;
	if header.timestamp >= anchor.timestamp
		|| header.total_difficulty() >= anchor.total_difficulty()
	{
		return Err(Error::AboveAnchor(height));
	}
	check_context(header, &item.context)?;

	let prev1 = match item.context.first() {
		Some(prev1) => prev1,
		None => return Ok(Level::Solution),
	};
	let claimed = header.total_difficulty() - prev1.total_difficulty();
	if header.pow.to_difficulty(height) < claimed {
		return Err(Error::DifficultyTooLow(height));
	}
	let prev2 = match item.context.get(1) {
		Some(prev2) if header.version >= HeaderVersion(5) => prev2,
		_ => return Ok(Level::Pow),
	};

	// Same rule as the chain: the network difficulty of H-1 comes from H-2,
	// and the difficulty of H must equal the WTEMA target exactly.
	let last = consensus::HeaderDifficultyInfo::new(
		Some(prev1.hash()),
		prev1.timestamp.timestamp() as u64,
		prev1.total_difficulty() - prev2.total_difficulty(),
		prev1.pow.secondary_scaling,
		prev1.pow.is_secondary(),
	);
	let before = consensus::HeaderDifficultyInfo::new(
		Some(prev2.hash()),
		prev2.timestamp.timestamp() as u64,
		Difficulty::zero(),
		prev2.pow.secondary_scaling,
		prev2.pow.is_secondary(),
	);
	let expected = consensus::next_wtema_difficulty(height, vec![last, before]);
	if claimed != expected.difficulty {
		return Err(Error::WrongDifficulty(height));
	}
	Ok(Level::Exact)
}

/// Context headers sit directly below `header`, nearest first, linked by
/// `prev_hash`, with strictly increasing timestamps and total difficulty.
fn check_context(header: &BlockHeader, context: &[BlockHeader]) -> Result<(), Error> {
	let mut child = header;
	for parent in context {
		if Some(parent.height) != child.height.checked_sub(1) || child.prev_hash != parent.hash() {
			return Err(Error::BrokenContext(parent.height));
		}
		if child.timestamp <= parent.timestamp {
			return Err(Error::BadTimestamp(child.height));
		}
		if child.total_difficulty() <= parent.total_difficulty() {
			return Err(Error::BadTotalDifficulty(child.height));
		}
		child = parent;
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::core::hash::ZERO_HASH;
	use crate::core::pmmr::{VecBackend, PMMR};
	use crate::global;
	use crate::pow::{Proof, ProofOfWork};
	use chrono::Duration;

	fn init() {
		global::set_local_chain_type(global::ChainTypes::AutomatedTesting);
	}

	/// Synthetic header with distinct nonces; its PoW is not valid.
	fn synthetic(height: u64, prev: Option<&BlockHeader>, prev_root: Hash) -> BlockHeader {
		let mut nonces: Vec<u64> = (0..global::proofsize() as u64)
			.map(|i| i + height * 1000)
			.collect();
		nonces.sort_unstable();
		let t0 = BlockHeader::default().timestamp;
		BlockHeader {
			height,
			version: consensus::header_version(height),
			timestamp: t0 + Duration::seconds(60 * height as i64),
			prev_hash: prev.map(|p| p.hash()).unwrap_or(ZERO_HASH),
			prev_root,
			pow: ProofOfWork {
				total_difficulty: Difficulty::from_num(1000 + 20 * height),
				proof: Proof {
					edge_bits: global::min_edge_bits(),
					nonces,
				},
				..Default::default()
			},
			..Default::default()
		}
	}

	/// Header MMR over `count` linked synthetic headers.
	fn synthetic_chain(count: u64) -> (VecBackend<BlockHeader>, Vec<BlockHeader>) {
		let mut backend = VecBackend::new();
		let mut headers: Vec<BlockHeader> = Vec::new();
		let mut size = 0u64;
		for height in 0..count {
			let root = ReadonlyPMMR::at(&backend, size).root().expect("root");
			let header = synthetic(height, headers.last(), root);
			let mut mmr = PMMR::at(&mut backend, size);
			mmr.push(&header).expect("push");
			size = mmr.size;
			headers.push(header);
		}
		(backend, headers)
	}

	fn samples(
		headers: &[BlockHeader],
		heights: &[u64],
		context: usize,
	) -> Vec<(BlockHeader, Vec<BlockHeader>)> {
		heights
			.iter()
			.map(|&h| {
				let ctx = (1..=context as u64)
					.map(|i| headers[(h - i) as usize].clone())
					.collect();
				(headers[h as usize].clone(), ctx)
			})
			.collect()
	}

	#[test]
	fn proofs_use_the_anchor_size_and_link_context() {
		init();
		let (backend, headers) = synthetic_chain(40);
		let anchor = &headers[24];
		let proofs =
			build(&backend, anchor, samples(&headers, &[2, 7, 8, 16, 23], 2)).expect("build");
		assert_eq!(proofs.anchor, anchor.hash());
		assert_eq!(proofs.anchor_height, 24);
		assert_eq!(proofs.mmr_size, header_mmr_size(24));
		assert_ne!(proofs.mmr_size, header_mmr_size(40));
		for item in &proofs.items {
			assert_eq!(item.context.len(), 2);
			assert!(item.path.len() <= MAX_PATH);
		}

		// The tip-sized root is not what the anchor commits to.
		let mut wrong = anchor.clone();
		wrong.prev_root = ReadonlyPMMR::at(&backend, header_mmr_size(40))
			.root()
			.expect("tip root");
		assert_eq!(
			build(&backend, &wrong, samples(&headers, &[3], 0)),
			Err(Error::RootMismatch)
		);

		// Context that does not sit below the sample is refused when building.
		let mut broken = samples(&headers, &[10], 2);
		broken[0].1.swap(0, 1);
		assert!(matches!(
			build(&backend, anchor, broken),
			Err(Error::BrokenContext(_))
		));
	}

	#[test]
	fn request_limits() {
		assert_eq!(check_request(100, &[], 0), Err(Error::ItemCount(0)));
		let many: Vec<u64> = (0..=MAX_ITEMS as u64).collect();
		assert_eq!(
			check_request(100, &many, 0),
			Err(Error::ItemCount(MAX_ITEMS + 1))
		);
		assert_eq!(
			check_request(100, &[70], MAX_CONTEXT + 1),
			Err(Error::ContextTooLarge(MAX_CONTEXT + 1))
		);
		let full: Vec<u64> = (10..10 + MAX_ITEMS as u64).collect();
		assert_eq!(
			check_request(100, &full, 4),
			Err(Error::TooManyHeaders(MAX_ITEMS * 5))
		);
		assert_eq!(
			check_request(100, &[5, 2], 0),
			Err(Error::NotIncreasing(5, 2))
		);
		assert_eq!(
			check_request(100, &[5, 5], 0),
			Err(Error::NotIncreasing(5, 5))
		);
		assert_eq!(
			check_request(100, &[100], 0),
			Err(Error::NotBelowAnchor(100, 100))
		);
		assert_eq!(
			check_request(100, &[1], 2),
			Err(Error::NoRoomForContext(1, 2))
		);
		assert_eq!(check_request(100, &[2, 99], 2), Ok(()));
	}

	#[test]
	fn tampering_breaks_membership() {
		init();
		let (backend, headers) = synthetic_chain(16);
		let anchor = &headers[9];
		let proofs = build(&backend, anchor, samples(&headers, &[3], 0)).expect("build");

		// The anchor hash commits to its PoW only, so another root shows up
		// as a failed membership check.
		let mut other_root = anchor.clone();
		other_root.prev_root = ZERO_HASH;
		assert_eq!(verify(&other_root, &proofs), Err(Error::NotMember(3)));

		let mut wrong_path = proofs.clone();
		wrong_path.items[0].path[0] = ZERO_HASH;
		assert_eq!(verify(anchor, &wrong_path), Err(Error::NotMember(3)));

		// The leaf commits to the PoW nonces.
		let mut wrong_leaf = proofs.clone();
		wrong_leaf.items[0].header.pow.proof.nonces[0] += 1;
		assert_eq!(verify(anchor, &wrong_leaf), Err(Error::NotMember(3)));

		let mut wrong_size = proofs.clone();
		wrong_size.mmr_size += 1;
		assert_eq!(verify(anchor, &wrong_size), Err(Error::AnchorMismatch));

		// Membership holds, but these synthetic headers carry no valid PoW.
		assert_eq!(verify(anchor, &proofs), Err(Error::BadPow(3)));
	}

	/// A header MMR of `below` placeholder leaves, then a mined header at
	/// height `below` on top of `context`, then an anchor above it. Returns
	/// the anchor and the proofs for the mined header.
	fn mined(
		height: u64,
		context: Vec<BlockHeader>,
		claimed: u64,
		target: u64,
	) -> (BlockHeader, HeaderProofs) {
		init();
		let mut backend = VecBackend::new();
		let mut size = 0u64;
		for _ in 0..height {
			let mut mmr = PMMR::at(&mut backend, size);
			mmr.push(&BlockHeader::default()).expect("push");
			size = mmr.size;
		}
		let prev_root = ReadonlyPMMR::at(&backend, size).root().expect("root");
		let parent = &context[0];
		let mut sampled = BlockHeader {
			height,
			version: consensus::header_version(height),
			timestamp: parent.timestamp + Duration::seconds(60),
			prev_hash: parent.hash(),
			prev_root,
			pow: ProofOfWork {
				total_difficulty: parent.total_difficulty() + Difficulty::from_num(claimed),
				..Default::default()
			},
			..Default::default()
		};
		pow::pow_size(
			&mut sampled,
			Difficulty::from_num(target),
			global::proofsize(),
			global::min_edge_bits(),
		)
		.expect("mine");
		let mut mmr = PMMR::at(&mut backend, size);
		mmr.push(&sampled).expect("push sampled");
		size = mmr.size;
		let anchor = BlockHeader {
			height: height + 1,
			prev_root: ReadonlyPMMR::at(&backend, size)
				.root()
				.expect("anchor root"),
			timestamp: sampled.timestamp + Duration::seconds(60),
			pow: ProofOfWork {
				total_difficulty: sampled.total_difficulty() + Difficulty::from_num(1000),
				..Default::default()
			},
			..Default::default()
		};
		let proofs = HeaderProofs {
			anchor: anchor.hash(),
			anchor_height: anchor.height,
			mmr_size: size,
			items: vec![HeaderProofItem {
				header: sampled.clone(),
				path: ReadonlyPMMR::at(&backend, size)
					.merkle_proof(header_mmr_size(height))
					.expect("path")
					.path,
				context,
			}],
		};
		(anchor, proofs)
	}

	/// H-1 and H-2 for a header at `height`: 60 seconds apart, H-1 with
	/// network difficulty 20.
	fn parents(height: u64) -> Vec<BlockHeader> {
		init();
		let t0 = BlockHeader::default().timestamp;
		let mut h2 = synthetic(height - 2, None, ZERO_HASH);
		h2.timestamp = t0;
		h2.pow.total_difficulty = Difficulty::from_num(1000);
		let mut h1 = synthetic(height - 1, Some(&h2), ZERO_HASH);
		h1.timestamp = t0 + Duration::seconds(60);
		h1.pow.total_difficulty = Difficulty::from_num(1020);
		vec![h1, h2]
	}

	#[test]
	fn post_hf4_levels() {
		// On AutomatedTesting, version 5 (HF4) starts at height 12. With a 60 s
		// block time the WTEMA target after a difficulty of 20 stays 20.
		let (anchor, proofs) = mined(12, parents(12), 20, 20);
		assert_eq!(verify(&anchor, &proofs), Ok(vec![Level::Exact]));

		let mut one = proofs.clone();
		one.items[0].context.truncate(1);
		assert_eq!(verify(&anchor, &one), Ok(vec![Level::Pow]));

		let mut none = proofs.clone();
		none.items[0].context.clear();
		assert_eq!(verify(&anchor, &none), Ok(vec![Level::Solution]));
	}

	#[test]
	fn wrong_difficulty_is_caught() {
		// The claimed difficulty is not the WTEMA target.
		let (anchor, proofs) = mined(12, parents(12), 21, 21);
		assert_eq!(verify(&anchor, &proofs), Err(Error::WrongDifficulty(12)));

		// A context header with another total difficulty changes the target:
		// H-1 now has a network difficulty of 30.
		let (anchor, mut proofs) = mined(12, parents(12), 20, 20);
		proofs.items[0].context[1].pow.total_difficulty = Difficulty::from_num(990);
		assert_eq!(verify(&anchor, &proofs), Err(Error::WrongDifficulty(12)));
	}

	#[test]
	fn difficulty_the_pow_does_not_reach_is_caught() {
		let (anchor, proofs) = mined(12, parents(12), 1_000_000_000, 1);
		assert_eq!(verify(&anchor, &proofs), Err(Error::DifficultyTooLow(12)));
	}

	#[test]
	fn pre_hf4_reaches_pow_level() {
		// Height 11 is version 4 on AutomatedTesting.
		let (anchor, proofs) = mined(11, parents(11), 20, 20);
		assert_eq!(verify(&anchor, &proofs), Ok(vec![Level::Pow]));
	}

	#[test]
	fn fields_outside_the_leaf_are_bound_by_pow() {
		let (anchor, mut proofs) = mined(12, parents(12), 20, 20);
		// The leaf commits to the nonces only, so membership still holds.
		proofs.items[0].header.output_root = Hash::from_vec(&[0xAB; 32]);
		assert_eq!(verify(&anchor, &proofs), Err(Error::BadPow(12)));
	}

	#[test]
	fn unlinked_context_is_caught() {
		let (anchor, mut proofs) = mined(12, parents(12), 20, 20);
		proofs.items[0].context.swap(0, 1);
		assert!(matches!(
			verify(&anchor, &proofs),
			Err(Error::BrokenContext(_))
		));

		let (anchor, mut proofs) = mined(12, parents(12), 20, 20);
		proofs.items[0].context[0].height = 9;
		assert!(matches!(
			verify(&anchor, &proofs),
			Err(Error::BrokenContext(_))
		));
	}
}
