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

use std::{error, fmt};
use std::io::{self, Write, Read};
use byteorder::{ReadBytesExt, WriteBytesExt, BigEndian};

/// Possible errors deriving from serializing or deserializing.
#[derive(Debug)]
pub enum Error {
	/// Wraps an io error produced when reading or writing
	IOErr(io::Error),
	/// Expected a given value that wasn't found
	UnexpectedData {
		expected: Vec<u8>,
		received: Vec<u8>,
	},
	/// Data wasn't in a consumable format
	CorruptedData,
	/// When asked to read too much data
	TooLargeReadErr(String),
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
			Error::UnexpectedData { expected: ref e, received: ref r } => write!(f, "expected {:?}, got {:?}", e, r),
			Error::CorruptedData => f.write_str("corrupted data"),
			Error::TooLargeReadErr(ref s) => f.write_str(&s)
		}
	}
}

impl error::Error for Error {
	fn cause(&self) -> Option<&error::Error> {
		match *self {
			Error::IOErr(ref e) => Some(e),
			_ => None
		}
	}

	fn description(&self) -> &str {
		match *self {
			Error::IOErr(ref e) => error::Error::description(e),
			Error::UnexpectedData { expected: _, received: _ } => "unexpected data",
			Error::CorruptedData => "corrupted data",
			Error::TooLargeReadErr(ref s) => s
		}
	}
}

/// Useful trait to implement on types that can be translated to byte slices
/// directly. Allows the use of `write_fixed_bytes` on them.
pub trait AsFixedBytes {
	/// The slice representation of self
	fn as_fixed_bytes(&self) -> &[u8];
}

/// Implementations defined how different numbers and binary structures are
/// written to an underlying stream or container (depending on implementation).
pub trait Writer {
	/// Writes a u8 as bytes
	fn write_u8(&mut self, n: u8) -> Result<(), Error>;
	/// Writes a u16 as bytes
	fn write_u16(&mut self, n: u16) -> Result<(), Error>;
	/// Writes a u32 as bytes
	fn write_u32(&mut self, n: u32) -> Result<(), Error>;
	/// Writes a u64 as bytes
	fn write_u64(&mut self, n: u64) -> Result<(), Error>;
	/// Writes a i64 as bytes
	fn write_i64(&mut self, n: i64) -> Result<(), Error>;
	/// Writes a variable length `Vec`, the length of the `Vec` is encoded as a
	/// prefix.
	fn write_bytes(&mut self, data: &[u8]) -> Result<(), Error>;
	/// Writes a fixed number of bytes from something that can turn itself into
	/// a `&[u8]`. The reader is expected to know the actual length on read.
	fn write_fixed_bytes(&mut self, b32: &AsFixedBytes) -> Result<(), Error>;
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
	/// Read a fixed number of bytes from the underlying reader.
	fn read_fixed_bytes(&mut self, length: usize) -> Result<Vec<u8>, Error>;
	/// Convenience function to read 32 fixed bytes
	fn read_32_bytes(&mut self) -> Result<Vec<u8>, Error>;
	/// Convenience function to read 33 fixed bytes
	fn read_33_bytes(&mut self) -> Result<Vec<u8>, Error>;
	/// Consumes a byte from the reader, producing an error if it doesn't have
	/// the expected value
	fn expect_u8(&mut self, val: u8) -> Result<u8, Error>;
}

/// Trait that every type that can be serialized as binary must implement.
/// Writes directly to a Writer, a utility type thinly wrapping an
/// underlying Write implementation.
pub trait Writeable {
	/// Write the data held by this Writeable to the provided writer
	fn write(&self, writer: &mut Writer) -> Result<(), Error>;
}

/// Trait that every type that can be deserialized from binary must implement.
/// Reads directly to a Reader, a utility type thinly wrapping an
/// underlying Read implementation.
pub trait Readable<T> {
	/// Reads the data necessary to this Readable from the provided reader
	fn read(reader: &mut Reader) -> Result<T, Error>;
}

/// Deserializes a Readeable from any std::io::Read implementation.
pub fn deserialize<T: Readable<T>>(mut source: &mut Read) -> Result<T, Error> {
	let mut reader = BinReader { source: source };
	T::read(&mut reader)
}

/// Serializes a Writeable into any std::io::Write implementation.
pub fn serialize(mut sink: &mut Write, thing: &Writeable) -> Result<(), Error> {
	let mut writer = BinWriter { sink: sink };
	thing.write(&mut writer)
}

/// Utility function to serialize a writeable directly in memory using a
/// Vec<u8>.
pub fn ser_vec(thing: &Writeable) -> Result<Vec<u8>, Error> {
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
	fn read_fixed_bytes(&mut self, length: usize) -> Result<Vec<u8>, Error> {
		// not reading more than 100k in a single read
		if length > 100000 {
			return Err(Error::TooLargeReadErr(format!("fixed bytes length too large: {}", length)));
		}
		let mut buf = vec![0; length];
		self.source.read_exact(&mut buf).map(move |_| buf).map_err(Error::IOErr)
	}
	fn read_32_bytes(&mut self) -> Result<Vec<u8>, Error> {
		self.read_fixed_bytes(32)
	}
	fn read_33_bytes(&mut self) -> Result<Vec<u8>, Error> {
		self.read_fixed_bytes(33)
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

/// Utility wrapper for an underlying byte Writer. Defines higher level methods
/// to write numbers, byte vectors, hashes, etc.
struct BinWriter<'a> {
	sink: &'a mut Write,
}

impl<'a> Writer for BinWriter<'a> {
	fn write_u8(&mut self, n: u8) -> Result<(), Error> {
		try!(self.sink.write_u8(n));
		Ok(())
	}
	fn write_u16(&mut self, n: u16) -> Result<(), Error> {
		try!(self.sink.write_u16::<BigEndian>(n));
		Ok(())
	}
	fn write_u32(&mut self, n: u32) -> Result<(), Error> {
		try!(self.sink.write_u32::<BigEndian>(n));
		Ok(())
	}

	fn write_u64(&mut self, n: u64) -> Result<(), Error> {
		try!(self.sink.write_u64::<BigEndian>(n));
		Ok(())
	}

	fn write_i64(&mut self, n: i64) -> Result<(), Error> {
		try!(self.sink.write_i64::<BigEndian>(n));
		Ok(())
	}


	fn write_bytes(&mut self, data: &[u8]) -> Result<(), Error> {
		try!(self.write_u64(data.len() as u64));
		try!(self.sink.write_all(data));
		Ok(())
	}

	fn write_fixed_bytes(&mut self, b32: &AsFixedBytes) -> Result<(), Error> {
		let bs = b32.as_fixed_bytes();
		try!(self.sink.write_all(bs));
		Ok(())
	}
}

macro_rules! impl_slice_bytes {
  ($byteable: ty) => {
    impl AsFixedBytes for $byteable {
      fn as_fixed_bytes(&self) -> &[u8] {
        &self[..]
      }
    }
  }
}

impl_slice_bytes!(::secp::key::SecretKey);
impl_slice_bytes!(::secp::Signature);
impl_slice_bytes!(::secp::pedersen::Commitment);
impl_slice_bytes!(Vec<u8>);

impl AsFixedBytes for ::core::hash::Hash {
	fn as_fixed_bytes(&self) -> &[u8] {
		self.to_slice()
	}
}

impl AsFixedBytes for ::secp::pedersen::RangeProof {
	fn as_fixed_bytes(&self) -> &[u8] {
		&self.bytes()
	}
}
