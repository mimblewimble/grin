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

//! Serialization and deserialization layer specialized for binary encoding.
//! Ensures consistency and safety. Basically a minimal subset or
//! rustc_serialize customized for our need.
//!
//! To use it simply implement `Writeable` or `Readable` and then use the
//! `serialize` or `deserialize` functions on them as appropriate.

use std::{error, fmt, cmp};
use std::io::{self, Write, Read};
use byteorder::{ByteOrder, ReadBytesExt, BigEndian};
use secp::pedersen::Commitment;
use secp::pedersen::RangeProof;
use secp::constants::PEDERSEN_COMMITMENT_SIZE;
use secp::constants::MAX_PROOF_SIZE;

/// Possible errors deriving from serializing or deserializing.
#[derive(Debug)]
pub enum Error {
	/// Wraps an io error produced when reading or writing
	IOErr(io::Error),
	/// Expected a given value that wasn't found
	UnexpectedData {
		/// What we wanted
		expected: Vec<u8>,
		/// What we got
		received: Vec<u8>,
	},
	/// Data wasn't in a consumable format
	CorruptedData,
	/// When asked to read too much data
	TooLargeReadErr,
}

impl From<io::Error> for Error {
	fn from(e: io::Error) -> Error {
		Error::IOErr(e)
	}
}

impl fmt::Display for Error {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		match *self {
			Error::IOErr(ref e) => write!(f, "{}", e),
			Error::UnexpectedData { expected: ref e, received: ref r } => {
				write!(f, "expected {:?}, got {:?}", e, r)
			}
			Error::CorruptedData => f.write_str("corrupted data"),
			Error::TooLargeReadErr => f.write_str("too large read"),
		}
	}
}

impl error::Error for Error {
	fn cause(&self) -> Option<&error::Error> {
		match *self {
			Error::IOErr(ref e) => Some(e),
			_ => None,
		}
	}

	fn description(&self) -> &str {
		match *self {
			Error::IOErr(ref e) => error::Error::description(e),
			Error::UnexpectedData { expected: _, received: _ } => "unexpected data",
			Error::CorruptedData => "corrupted data",
			Error::TooLargeReadErr => "too large read",
		}
	}
}

/// Signal to a serializable object how much of its data should be serialized
#[derive(Copy, Clone, PartialEq, Eq)]
pub enum SerializationMode {
	/// Serialize everything sufficiently to fully reconstruct the object
	Full,
	/// Serialize the data that defines the object
	Hash,
	/// Serialize everything that a signer of the object should know
	SigHash,
}

/// Implementations defined how different numbers and binary structures are
/// written to an underlying stream or container (depending on implementation).
pub trait Writer {
	/// The mode this serializer is writing in
	fn serialization_mode(&self) -> SerializationMode;

	/// Writes a u8 as bytes
	fn write_u8(&mut self, n: u8) -> Result<(), Error> {
		self.write_fixed_bytes(&[n])
	}

	/// Writes a u16 as bytes
	fn write_u16(&mut self, n: u16) -> Result<(), Error> {
		let mut bytes = [0; 2];
		BigEndian::write_u16(&mut bytes, n);
		self.write_fixed_bytes(&bytes)
	}

	/// Writes a u32 as bytes
	fn write_u32(&mut self, n: u32) -> Result<(), Error> {
		let mut bytes = [0; 4];
		BigEndian::write_u32(&mut bytes, n);
		self.write_fixed_bytes(&bytes)
	}

	/// Writes a u64 as bytes
	fn write_u64(&mut self, n: u64) -> Result<(), Error> {
		let mut bytes = [0; 8];
		BigEndian::write_u64(&mut bytes, n);
		self.write_fixed_bytes(&bytes)
	}

	/// Writes a i64 as bytes
	fn write_i64(&mut self, n: i64) -> Result<(), Error> {
		let mut bytes = [0; 8];
		BigEndian::write_i64(&mut bytes, n);
		self.write_fixed_bytes(&bytes)
	}

	/// Writes a variable number of bytes. The length is encoded as a 64-bit
	/// prefix.
	fn write_bytes<T: AsFixedBytes>(&mut self, bytes: &T) -> Result<(), Error> {
		try!(self.write_u64(bytes.as_ref().len() as u64));
		self.write_fixed_bytes(bytes)
	}

	/// Writes a fixed number of bytes from something that can turn itself into
	/// a `&[u8]`. The reader is expected to know the actual length on read.
	fn write_fixed_bytes<T: AsFixedBytes>(&mut self, fixed: &T) -> Result<(), Error>;
}

/// Implementations defined how different numbers and binary structures are
/// read from an underlying stream or container (depending on implementation).
pub trait Reader {
	/// Read a u8 from the underlying Read
	fn read_u8(&mut self) -> Result<u8, Error>;
	/// Read a u16 from the underlying Read
	fn read_u16(&mut self) -> Result<u16, Error>;
	/// Read a u32 from the underlying Read
	fn read_u32(&mut self) -> Result<u32, Error>;
	/// Read a u64 from the underlying Read
	fn read_u64(&mut self) -> Result<u64, Error>;
	/// Read a i32 from the underlying Read
	fn read_i64(&mut self) -> Result<i64, Error>;
	/// first before the data bytes.
	fn read_vec(&mut self) -> Result<Vec<u8>, Error>;
	/// first before the data bytes limited to max bytes.
	fn read_limited_vec(&mut self, max: usize) -> Result<Vec<u8>, Error>;
	/// Read a fixed number of bytes from the underlying reader.
	fn read_fixed_bytes(&mut self, length: usize) -> Result<Vec<u8>, Error>;
	/// Consumes a byte from the reader, producing an error if it doesn't have
	/// the expected value
	fn expect_u8(&mut self, val: u8) -> Result<u8, Error>;
}

/// Trait that every type that can be serialized as binary must implement.
/// Writes directly to a Writer, a utility type thinly wrapping an
/// underlying Write implementation.
pub trait Writeable {
	/// Write the data held by this Writeable to the provided writer
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), Error>;
}

/// Trait that every type that can be deserialized from binary must implement.
/// Reads directly to a Reader, a utility type thinly wrapping an
/// underlying Read implementation.
pub trait Readable
	where Self: Sized
{
	/// Reads the data necessary to this Readable from the provided reader
	fn read(reader: &mut Reader) -> Result<Self, Error>;
}

/// Deserializes a Readeable from any std::io::Read implementation.
pub fn deserialize<T: Readable>(mut source: &mut Read) -> Result<T, Error> {
	let mut reader = BinReader { source: source };
	T::read(&mut reader)
}

/// Serializes a Writeable into any std::io::Write implementation.
pub fn serialize<W: Writeable>(mut sink: &mut Write, thing: &W) -> Result<(), Error> {
	let mut writer = BinWriter { sink: sink };
	thing.write(&mut writer)
}

/// Utility function to serialize a writeable directly in memory using a
/// Vec<u8>.
pub fn ser_vec<W: Writeable>(thing: &W) -> Result<Vec<u8>, Error> {
	let mut vec = Vec::new();
	try!(serialize(&mut vec, thing));
	Ok(vec)
}

struct BinReader<'a> {
	source: &'a mut Read,
}

/// Utility wrapper for an underlying byte Reader. Defines higher level methods
/// to read numbers, byte vectors, hashes, etc.
impl<'a> Reader for BinReader<'a> {
	fn read_u8(&mut self) -> Result<u8, Error> {
		self.source.read_u8().map_err(Error::IOErr)
	}
	fn read_u16(&mut self) -> Result<u16, Error> {
		self.source.read_u16::<BigEndian>().map_err(Error::IOErr)
	}
	fn read_u32(&mut self) -> Result<u32, Error> {
		self.source.read_u32::<BigEndian>().map_err(Error::IOErr)
	}
	fn read_u64(&mut self) -> Result<u64, Error> {
		self.source.read_u64::<BigEndian>().map_err(Error::IOErr)
	}
	fn read_i64(&mut self) -> Result<i64, Error> {
		self.source.read_i64::<BigEndian>().map_err(Error::IOErr)
	}
	/// Read a variable size vector from the underlying Read. Expects a usize
	fn read_vec(&mut self) -> Result<Vec<u8>, Error> {
		let len = try!(self.read_u64());
		self.read_fixed_bytes(len as usize)
	}
	/// Read limited variable size vector from the underlying Read. Expects a
	/// usize
	fn read_limited_vec(&mut self, max: usize) -> Result<Vec<u8>, Error> {
		let len = cmp::min(max, try!(self.read_u64()) as usize);
		self.read_fixed_bytes(len as usize)
	}
	fn read_fixed_bytes(&mut self, length: usize) -> Result<Vec<u8>, Error> {
		// not reading more than 100k in a single read
		if length > 100000 {
			return Err(Error::TooLargeReadErr);
		}
		let mut buf = vec![0; length];
		self.source.read_exact(&mut buf).map(move |_| buf).map_err(Error::IOErr)
	}

	fn expect_u8(&mut self, val: u8) -> Result<u8, Error> {
		let b = try!(self.read_u8());
		if b == val {
			Ok(b)
		} else {
			Err(Error::UnexpectedData {
				expected: vec![val],
				received: vec![b],
			})
		}
	}
}


impl Readable for Commitment {
	fn read(reader: &mut Reader) -> Result<Commitment, Error> {
		let a = try!(reader.read_fixed_bytes(PEDERSEN_COMMITMENT_SIZE));
		let mut c = [0; PEDERSEN_COMMITMENT_SIZE];
		for i in 0..PEDERSEN_COMMITMENT_SIZE {
			c[i] = a[i];
		}
		Ok(Commitment(c))
	}
}

impl Readable for RangeProof {
	fn read(reader: &mut Reader) -> Result<RangeProof, Error> {
		let p = try!(reader.read_limited_vec(MAX_PROOF_SIZE));
		let mut a = [0; MAX_PROOF_SIZE];
		for i in 0..p.len() {
			a[i] = p[i];
		}
		Ok(RangeProof {
			proof: a,
			plen: p.len(),
		})
	}
}

/// Utility wrapper for an underlying byte Writer. Defines higher level methods
/// to write numbers, byte vectors, hashes, etc.
struct BinWriter<'a> {
	sink: &'a mut Write,
}

impl<'a> Writer for BinWriter<'a> {
	fn serialization_mode(&self) -> SerializationMode {
		SerializationMode::Full
	}

	fn write_fixed_bytes<T: AsFixedBytes>(&mut self, fixed: &T) -> Result<(), Error> {
		let bs = fixed.as_ref();
		try!(self.sink.write_all(bs));
		Ok(())
	}
}

macro_rules! impl_int {
    ($int: ty, $w_fn: ident, $r_fn: ident) => {
        impl Writeable for $int {
            fn write<W: Writer>(&self, writer: &mut W) -> Result<(), Error> {
                writer.$w_fn(*self)
            }
        }

        impl Readable for $int {
            fn read(reader: &mut Reader) -> Result<$int, Error> {
                reader.$r_fn()
            }
        }
    }
}

impl_int!(u8, write_u8, read_u8);
impl_int!(u16, write_u16, read_u16);
impl_int!(u32, write_u32, read_u32);
impl_int!(u64, write_u64, read_u64);
impl_int!(i64, write_i64, read_i64);

impl<'a, A: Writeable> Writeable for &'a A {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), Error> {
		Writeable::write(*self, writer)
	}
}

impl<A: Writeable, B: Writeable> Writeable for (A, B) {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), Error> {
		try!(Writeable::write(&self.0, writer));
		Writeable::write(&self.1, writer)
	}
}

impl<A: Readable, B: Readable> Readable for (A, B) {
	fn read(reader: &mut Reader) -> Result<(A, B), Error> {
		Ok((try!(Readable::read(reader)), try!(Readable::read(reader))))
	}
}

impl<A: Writeable, B: Writeable, C: Writeable> Writeable for (A, B, C) {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), Error> {
		try!(Writeable::write(&self.0, writer));
		try!(Writeable::write(&self.1, writer));
		Writeable::write(&self.2, writer)
	}
}

impl<A: Writeable, B: Writeable, C: Writeable, D: Writeable> Writeable for (A, B, C, D) {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), Error> {
		try!(Writeable::write(&self.0, writer));
		try!(Writeable::write(&self.1, writer));
		try!(Writeable::write(&self.2, writer));
		Writeable::write(&self.3, writer)
	}
}

impl<A: Readable, B: Readable, C: Readable> Readable for (A, B, C) {
	fn read(reader: &mut Reader) -> Result<(A, B, C), Error> {
		Ok((try!(Readable::read(reader)),
		    try!(Readable::read(reader)),
		    try!(Readable::read(reader))))
	}
}

impl<A: Readable, B: Readable, C: Readable, D: Readable> Readable for (A, B, C, D) {
	fn read(reader: &mut Reader) -> Result<(A, B, C, D), Error> {
		Ok((try!(Readable::read(reader)),
		    try!(Readable::read(reader)),
		    try!(Readable::read(reader)),
		    try!(Readable::read(reader))))
	}
}

impl Writeable for [u8; 4] {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), Error> {
		writer.write_bytes(self)
	}
}

/// Useful marker trait on types that can be sized byte slices
pub trait AsFixedBytes: Sized + AsRef<[u8]> {
	fn len(&self) -> usize;
}

impl<'a> AsFixedBytes for &'a [u8] {
	fn len(&self) -> usize {
		return 1;
	}
}
impl AsFixedBytes for Vec<u8> {
	fn len(&self) -> usize {
		return self.len();
	}
}
impl AsFixedBytes for [u8; 1] {
	fn len(&self) -> usize {
		return 1;
	}
}
impl AsFixedBytes for [u8; 2] {
	fn len(&self) -> usize {
		return 2;
	}
}
impl AsFixedBytes for [u8; 4] {
	fn len(&self) -> usize {
		return 4;
	}
}
impl AsFixedBytes for [u8; 8] {
	fn len(&self) -> usize {
		return 8;
	}
}
impl AsFixedBytes for [u8; 32] {
	fn len(&self) -> usize {
		return 32;
	}
}
impl AsFixedBytes for String {
	fn len(&self) -> usize {
		return self.len();
	}
}
impl AsFixedBytes for ::core::hash::Hash {
	fn len(&self) -> usize {
		return 32;
	}
}
impl AsFixedBytes for ::secp::pedersen::RangeProof {
	fn len(&self) -> usize {
		return self.plen;
	}
}
impl AsFixedBytes for ::secp::key::SecretKey {
	fn len(&self) -> usize {
		return 1;
	}
}
impl AsFixedBytes for ::secp::Signature {
	fn len(&self) -> usize {
		return 64;
	}
}
impl AsFixedBytes for ::secp::pedersen::Commitment {
	fn len(&self) -> usize {
		return PEDERSEN_COMMITMENT_SIZE;
	}
}
