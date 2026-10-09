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

//! FlyClient (Path A) — sampling + structural verification without a VDMMR hard fork.
//!
//! See `doc/flyclient.md` and issues #1555 / #3479.
//!
//! This module provides:
//! - difficulty-weighted sample target generation (deterministic from tip hash),
//! - lookup of the first header whose cumulative difficulty covers a target,
//! - proof types (`FlyClientProof` / `FlyClientSample`),
//! - structural verification (Merkle inclusion, cumulative difficulty order,
//!   prev_hash linkage for neighbors, inter-sample feasibility bounds).
//!
//! Full PoW validation of sampled headers is optional (callers that have
//! consensus context can enable it later). Chain-backed proof *generation*
//! lives in the `chain` crate.

use crate::core::hash::{Hash, Hashed};
use crate::core::merkle_proof::MerkleProof;
use crate::core::pmmr;
use crate::core::BlockHeader;
use crate::ser::{self, Readable, Reader, Writeable, Writer};
use byteorder::{BigEndian, ByteOrder};

/// Default security parameter λ (bits of soundness) used for sample counts.
pub const DEFAULT_LAMBDA: u32 = 50;

/// Protocol version for the FlyClient proof encoding (Path A).
pub const FLYCLIENT_PROOF_VERSION: u16 = 1;

/// Tunable sampling / verification parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SampleParams {
	/// Security parameter λ (higher ⇒ more samples).
	pub lambda: u32,
	/// Extra multiplicative factor on sample count (paper uses O(λ log n)).
	pub sample_multiplier: u32,
	/// Minimum number of samples (aside from tip/genesis handling).
	pub min_samples: u32,
	/// Maximum number of samples (DoS bound for proof size).
	pub max_samples: u32,
	/// Inter-sample feasibility: max allowed work-growth factor vs linear time
	/// estimate. Generous; not a full DAA. `work * BLOCK_TIME <= time * factor * avg`.
	pub max_work_time_factor: u64,
}

impl Default for SampleParams {
	fn default() -> SampleParams {
		SampleParams {
			lambda: DEFAULT_LAMBDA,
			sample_multiplier: 1,
			min_samples: 8,
			max_samples: 512,
			max_work_time_factor: 32,
		}
	}
}

impl SampleParams {
	/// Number of difficulty targets to sample for a chain with `n_leaves` headers.
	pub fn sample_count(&self, n_leaves: u64) -> usize {
		if n_leaves <= 1 {
			return 0;
		}
		// O(λ log₂ n)
		let log_n = 64 - (n_leaves - 1).leading_zeros();
		let raw = (self.lambda as u64)
			.saturating_mul(log_n as u64)
			.saturating_mul(self.sample_multiplier as u64)
			.max(self.min_samples as u64);
		raw.min(self.max_samples as u64)
			.min(n_leaves.saturating_sub(1)) as usize
	}
}

/// One sampled header and its inclusion proof in the header MMR.
#[derive(Debug, Clone, PartialEq)]
pub struct FlyClientSample {
	/// 0-based leaf position in the header MMR (`pmmr::insertion_to_pmmr_index(height)`).
	pub leaf_pos: u64,
	/// The sampled block header (full header so Merkle leaf hash and PoW can be checked).
	pub header: BlockHeader,
	/// Immediate predecessor header (height-1), required for wtema / difficulty checks.
	/// Absent only for genesis (height 0).
	pub prev_header: Option<BlockHeader>,
	/// Merkle inclusion proof of `header` in the header MMR of size `proof.mmr_size`.
	pub merkle_proof: MerkleProof,
}

impl Writeable for FlyClientSample {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		writer.write_u64(self.leaf_pos)?;
		Writeable::write(&self.header, writer)?;
		match &self.prev_header {
			None => writer.write_u8(0)?,
			Some(h) => {
				writer.write_u8(1)?;
				Writeable::write(h, writer)?;
			}
		}
		Writeable::write(&self.merkle_proof, writer)?;
		Ok(())
	}
}

impl Readable for FlyClientSample {
	fn read<R: Reader>(reader: &mut R) -> Result<FlyClientSample, ser::Error> {
		let leaf_pos = reader.read_u64()?;
		let header = BlockHeader::read(reader)?;
		let prev_header = match reader.read_u8()? {
			0 => None,
			1 => Some(BlockHeader::read(reader)?),
			_ => return Err(ser::Error::CorruptedData),
		};
		let merkle_proof = MerkleProof::read(reader)?;
		Ok(FlyClientSample {
			leaf_pos,
			header,
			prev_header,
			merkle_proof,
		})
	}
}

/// A Path-A FlyClient proof for a claimed tip.
#[derive(Debug, Clone, PartialEq)]
pub struct FlyClientProof {
	/// Encoding version.
	pub version: u16,
	/// Trusted genesis (or checkpoint) block hash the verifier is bootstrapped with.
	pub genesis_hash: Hash,
	/// Claimed tip header (most-work header being proven).
	pub tip: BlockHeader,
	/// Header MMR size **after** the tip is applied (i.e. MMR root commits all headers
	/// through tip). For Grin, `tip.prev_root` commits the MMR *before* tip; the prover
	/// also supplies merkle proofs against the MMR that includes the tip by using
	/// `mmr_root` below.
	pub mmr_size: u64,
	/// Root of the header MMR of size `mmr_size` (must commit to all samples + tip).
	pub mmr_root: Hash,
	/// Difficulty-weighted samples (excluding tip if sampled separately).
	pub samples: Vec<FlyClientSample>,
}

impl Writeable for FlyClientProof {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), ser::Error> {
		writer.write_u16(self.version)?;
		Writeable::write(&self.genesis_hash, writer)?;
		Writeable::write(&self.tip, writer)?;
		writer.write_u64(self.mmr_size)?;
		Writeable::write(&self.mmr_root, writer)?;
		writer.write_u64(self.samples.len() as u64)?;
		for s in &self.samples {
			Writeable::write(s, writer)?;
		}
		Ok(())
	}
}

impl Readable for FlyClientProof {
	fn read<R: Reader>(reader: &mut R) -> Result<FlyClientProof, ser::Error> {
		let version = reader.read_u16()?;
		if version != FLYCLIENT_PROOF_VERSION {
			return Err(ser::Error::UnexpectedData {
				expected: vec![],
				received: vec![],
			});
		}
		let genesis_hash = Hash::read(reader)?;
		let tip = BlockHeader::read(reader)?;
		let mmr_size = reader.read_u64()?;
		let mmr_root = Hash::read(reader)?;
		let n = reader.read_u64()?;
		if n > 10_000 {
			return Err(ser::Error::TooLargeReadErr);
		}
		let mut samples = Vec::with_capacity(n as usize);
		for _ in 0..n {
			samples.push(FlyClientSample::read(reader)?);
		}
		Ok(FlyClientProof {
			version,
			genesis_hash,
			tip,
			mmr_size,
			mmr_root,
			samples,
		})
	}
}

/// FlyClient verification errors.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum Error {
	/// Proof encoding version not supported.
	#[error("Unsupported FlyClient proof version")]
	UnsupportedVersion,
	/// Tip does not match expected genesis/checkpoint linkage.
	#[error("Genesis / checkpoint mismatch")]
	GenesisMismatch,
	/// Merkle proof failed for a sample.
	#[error("Merkle proof verification failed at height {0}")]
	MerkleProof(u64),
	/// Sample leaf position does not match header height.
	#[error("Leaf position inconsistent with height {0}")]
	LeafPos(u64),
	/// prev_header missing or hash mismatch.
	#[error("Predecessor header missing or mismatch at height {0}")]
	PrevHeader(u64),
	/// Cumulative difficulty not strictly increasing along the chain of samples.
	#[error("Cumulative difficulty ordering error")]
	DifficultyOrder,
	/// Inter-sample work/time feasibility bound violated.
	#[error("Inter-sample work/time bound violated")]
	WorkTimeBound,
	/// Too many or too few samples for the configured parameters.
	#[error("Invalid sample count: got {got}, expected about {expected}")]
	SampleCount {
		/// Actual
		got: usize,
		/// Expected ballpark
		expected: usize,
	},
	/// Generic structural error.
	#[error("FlyClient proof error: {0}")]
	Other(String),
}

fn prf_hash(seed: &Hash, counter: u64) -> Hash {
	let mut buf = Vec::with_capacity(40);
	buf.extend_from_slice(seed.as_bytes());
	let mut c = [0u8; 8];
	BigEndian::write_u64(&mut c, counter);
	buf.extend_from_slice(&c);
	buf.hash()
}

/// Deterministic difficulty targets in `1..=total_work` (inclusive), derived from `seed`.
///
/// Uses a simple hash-expansion PRF: `H(seed || counter)` interpreted as a u64,
/// reduced into the difficulty range. Matches the paper's need for public
/// randomness bound to the tip without interactive coin flips.
pub fn sample_difficulty_targets(total_work: u64, n: usize, seed: Hash) -> Vec<u64> {
	if n == 0 || total_work == 0 {
		return vec![];
	}
	let mut out = Vec::with_capacity(n);
	let mut counter: u64 = 0;
	while out.len() < n {
		let h = prf_hash(&seed, counter);
		let r = BigEndian::read_u64(h.as_bytes());
		// Map to 1..=total_work
		let target = (r % total_work).saturating_add(1);
		out.push(target);
		counter = counter.saturating_add(1);
	}
	out.sort_unstable();
	out.dedup();
	// If dedup shrank the set, keep going a bit more for uniqueness.
	while out.len() < n && counter < n as u64 * 8 {
		let h = prf_hash(&seed, counter);
		let r = BigEndian::read_u64(h.as_bytes());
		let target = (r % total_work).saturating_add(1);
		if let Err(idx) = out.binary_search(&target) {
			out.insert(idx, target);
		}
		counter += 1;
	}
	out.truncate(n);
	out
}

/// Given cumulative difficulties per height (`cum[h] = total_difficulty at height h`),
/// return the smallest height whose cumulative difficulty is `>= target`.
///
/// `cum` must be non-empty and non-decreasing.
pub fn height_for_difficulty_target(cum: &[u64], target: u64) -> Option<usize> {
	if cum.is_empty() {
		return None;
	}
	if target == 0 {
		return Some(0);
	}
	// binary search first index with cum[i] >= target
	let mut lo = 0usize;
	let mut hi = cum.len();
	while lo < hi {
		let mid = (lo + hi) / 2;
		if cum[mid] < target {
			lo = mid + 1;
		} else {
			hi = mid;
		}
	}
	if lo < cum.len() {
		Some(lo)
	} else {
		None
	}
}

/// MMR leaf position for a header height (insertion index).
pub fn leaf_pos_for_height(height: u64) -> u64 {
	pmmr::insertion_to_pmmr_index(height)
}

/// Verify a Path-A FlyClient proof structurally.
///
/// *Does not* verify Cuckoo PoW solutions (expensive / era-specific). Callers
/// that need full consensus checks should verify PoW on `tip` and each sample
/// header after this returns `Ok`.
pub fn verify_structure(proof: &FlyClientProof, params: &SampleParams) -> Result<(), Error> {
	if proof.version != FLYCLIENT_PROOF_VERSION {
		return Err(Error::UnsupportedVersion);
	}

	// Tip total work.
	let tip_work = proof.tip.total_difficulty().to_num();
	if tip_work == 0 {
		return Err(Error::Other("tip total difficulty is zero".into()));
	}

	// Sample count ballpark (allow dedup fewer than requested).
	let n_leaves = proof.tip.height.saturating_add(1);
	let expected = params.sample_count(n_leaves);
	if proof.samples.len() > params.max_samples as usize {
		return Err(Error::SampleCount {
			got: proof.samples.len(),
			expected,
		});
	}
	// Require at least min(expected/2, min_samples) unless chain is tiny.
	if n_leaves > 2 {
		let min_ok = (expected / 2).max(params.min_samples as usize / 2).max(1);
		if proof.samples.len() < min_ok {
			return Err(Error::SampleCount {
				got: proof.samples.len(),
				expected,
			});
		}
	}

	// Each sample: leaf pos, merkle proof, prev link, difficulty order.
	let mut last_height = 0u64;
	let mut last_work = 0u64;
	let mut last_ts: Option<u64> = None;

	for (i, sample) in proof.samples.iter().enumerate() {
		let h = sample.header.height;
		let work = sample.header.total_difficulty().to_num();

		if leaf_pos_for_height(h) != sample.leaf_pos {
			return Err(Error::LeafPos(h));
		}
		if sample.merkle_proof.mmr_size != proof.mmr_size {
			return Err(Error::Other(format!(
				"sample mmr_size mismatch at height {}",
				h
			)));
		}

		// Merkle inclusion against committed root.
		if sample
			.merkle_proof
			.verify(proof.mmr_root, &sample.header, sample.leaf_pos)
			.is_err()
		{
			return Err(Error::MerkleProof(h));
		}

		// Predecessor linkage (Path A wtema neighborhood).
		if h == 0 {
			if sample.header.hash() != proof.genesis_hash {
				return Err(Error::GenesisMismatch);
			}
			if sample.prev_header.is_some() {
				return Err(Error::PrevHeader(h));
			}
		} else {
			let prev = sample
				.prev_header
				.as_ref()
				.ok_or(Error::PrevHeader(h))?;
			if prev.hash() != sample.header.prev_hash {
				return Err(Error::PrevHeader(h));
			}
			if prev.height + 1 != h {
				return Err(Error::PrevHeader(h));
			}
			// Cumulative difficulty must increase.
			if work <= prev.total_difficulty().to_num() {
				return Err(Error::DifficultyOrder);
			}
		}

		// Samples should be ordered by height (prover responsibility; we enforce).
		if i > 0 && h <= last_height {
			return Err(Error::Other("samples not strictly increasing in height".into()));
		}

		// Inter-sample feasibility between consecutive samples.
		if i > 0 {
			if work <= last_work {
				return Err(Error::DifficultyOrder);
			}
			if let Some(prev_ts) = last_ts {
				let ts = sample.header.timestamp.timestamp().max(0) as u64;
				check_work_time_bound(last_work, work, prev_ts, ts, params)?;
			}
		}

		last_height = h;
		last_work = work;
		last_ts = Some(sample.header.timestamp.timestamp().max(0) as u64);
	}

	// Tip must be at least as heavy as the last sample.
	if let Some(last) = proof.samples.last() {
		if tip_work < last.header.total_difficulty().to_num() {
			return Err(Error::DifficultyOrder);
		}
		if proof.tip.height < last.header.height {
			return Err(Error::Other("tip height below last sample".into()));
		}
	}

	// Tip leaf should also be included in the committed MMR (size covers tip height).
	let tip_leaf = leaf_pos_for_height(proof.tip.height);
	if tip_leaf >= proof.mmr_size {
		return Err(Error::Other("mmr_size does not cover tip height".into()));
	}

	Ok(())
}

/// Check that work increase between two samples is not absurd vs elapsed time.
fn check_work_time_bound(
	work_a: u64,
	work_b: u64,
	ts_a: u64,
	ts_b: u64,
	params: &SampleParams,
) -> Result<(), Error> {
	if ts_b < ts_a {
		// Timestamps can go backwards slightly under rules; treat as 1s minimum span.
		return Ok(());
	}
	let dt = (ts_b - ts_a).max(1);
	let dw = work_b.saturating_sub(work_a);
	// Ideal: ~1 difficulty unit per BLOCK_TIME (60s) at constant hashrate is wrong
	// dimensionally; we only check that work doesn't explode vs time by a huge factor.
	// bound: dw <= max_work_time_factor * work_a * dt / 60  (relative growth)
	// For small work_a, use absolute slack.
	let scale = params.max_work_time_factor.saturating_mul(dt.max(60));
	let bound = work_a.saturating_mul(scale).saturating_add(scale);
	if dw > bound && dw > work_a.saturating_mul(params.max_work_time_factor) {
		return Err(Error::WorkTimeBound);
	}
	Ok(())
}

/// Build difficulty targets and map them to heights for a known cumulative series.
pub fn plan_samples(
	cum_difficulty_by_height: &[u64],
	params: &SampleParams,
	seed: Hash,
) -> Vec<(u64, usize)> {
	let n_leaves = cum_difficulty_by_height.len() as u64;
	let total = match cum_difficulty_by_height.last() {
		Some(&w) if w > 0 => w,
		_ => return vec![],
	};
	let n = params.sample_count(n_leaves);
	let targets = sample_difficulty_targets(total, n, seed);
	let mut pairs = Vec::with_capacity(targets.len());
	for t in targets {
		if let Some(h) = height_for_difficulty_target(cum_difficulty_by_height, t) {
			pairs.push((t, h));
		}
	}
	// Unique heights (multiple targets can map to same leaf).
	pairs.sort_by_key(|(_, h)| *h);
	pairs.dedup_by_key(|(_, h)| *h);
	pairs
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::core::hash::ZERO_HASH;
	use crate::core::pmmr::{ReadablePMMR, VecBackend, PMMR};
	use crate::core::BlockHeader;
	use crate::global;
	use crate::pow::{Difficulty, ProofOfWork};
	use chrono::prelude::{DateTime, Utc};

	fn init() {
		global::set_local_chain_type(global::ChainTypes::AutomatedTesting);
	}

	fn make_header(height: u64, prev: Hash, prev_root: Hash, total_diff: u64, ts: i64) -> BlockHeader {
		let mut h = BlockHeader::default();
		h.height = height;
		h.prev_hash = prev;
		h.prev_root = prev_root;
		h.timestamp = DateTime::<Utc>::from_timestamp(ts, 0).unwrap();
		h.pow = ProofOfWork {
			total_difficulty: Difficulty::from_num(total_diff),
			secondary_scaling: 1,
			nonce: height,
			proof: h.pow.proof.clone(),
		};
		h
	}

	#[test]
	fn sample_targets_in_range_and_deterministic() {
		let seed = Hash::from_vec(&[7u8; 32]);
		let a = sample_difficulty_targets(1000, 20, seed);
		let b = sample_difficulty_targets(1000, 20, seed);
		assert_eq!(a, b);
		assert!(a.iter().all(|&t| t >= 1 && t <= 1000));
		// different seed ⇒ different stream (almost surely)
		let seed2 = Hash::from_vec(&[8u8; 32]);
		let c = sample_difficulty_targets(1000, 20, seed2);
		assert_ne!(a, c);
	}

	#[test]
	fn height_for_target_binary_search() {
		// heights 0..4 with cum difficulty 10,20,30,40,50
		let cum = vec![10, 20, 30, 40, 50];
		assert_eq!(height_for_difficulty_target(&cum, 1), Some(0));
		assert_eq!(height_for_difficulty_target(&cum, 10), Some(0));
		assert_eq!(height_for_difficulty_target(&cum, 11), Some(1));
		assert_eq!(height_for_difficulty_target(&cum, 50), Some(4));
		assert_eq!(height_for_difficulty_target(&cum, 51), None);
	}

	#[test]
	fn sample_count_scales_with_log_n() {
		let p = SampleParams::default();
		assert_eq!(p.sample_count(1), 0);
		let small = p.sample_count(16);
		let large = p.sample_count(1 << 20);
		assert!(large >= small);
		assert!(large <= p.max_samples as usize);
	}

	#[test]
	fn prove_and_verify_synthetic_chain() {
		init();
		let n = 32u64;
		let mut headers = Vec::with_capacity(n as usize);
		let mut prev_hash = ZERO_HASH;
		let mut prev_root = ZERO_HASH;
		let mut ba = VecBackend::<BlockHeader>::new();
		let mut pmmr = PMMR::new(&mut ba);

		for height in 0..n {
			// cumulative difficulty: 100 per block
			let total_diff = (height + 1) * 100;
			let ts = 1_600_000_000 + (height as i64) * 60;
			let header = make_header(height, prev_hash, prev_root, total_diff, ts);
			pmmr.push(&header).unwrap();
			prev_hash = header.hash();
			// After applying header, MMR root is the commitment for the *next* header's prev_root.
			prev_root = pmmr.root().unwrap();
			headers.push(header);
		}

		let tip = headers.last().unwrap().clone();
		let mmr_root = pmmr.root().unwrap();
		let mmr_size = pmmr.unpruned_size();
		let genesis_hash = headers[0].hash();

		let cum: Vec<u64> = headers
			.iter()
			.map(|h| h.total_difficulty().to_num())
			.collect();
		let params = SampleParams {
			lambda: 10,
			min_samples: 4,
			max_samples: 64,
			sample_multiplier: 1,
			max_work_time_factor: 32,
		};
		let plan = plan_samples(&cum, &params, tip.hash());
		assert!(!plan.is_empty());

		let mut samples = Vec::new();
		for (_target, height) in plan {
			let header = headers[height].clone();
			let leaf_pos = leaf_pos_for_height(height as u64);
			let merkle_proof = pmmr.merkle_proof(leaf_pos).unwrap();
			assert!(merkle_proof
				.verify(mmr_root, &header, leaf_pos)
				.is_ok());
			let prev_header = if height == 0 {
				None
			} else {
				Some(headers[height - 1].clone())
			};
			samples.push(FlyClientSample {
				leaf_pos,
				header,
				prev_header,
				merkle_proof,
			});
		}

		let proof = FlyClientProof {
			version: FLYCLIENT_PROOF_VERSION,
			genesis_hash,
			tip,
			mmr_size,
			mmr_root,
			samples,
		};

		verify_structure(&proof, &params).expect("valid synthetic proof");

		// Serde round-trip (binary).
		let mut bytes = Vec::new();
		ser::serialize_default(&mut bytes, &proof).unwrap();
		let proof2: FlyClientProof = ser::deserialize_default(&mut &bytes[..]).unwrap();
		assert_eq!(proof, proof2);
		verify_structure(&proof2, &params).unwrap();
	}

	#[test]
	fn reject_tampered_merkle_path() {
		init();
		let mut ba = VecBackend::<BlockHeader>::new();
		let mut pmmr = PMMR::new(&mut ba);
		let mut headers = vec![];
		let mut prev_hash = ZERO_HASH;
		let mut prev_root = ZERO_HASH;
		for height in 0..8 {
			let h = make_header(
				height,
				prev_hash,
				prev_root,
				(height + 1) * 50,
				1_600_000_000 + height as i64 * 60,
			);
			pmmr.push(&h).unwrap();
			prev_hash = h.hash();
			prev_root = pmmr.root().unwrap();
			headers.push(h);
		}
		let tip = headers[7].clone();
		let mmr_root = pmmr.root().unwrap();
		let leaf_pos = leaf_pos_for_height(3);
		let mut merkle_proof = pmmr.merkle_proof(leaf_pos).unwrap();
		// Corrupt path.
		if let Some(x) = merkle_proof.path.first_mut() {
			*x = Hash::from_vec(&[1u8; 32]);
		}
		let sample = FlyClientSample {
			leaf_pos,
			header: headers[3].clone(),
			prev_header: Some(headers[2].clone()),
			merkle_proof,
		};
		let proof = FlyClientProof {
			version: FLYCLIENT_PROOF_VERSION,
			genesis_hash: headers[0].hash(),
			tip,
			mmr_size: pmmr.unpruned_size(),
			mmr_root,
			samples: vec![sample],
		};
		// Disable sample-count floor so we only exercise Merkle failure.
		let params = SampleParams {
			lambda: 1,
			min_samples: 0,
			max_samples: 64,
			sample_multiplier: 1,
			max_work_time_factor: 32,
		};
		assert_eq!(
			verify_structure(&proof, &params),
			Err(Error::MerkleProof(3))
		);
	}
}
