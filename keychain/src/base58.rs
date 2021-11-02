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

// Rust Bitcoin Library
// Written in 2014 by
//   Andrew Poelstra <apoelstra@wpsoftware.net>
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

//! Base58 encoder and decoder

use digest::Digest;
use sha2::Sha256;
use std::{error, fmt, str};

use byteorder::{ByteOrder, LittleEndian};

/// Sha256dHash
fn sha256d_hash(data: &[u8]) -> [u8; 32] {
	let mut ret = [0; 32];
	let mut sha2 = Sha256::new();
	sha2.update(data);
	ret.copy_from_slice(sha2.finalize().as_slice());
	sha2 = Sha256::new();
	sha2.update(&ret);
	ret.copy_from_slice(sha2.finalize().as_slice());
	ret
}

#[inline]
pub fn into_le_low_u32(data: &[u8; 32]) -> u32 {
	LittleEndian::read_u64(&data[0..8]) as u32
}

/// An error that might occur during base58 decoding
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Error {
	/// Invalid character encountered
	BadByte(u8),
	/// Checksum was not correct (expected, actual)
	BadChecksum(u32, u32),
	/// The length (in bytes) of the object was not correct
	/// Note that if the length is excessively long the provided length may be
	/// an estimate (and the checksum step may be skipped).
	InvalidLength(usize),
	/// Version byte(s) were not recognized
	InvalidVersion(Vec<u8>),
	/// Checked data was less than 4 bytes
	TooShort(usize),
	/// Any other error
	Other(String),
}

impl fmt::Display for Error {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match *self {
			Error::BadByte(b) => write!(f, "invalid base58 character 0x{:x}", b),
			Error::BadChecksum(exp, actual) => write!(
				f,
				"base58ck checksum 0x{:x} does not match expected 0x{:x}",
				actual, exp
			),
			Error::InvalidLength(ell) => write!(f, "length {} invalid for this base58 type", ell),
			Error::InvalidVersion(ref v) => {
				write!(f, "version {:?} invalid for this base58 type", v)
			}
			Error::TooShort(_) => write!(f, "base58ck data not even long enough for a checksum"),
			Error::Other(ref s) => f.write_str(s),
		}
	}
}

impl error::Error for Error {
	fn cause(&self) -> Option<&dyn error::Error> {
		None
	}
	fn description(&self) -> &'static str {
		match *self {
			Error::BadByte(_) => "invalid b58 character",
			Error::BadChecksum(_, _) => "invalid b58ck checksum",
			Error::InvalidLength(_) => "invalid length for b58 type",
			Error::InvalidVersion(_) => "invalid version for b58 type",
			Error::TooShort(_) => "b58ck data less than 4 bytes",
			Error::Other(_) => "unknown b58 error",
		}
	}
}

static BASE58_CHARS: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

static BASE58_DIGITS: [Option<u8>; 128] = [
	None,
	None,
	None,
	None,
	None,
	None,
	None,
	None, // 0-7
	None,
	None,
	None,
	None,
	None,
	None,
	None,
	None, // 8-15
	None,
	None,
	None,
	None,
	None,
	None,
	None,
	None, // 16-23
	None,
	None,
	None,
	None,
	None,
	None,
	None,
	None, // 24-31
	None,
	None,
	None,
	None,
	None,
	None,
	None,
	None, // 32-39
	None,
	None,
	None,
	None,
	None,
	None,
	None,
	None, // 40-47
	None,
	Some(0),
	Some(1),
	Some(2),
	Some(3),
	Some(4),
	Some(5),
	Some(6), // 48-55
	Some(7),
	Some(8),
	None,
	None,
	None,
	None,
	None,
	None, // 56-63
	None,
	Some(9),
	Some(10),
	Some(11),
	Some(12),
	Some(13),
	Some(14),
	Some(15), // 64-71
	Some(16),
	None,
	Some(17),
	Some(18),
	Some(19),
	Some(20),
	Some(21),
	None, // 72-79
	Some(22),
	Some(23),
	Some(24),
	Some(25),
	Some(26),
	Some(27),
	Some(28),
	Some(29), // 80-87
	Some(30),
	Some(31),
	Some(32),
	None,
	None,
	None,
	None,
	None, // 88-95
	None,
	Some(33),
	Some(34),
	Some(35),
	Some(36),
	Some(37),
	Some(38),
	Some(39), // 96-103
	Some(40),
	Some(41),
	Some(42),
	Some(43),
	None,
	Some(44),
	Some(45),
	Some(46), // 104-111
	Some(47),
	Some(48),
	Some(49),
	Some(50),
	Some(51),
	Some(52),
	Some(53),
	Some(54), // 112-119
	Some(55),
	Some(56),
	Some(57),
	None,
	None,
	None,
	None,
	None, // 120-127
];

/// Decode base58-encoded string into a byte vector
pub fn from(data: &str) -> Result<Vec<u8>, Error> {
	// 11/15 is just over log_256(58)
	let mut scratch = vec![0u8; 1 + data.len() * 11 / 15];
	// Build in base 256
	for d58 in data.bytes() {
		// Compute "X = X * 58 + next_digit" in base 256
		if d58 as usize > BASE58_DIGITS.len() {
			return Err(Error::BadByte(d58));
		}
		let mut carry = match BASE58_DIGITS[d58 as usize] {
			Some(d58) => d58 as u32,
			None => {
				return Err(Error::BadByte(d58));
			}
		};
		for d256 in scratch.iter_mut().rev() {
			carry += *d256 as u32 * 58;
			*d256 = carry as u8;
			carry /= 256;
		}
		assert_eq!(carry, 0);
	}

	// Copy leading zeroes directly
	let mut ret: Vec<u8> = data
		.bytes()
		.take_while(|&x| x == BASE58_CHARS[0])
		.map(|_| 0)
		.collect();
	// Copy rest of string
	ret.extend(scratch.into_iter().skip_while(|&x| x == 0));
	Ok(ret)
}

/// Decode a base58check-encoded string
pub fn from_check(data: &str) -> Result<Vec<u8>, Error> {
	let mut ret: Vec<u8> = from(data)?;
	if ret.len() < 4 {
		return Err(Error::TooShort(ret.len()));
	}
	let ck_start = ret.len() - 4;
	let expected = sha256d_hash(&ret[..ck_start]);
	let expected = into_le_low_u32(&expected);
	let actual = LittleEndian::read_u32(&ret[ck_start..(ck_start + 4)]);
	if expected != actual {
		return Err(Error::BadChecksum(expected, actual));
	}

	ret.truncate(ck_start);
	Ok(ret)
}

fn encode_iter_utf8<I>(data: I) -> Vec<u8>
where
	I: Iterator<Item = u8> + Clone,
{
	let (len, _) = data.size_hint();

	// 7/5 is just over log_58(256)
	let mut ret = Vec::with_capacity(1 + len * 7 / 5);

	let mut leading_zero_count = 0;
	let mut leading_zeroes = true;
	// Build string in little endian with 0-58 in place of characters...
	for d256 in data {
		let mut carry = d256 as usize;
		if leading_zeroes && carry == 0 {
			leading_zero_count += 1;
		} else {
			leading_zeroes = false;
		}

		for ch in ret.iter_mut() {
			let new_ch = *ch as usize * 256 + carry;
			*ch = (new_ch % 58) as u8;
			carry = new_ch / 58;
		}
		while carry > 0 {
			ret.push((carry % 58) as u8);
			carry /= 58;
		}
	}

	// ... then reverse it and convert to chars
	for _ in 0..leading_zero_count {
		ret.push(0);
	}
	ret.reverse();
	for ch in ret.iter_mut() {
		*ch = BASE58_CHARS[*ch as usize];
	}
	ret
}

fn encode_iter<I>(data: I) -> String
where
	I: Iterator<Item = u8> + Clone,
{
	let ret = encode_iter_utf8(data);
	String::from_utf8(ret).unwrap()
}

/// Directly encode a slice as base58 into a `Formatter`.
fn _encode_iter_to_fmt<I>(fmt: &mut fmt::Formatter<'_>, data: I) -> fmt::Result
where
	I: Iterator<Item = u8> + Clone,
{
	let ret = encode_iter_utf8(data);
	fmt.write_str(str::from_utf8(&ret).unwrap())
}

/// Directly encode a slice as base58
pub fn _encode_slice(data: &[u8]) -> String {
	encode_iter(data.iter().cloned())
}

/// Obtain a string with the base58check encoding of a slice
/// (Tack the first 4 256-digits of the object's Bitcoin hash onto the end.)
pub fn check_encode_slice(data: &[u8]) -> String {
	let checksum = sha256d_hash(&data);
	encode_iter(data.iter().cloned().chain(checksum[0..4].iter().cloned()))
}

/// Obtain a string with the base58check encoding of a slice
/// (Tack the first 4 256-digits of the object's Bitcoin hash onto the end.)
pub fn _check_encode_slice_to_fmt(fmt: &mut fmt::Formatter<'_>, data: &[u8]) -> fmt::Result {
	let checksum = sha256d_hash(&data);
	let iter = data.iter().cloned().chain(checksum[0..4].iter().cloned());
	_encode_iter_to_fmt(fmt, iter)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::util::from_hex;

	#[test]
	fn test_base58_encode() {
		// Basics
		assert_eq!(&_encode_slice(&[0][..]), "1");
		assert_eq!(&_encode_slice(&[1][..]), "2");
		assert_eq!(&_encode_slice(&[58][..]), "21");
		assert_eq!(&_encode_slice(&[13, 36][..]), "211");

		// Leading zeroes
		assert_eq!(&_encode_slice(&[0, 13, 36][..]), "1211");
		assert_eq!(&_encode_slice(&[0, 0, 0, 0, 13, 36][..]), "1111211");

		// Addresses
		let addr = from_hex("00f8917303bfa8ef24f292e8fa1419b20460ba064d").unwrap();
		assert_eq!(
			&check_encode_slice(&addr[..]),
			"1PfJpZsjreyVrqeoAfabrRwwjQyoSQMmHH"
		);
	}

	#[test]
	fn test_base58_decode() {
		// Basics
		assert_eq!(from("1").ok(), Some(vec![0u8]));
		assert_eq!(from("2").ok(), Some(vec![1u8]));
		assert_eq!(from("21").ok(), Some(vec![58u8]));
		assert_eq!(from("211").ok(), Some(vec![13u8, 36]));

		// Leading zeroes
		assert_eq!(from("1211").ok(), Some(vec![0u8, 13, 36]));
		assert_eq!(from("111211").ok(), Some(vec![0u8, 0, 0, 13, 36]));

		// Addresses
		assert_eq!(
			from_check("1PfJpZsjreyVrqeoAfabrRwwjQyoSQMmHH").ok(),
			Some(from_hex("00f8917303bfa8ef24f292e8fa1419b20460ba064d").unwrap())
		)
	}

	#[test]
	fn test_base58_roundtrip() {
		let s = "xprv9wTYmMFdV23N2TdNG573QoEsfRrWKQgWeibmLntzniatZvR9BmLnvSxqu53Kw1UmYPxLgboyZQaXwTCg8MSY3H2EU4pWcQDnRnrVA1xe8fs";
		let v: Vec<u8> = from_check(s).unwrap();
		assert_eq!(check_encode_slice(&v[..]), s);
		assert_eq!(from_check(&check_encode_slice(&v[..])).ok(), Some(v));
	}
}
