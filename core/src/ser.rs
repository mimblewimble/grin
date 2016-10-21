//! Serialization and deserialization layer specialized for binary encoding.
//! Ensures consistency and safety. Basically a minimal subset or
//! rustc_serialize customized for our need.
//!
//! To use it simply implement `Writeable` or `Readable` and then use the
//! `serialize` or `deserialize` functions on them as appropriate.

use std::io;
use std::io::{Write, Read};
use byteorder::{ReadBytesExt, WriteBytesExt, BigEndian};

/// Possible errors deriving from serializing or deserializing.
#[derive(Debug)]
pub enum Error {
	/// Wraps an io error produced when reading or writing
	IOErr(io::Error),
	/// When asked to read too much data
	TooLargeReadErr(String),
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
	/// Writes a u32 as bytes
	fn write_u32(&mut self, n: u32) -> Option<Error>;
	/// Writes a u64 as bytes
	fn write_u64(&mut self, n: u64) -> Option<Error>;
	/// Writes a i64 as bytes
	fn write_i64(&mut self, n: i64) -> Option<Error>;
	/// Writes a variable length `Vec`, the length of the `Vec` is encoded as a
	/// prefix.
	fn write_vec(&mut self, vec: &mut Vec<u8>) -> Option<Error>;
	/// Writes a fixed number of bytes from something that can turn itself into
	/// a `&[u8]`. The reader is expected to know the actual length on read.
	fn write_fixed_bytes(&mut self, b32: &AsFixedBytes) -> Option<Error>;
}

/// Implementations defined how different numbers and binary structures are
/// read from an underlying stream or container (depending on implementation).
pub trait Reader {
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
}

/// Trait that every type that can be serialized as binary must implement.
/// Writes directly to a Writer, a utility type thinly wrapping an
/// underlying Write implementation.
pub trait Writeable {
	/// Write the data held by this Writeable to the provided writer
	fn write(&self, writer: &mut Writer) -> Option<Error>;
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
pub fn serialize(mut sink: &mut Write, thing: &Writeable) -> Option<Error> {
	let mut writer = BinWriter { sink: sink };
	thing.write(&mut writer)
}

/// Utility function to serialize a writeable directly in memory using a
/// Vec<u8>.
pub fn ser_vec(thing: &Writeable) -> Result<Vec<u8>, Error> {
	let mut vec = Vec::new();
	if let Some(err) = serialize(&mut vec, thing) {
		return Err(err);
	}
	Ok(vec)
}

struct BinReader<'a> {
	source: &'a mut Read,
}

/// Utility wrapper for an underlying byte Reader. Defines higher level methods
/// to read numbers, byte vectors, hashes, etc.
impl<'a> Reader for BinReader<'a> {
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
}

/// Utility wrapper for an underlying byte Writer. Defines higher level methods
/// to write numbers, byte vectors, hashes, etc.
struct BinWriter<'a> {
	sink: &'a mut Write,
}

impl<'a> Writer for BinWriter<'a> {
	fn write_u32(&mut self, n: u32) -> Option<Error> {
		self.sink.write_u32::<BigEndian>(n).err().map(Error::IOErr)
	}

	fn write_u64(&mut self, n: u64) -> Option<Error> {
		self.sink.write_u64::<BigEndian>(n).err().map(Error::IOErr)
	}

	fn write_i64(&mut self, n: i64) -> Option<Error> {
		self.sink.write_i64::<BigEndian>(n).err().map(Error::IOErr)
	}


	fn write_vec(&mut self, vec: &mut Vec<u8>) -> Option<Error> {
		try_m!(self.write_u64(vec.len() as u64));
		self.sink.write_all(vec).err().map(Error::IOErr)
	}

	fn write_fixed_bytes(&mut self, b32: &AsFixedBytes) -> Option<Error> {
		let bs = b32.as_fixed_bytes();
		self.sink.write_all(bs).err().map(Error::IOErr)
	}
}
