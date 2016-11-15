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

//! Primary hash function used in the protocol
//!

use byteorder::{ByteOrder, BigEndian};
use std::fmt;
use std::ops::{Add, Sub, Shl, Shr, Index, IndexMut};
use tiny_keccak::Keccak;

use ser::{self, Reader, Writer, Writeable, Readable};

/// A Bitcoin-style target, implemented as a 32 bytes positive big number that
/// can be compared against a proof of work. Serializes in a compact format
/// composed of a one-byte exponent and a 4 bytes mantissa. Hence a target will
/// only ever have a u32 worth of significant numbers.
#[derive(Debug, Copy, Clone, PartialEq, PartialOrd, Eq, Ord)]
pub struct Target(pub [u8; 32]);

impl fmt::Display for Target {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		for i in self.0[..].iter().cloned() {
			try!(write!(f, "{:02x}", i));
		}
		Ok(())
	}
}

impl Writeable for Target {
	fn write(&self, writer: &mut Writer) -> Result<(), ser::Error> {
		let (exp, mantissa) = self.split();
		try!(writer.write_u8(exp));
		writer.write_u32(mantissa)
	}
}

impl Readable<Target> for Target {
	fn read(reader: &mut Reader) -> Result<Target, ser::Error> {
		let (exp, mantissa) = ser_multiread!(reader, read_u8, read_u32);
		Target::join(exp, mantissa)
	}
}

impl Target {
	/// Takes a u32 mantissa, bringing it to the provided exponent to build a
	/// new target.
	fn join(exp: u8, mantissa: u32) -> Result<Target, ser::Error> {
		if exp > 192 {
			return Err(ser::Error::CorruptedData);
		}
		let mut t = [0; 32];
		t[31] = (mantissa & 0xff) as u8;
		t[30] = (mantissa >> 8) as u8;
		t[29] = (mantissa >> 16) as u8;
		t[28] = (mantissa >> 24) as u8;
		Ok(Target(t) << (exp as usize))
	}

	/// Splits the target into an exponent and a mantissa with most significant
	/// numbers. The precision is one of a u32.
	fn split(&self) -> (u8, u32) {
		let mut exp = 0;
		let mut mantissa = self.clone();
		let max_target = Target::join(32, 1).unwrap();
		while mantissa > max_target {
			exp += 1;
			mantissa = mantissa >> 1;
		}
		let mut res = mantissa[31] as u32;
		res += (mantissa[30] as u32) << 8;
		res += (mantissa[29] as u32) << 16;
		res += (mantissa[28] as u32) << 24;
		(exp, res)
	}
}

impl Index<usize> for Target {
	type Output = u8;
	fn index(&self, idx: usize) -> &u8 {
		&self.0[idx]
	}
}

impl IndexMut<usize> for Target {
	fn index_mut(&mut self, idx: usize) -> &mut u8 {
		&mut self.0[idx]
	}
}

/// Implements shift left to break the target down into an exponent and a
/// mantissa and provide simple multiplication by a power of 2.
impl Shl<usize> for Target {
	type Output = Target;

	fn shl(self, shift: usize) -> Target {
		let Target(ref t) = self;
		let mut ret = [0; 32];
		let byte_shift = shift / 8;
		let bit_shift = shift % 8;

		// shift
		for i in byte_shift..32 {
			ret[i - byte_shift] = t[i] << bit_shift;
		}
		// carry
		if bit_shift > 0 {
			for i in byte_shift + 1..32 {
				let s = t[i] >> (8 - bit_shift);
				ret[i - 1 - byte_shift] += s;
			}
		}
		Target(ret)
	}
}

/// Implements shift right to build a target from an exponent and a mantissa
/// and provide simple division by a power of 2.
impl Shr<usize> for Target {
	type Output = Target;

	fn shr(self, shift: usize) -> Target {
		let Target(ref t) = self;
		let mut ret = [0; 32];
		let byte_shift = shift / 8;
		let bit_shift = shift % 8;

		// shift
		for i in byte_shift..32 {
			let (s, _) = t[i - byte_shift].overflowing_shr(bit_shift as u32);
			ret[i] = s;
		}
		// Carry
		if bit_shift > 0 {
			for i in byte_shift + 1..32 {
				ret[i] += t[i - byte_shift - 1] << (8 - bit_shift);
			}
		}
		Target(ret)
	}
}

/// Implement addition between targets. Overflow is truncated.
impl Add for Target {
	type Output = Target;

	fn add(self, other: Target) -> Target {
		let mut sum = [0; 32];
		let mut carry = 0;
		for i in (0..32).rev() {
			let (sum_i, carry_i) = add_with_carryover(self[i], other[i], carry);
			sum[i] = sum_i;
			carry = carry_i;
		}
		Target(sum)
	}
}

fn add_with_carryover(a: u8, b: u8, carry: u8) -> (u8, u8) {
	let mut new_carry = 0;
	let (a_carry, over) = a.overflowing_add(carry);
	if over {
		new_carry += 1;
	}
	let (sum, over) = a_carry.overflowing_add(b);
	if over {
		new_carry += 1;
	}
	(sum, new_carry)
}

/// Implement subtractions between targets. Underflow is truncated.
impl Sub for Target {
	type Output = Target;

	fn sub(self, other: Target) -> Target {
		let mut diff = [0; 32];
		let mut carry = 0;
		for i in (0..32).rev() {
			let (diff_i, carry_i) = sub_with_carryover(self[i], other[i], carry);
			diff[i] = diff_i;
			carry = carry_i;
		}
		Target(diff)
	}
}

fn sub_with_carryover(a: u8, b: u8, carry: u8) -> (u8, u8) {
	let mut new_carry = 0;
	let (a_carry, under) = a.overflowing_sub(carry);
	if under {
		new_carry += 1;
	}
	let (diff, under) = a_carry.overflowing_sub(b);
	if under {
		new_carry += 1;
	}
	(diff, new_carry)
}

#[cfg(test)]
mod test {
	use super::*;

	#[test]
	fn small_shift_left_right() {
		let mut arr = [0; 32];
		arr[31] = 128;
		let t = Target(arr);
		let tsl = t << 2;
		assert_eq!(tsl[31], 0);
		assert_eq!(tsl[30], 2);
		assert_eq!(tsl[29], 0);
		let rsl = tsl >> 2;
		assert_eq!(rsl[31], 128);
		assert_eq!(rsl[30], 0);
		assert_eq!(rsl[0], 0);
	}

	#[test]
	fn shift_byte_left_right() {
		let mut arr = [0; 32];
		arr[31] = 64;
		let t = Target(arr);
		let tsl = t << 7 * 8 + 1;
		assert_eq!(tsl[31], 0);
		assert_eq!(tsl[24], 128);
		let rsl = tsl >> 5 * 8 + 1;
		assert_eq!(rsl[29], 64);
		assert_eq!(rsl[24], 0);
		assert_eq!(rsl[31], 0);
	}

	#[test]
	fn shift_truncate() {
		assert_eq!((Target::join(0, 0xffff).unwrap() >> 8) << 8,
		           Target::join(0, 0xff00).unwrap());
	}

	#[test]
	fn split_fit() {
		let t = Target::join(10 * 8, ::std::u32::MAX).unwrap();
		let (exp, mant) = t.split();
		assert_eq!(exp, 10 * 8);
		assert_eq!(mant, ::std::u32::MAX);
	}

	#[test]
	fn split_nofit() {
		let mut t = Target::join(10 * 8, 255).unwrap();
		t[0] = 10;
		t[25] = 17;
		let (exp, mant) = t.split();
		assert_eq!(exp, 220);
		assert_eq!(mant, 10 << 28);
	}

	#[test]
	fn addition() {
		assert_eq!(Target::join(0, 10).unwrap() + Target::join(0, 20).unwrap(),
		           Target::join(0, 30).unwrap());
		// single overflow
		assert_eq!(Target::join(0, 250).unwrap() + Target::join(0, 250).unwrap(),
		           Target::join(0, 500).unwrap());
		// multiple overflows
		assert_eq!(Target::join(0, 300).unwrap() + Target::join(0, 300).unwrap(),
		           Target::join(0, 600).unwrap());
		assert_eq!(Target::join(10, 300).unwrap() + Target::join(10, 300).unwrap(),
		           Target::join(10, 600).unwrap());
		// cascading overflows
		assert_eq!(Target::join(8, 0xffff).unwrap() + Target::join(8, 0xffff).unwrap(),
		           Target::join(8, 0x1fffe).unwrap());
	}

	#[test]
	fn subtraction() {
		assert_eq!(Target::join(0, 40).unwrap() - Target::join(0, 10).unwrap(),
		           Target::join(0, 30).unwrap());
		assert_eq!(Target::join(0, 300).unwrap() - Target::join(0, 100).unwrap(),
		           Target::join(0, 200).unwrap());
		assert_eq!(Target::join(0, 0xffff).unwrap() - Target::join(0, 0xffff).unwrap(),
		           Target::join(0, 0).unwrap());
		assert_eq!(Target::join(0, 0xffff).unwrap() - Target::join(0, 0xff01).unwrap(),
		           Target::join(0, 0xfe).unwrap());
	}
}
