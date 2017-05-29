// Bitcoin secp256k1 bindings
// Written in 2014 by
//   Dawid Ciężarkiewicz
//   Andrew Poelstra
//
// To the extent possible under law, the author(s) have dedicated all
// copyright and related and neighboring rights to this software to
// the public domain worldwide. This software is distributed without
// any warranty.
//
// You should have received a copy of the CC0 Public Domain Dedication
// along with this software.
// If not, see <http://creativecommons.org/publicdomain/zero/1.0/>.
//

//! # Pedersen commitments and related range proofs

use std::cmp::min;
use std::fmt;
use std::mem;

use ContextFlag;
use Error;
use Secp256k1;

use constants;
use ffi;
use key;
use key::SecretKey;
use rand::{Rng, OsRng};
use serde::{ser, de};

/// A Pedersen commitment
pub struct Commitment(pub [u8; constants::PEDERSEN_COMMITMENT_SIZE]);
impl_array_newtype!(Commitment, u8, constants::PEDERSEN_COMMITMENT_SIZE);
impl_pretty_debug!(Commitment);


impl Commitment {
  /// Builds a Hash from a byte vector. If the vector is too short, it will be
  /// completed by zeroes. If it's too long, it will be truncated.
  pub fn from_vec(v: Vec<u8>) -> Commitment {
    let mut h = [0; constants::PEDERSEN_COMMITMENT_SIZE];
    for i in 0..min(v.len(), constants::PEDERSEN_COMMITMENT_SIZE) {
      h[i] = v[i];
    }
    Commitment(h)
  }
	/// Uninitialized commitment, use with caution
	unsafe fn blank() -> Commitment {
		mem::uninitialized()
	}
	/// Converts a commitment to a public key
	pub fn to_pubkey(&self, secp: &Secp256k1) -> Result<key::PublicKey, Error> {
		key::PublicKey::from_slice(secp, &self.0)
	}
}

/// A range proof. Typically much larger in memory that the above (~5k).
#[derive(Copy)]
pub struct RangeProof {
	/// The proof itself, at most 5134 bytes long
	pub proof: [u8; constants::MAX_PROOF_SIZE],
	/// The length of the proof
	pub plen: usize,
}

impl PartialEq for RangeProof {
	fn eq(&self, other: &Self) -> bool {
		self.proof.as_ref() == other.proof.as_ref()
	}
}

impl Clone for RangeProof {
	#[inline]
	fn clone(&self) -> RangeProof {
		unsafe {
			use std::intrinsics::copy_nonoverlapping;
			use std::mem;
			let mut ret: [u8; constants::MAX_PROOF_SIZE] = mem::uninitialized();
			copy_nonoverlapping(self.proof.as_ptr(),
			                    ret.as_mut_ptr(),
			                    mem::size_of::<RangeProof>());
			RangeProof {
				proof: ret,
				plen: self.plen,
			}
		}
	}
}

impl ser::Serialize for RangeProof {
	fn serialize<S>(&self, s: S) -> Result<S::Ok, S::Error>
		where S: ser::Serializer
	{
		(&self.proof[..self.plen]).serialize(s)
	}
}

struct Visitor;

impl<'di> de::Visitor<'di> for Visitor {
	type Value = RangeProof;

	fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
		formatter.write_str("an array of bytes")
	}

	#[inline]
	fn visit_seq<V>(self, mut v: V) -> Result<RangeProof, V::Error>
		where V: de::SeqAccess<'di>
	{
		unsafe {
			use std::mem;
			let mut ret: [u8; constants::MAX_PROOF_SIZE] = mem::uninitialized();
			let mut i = 0;
			while let Some(val) = v.next_element()? {
				ret[i] = val;
				i += 1;
			}
			Ok(RangeProof {
				proof: ret,
				plen: i,
			})
		}
	}
}

impl<'de> de::Deserialize<'de> for RangeProof {
	fn deserialize<D>(d: D) -> Result<RangeProof, D::Error>
		where D: de::Deserializer<'de>
	{

		// Begin actual function
		d.deserialize_seq(Visitor)
	}
}

impl AsRef<[u8]> for RangeProof {
	fn as_ref(&self) -> &[u8] {
		&self.proof[..self.plen as usize]
	}
}

impl RangeProof {
	pub fn zero() -> RangeProof {
		RangeProof {
			proof: [0; constants::MAX_PROOF_SIZE],
			plen: 0,
		}
	}
	/// The range proof as a byte slice.
	pub fn bytes(&self) -> &[u8] {
		&self.proof[..self.plen as usize]
	}
	/// Length of the range proof in bytes.
	pub fn len(&self) -> usize {
		self.plen
	}
}

/// The range that was proven
pub struct ProofRange {
	/// Min value that was proven
	pub min: u64,
	/// Max value that was proven
	pub max: u64,
}

/// Information about a valid proof after rewinding it.
pub struct ProofInfo {
	/// Whether the proof is valid or not
	pub success: bool,
	/// Value that was used by the commitment
	pub value: u64,
	/// Message embedded in the proof
	pub message: [u8; constants::PROOF_MSG_SIZE],
	/// Length of the embedded message
	pub mlen: i32,
	/// Min value that was proven
	pub min: u64,
	/// Max value that was proven
	pub max: u64,
	/// Exponent used by the proof
	pub exp: i32,
	/// Mantissa used by the proof
	pub mantissa: i32,
}

impl ::std::fmt::Debug for RangeProof {
	fn fmt(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
		try!(write!(f, "{}(", stringify!(RangeProof)));
		for i in self.proof[..self.plen].iter().cloned() {
			try!(write!(f, "{:02x}", i));
		}
		write!(f, ")[{}]", self.plen)
	}
}

impl Secp256k1 {
	/// Creates a pedersen commitment from a value and a blinding factor
	pub fn commit(&self, value: u64, blind: SecretKey) -> Result<Commitment, Error> {

		if self.caps != ContextFlag::Commit {
			return Err(Error::IncapableContext);
		}
		let mut commit = [0; 33];
		unsafe {
			ffi::secp256k1_pedersen_commit(self.ctx, commit.as_mut_ptr(), blind.as_ptr(), value)
		};
		Ok(Commitment(commit))
	}

	/// Convenience method to Create a pedersen commitment only from a value,
	/// with a zero blinding factor
	pub fn commit_value(&self, value: u64) -> Result<Commitment, Error> {

		if self.caps != ContextFlag::Commit {
			return Err(Error::IncapableContext);
		}
		let mut commit = [0; 33];
		let zblind = [0; 32];
		unsafe {
			ffi::secp256k1_pedersen_commit(self.ctx, commit.as_mut_ptr(), zblind.as_ptr(), value)
		};
		Ok(Commitment(commit))
	}

	/// Taking vectors of positive and negative commitments as well as an
	/// expected excess, verifies that it all sums to zero.
	pub fn verify_commit_sum(&self,
	                         positive: Vec<Commitment>,
	                         negative: Vec<Commitment>,
	                         excess: i64)
	                         -> bool {
		let pos = map_vec!(positive, |p| p.0.as_ptr());
		let neg = map_vec!(negative, |n| n.0.as_ptr());
		unsafe {
			ffi::secp256k1_pedersen_verify_tally(self.ctx,
			                                     pos.as_ptr(),
			                                     pos.len() as i32,
			                                     neg.as_ptr(),
			                                     neg.len() as i32,
			                                     excess) == 1
		}
	}

	/// Computes the sum of multiple positive and negative pedersen commitments.
	pub fn commit_sum(&self,
	                  positive: Vec<Commitment>,
	                  negative: Vec<Commitment>)
	                  -> Result<Commitment, Error> {
		let pos = map_vec!(positive, |p| p.0.as_ptr());
		let neg = map_vec!(negative, |n| n.0.as_ptr());
		let mut ret = unsafe { Commitment::blank() };
		let err = unsafe {
			ffi::secp256k1_pedersen_commit_sum(self.ctx,
			                                   ret.as_mut_ptr(),
			                                   pos.as_ptr(),
			                                   pos.len() as i32,
			                                   neg.as_ptr(),
			                                   neg.len() as i32)
		};
		if err == 1 {
			Ok(ret)
		} else {
			Err(Error::IncorrectCommitSum)
		}
	}

	/// Computes the sum of multiple positive and negative blinding factors.
	pub fn blind_sum(&self,
	                 positive: Vec<SecretKey>,
	                 negative: Vec<SecretKey>)
	                 -> Result<SecretKey, Error> {
		let mut neg = map_vec!(negative, |n| n.as_ptr());
		let mut all = map_vec!(positive, |p| p.as_ptr());
		all.append(&mut neg);
		let mut ret: [u8; 32] = unsafe { mem::uninitialized() };
		unsafe {
			assert_eq!(ffi::secp256k1_pedersen_blind_sum(self.ctx,
                                                         ret.as_mut_ptr(),
                                                         all.as_ptr(),
                                                         all.len() as i32,
                                                         positive.len() as i32),
                       1)
		}
		// secp256k1 should never return an invalid private
		SecretKey::from_slice(self, &ret)
	}

	/// Produces a range proof for the provided value, using min and max
	/// bounds, relying
	/// on the blinding factor and commitment.
	pub fn range_proof(&self,
	                   min: u64,
	                   value: u64,
	                   blind: SecretKey,
	                   commit: Commitment)
	                   -> RangeProof {

		let mut rng = OsRng::new().unwrap();
		let mut nonce = [0u8; 32];
		rng.fill_bytes(&mut nonce);

		let mut retried = false;
		let mut proof = [0; constants::MAX_PROOF_SIZE];
		let mut plen = constants::MAX_PROOF_SIZE as i32;
		loop {
			let err = unsafe {
				// because: "This can randomly fail with probability around one in 2^100.
				// If this happens, buy a lottery ticket and retry."
				ffi::secp256k1_rangeproof_sign(self.ctx,
				                               proof.as_mut_ptr(),
				                               &mut plen,
				                               min,
				                               commit.as_ptr(),
				                               blind.as_ptr(),
				                               nonce.as_ptr(),
				                               0,
				                               64,
				                               value)
			};
			if retried {
				break;
			}
			if err == 1 {
				retried = true;
			}
		}
		RangeProof {
			proof: proof,
			plen: plen as usize,
		}
	}

	/// Verify a proof that a committed value is within a range.
	pub fn verify_range_proof(&self,
	                          commit: Commitment,
	                          proof: RangeProof)
	                          -> Result<ProofRange, Error> {
		let mut min: u64 = 0;
		let mut max: u64 = 0;

		let success = unsafe {
			ffi::secp256k1_rangeproof_verify(self.ctx,
			                                 &mut min,
			                                 &mut max,
			                                 commit.as_ptr(),
			                                 proof.proof.as_ptr(),
			                                 proof.plen as i32) == 1
		};
		if success {
			Ok(ProofRange {
				min: min,
				max: max,
			})
		} else {
			Err(Error::InvalidRangeProof)
		}
	}

	/// Verify a range proof proof and rewind the proof to recover information
	/// sent by its author.
	pub fn rewind_range_proof(&self,
	                          commit: Commitment,
	                          proof: RangeProof,
	                          nonce: [u8; 32])
	                          -> ProofInfo {
		let mut value: u64 = 0;
		let mut blind: [u8; 32] = unsafe { mem::uninitialized() };
		let mut message = [0u8; constants::PROOF_MSG_SIZE];
		let mut mlen: i32 = 0;
		let mut min: u64 = 0;
		let mut max: u64 = 0;
		let success = unsafe {
			ffi::secp256k1_rangeproof_rewind(self.ctx,
			                                 blind.as_mut_ptr(),
			                                 &mut value,
			                                 message.as_mut_ptr(),
			                                 &mut mlen,
			                                 nonce.as_ptr(),
			                                 &mut min,
			                                 &mut max,
			                                 commit.as_ptr(),
			                                 proof.proof.as_ptr(),
			                                 proof.plen as i32) == 1
		};
		ProofInfo {
			success: success,
			value: value,
			message: message,
			mlen: mlen,
			min: min,
			max: max,
			exp: 0,
			mantissa: 0,
		}
	}

	/// General information extracted from a range proof. Does not provide any
	/// information about the value or the message (see rewind).
	pub fn range_proof_info(&self, proof: RangeProof) -> ProofInfo {
		let mut exp: i32 = 0;
		let mut mantissa: i32 = 0;
		let mut min: u64 = 0;
		let mut max: u64 = 0;
		let success = unsafe {
			ffi::secp256k1_rangeproof_info(self.ctx,
			                               &mut exp,
			                               &mut mantissa,
			                               &mut min,
			                               &mut max,
			                               proof.proof.as_ptr(),
			                               proof.plen as i32) == 1
		};
		ProofInfo {
			success: success,
			value: 0,
			message: [0; 4096],
			mlen: 0,
			min: min,
			max: max,
			exp: exp,
			mantissa: mantissa,
		}
	}
}
