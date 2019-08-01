// Copyright 2018 The Grin Developers
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

use crate::core::hash::{DefaultHashable, Hash, Hashed};
use crate::global::PROTOCOL_VERSION;
use crate::keychain::{BlindingFactor, Identifier, IDENTIFIER_SIZE};
use crate::util::secp::constants::{
	AGG_SIGNATURE_SIZE, COMPRESSED_PUBLIC_KEY_SIZE, MAX_PROOF_SIZE, PEDERSEN_COMMITMENT_SIZE,
	SECRET_KEY_SIZE,
};
use crate::util::secp::key::PublicKey;
use crate::util::secp::pedersen::{Commitment, RangeProof};
use crate::util::secp::Signature;
use crate::util::secp::{ContextFlag, Secp256k1};
use byteorder::{BigEndian, ByteOrder, ReadBytesExt};
use std::fmt::{self, Debug};
use std::io::{self, Read, Write};
use std::marker;
use std::{cmp, error};

/// Possible errors deriving from serializing or deserializing.
#[derive(Clone, Eq, PartialEq, Debug, Serialize, Deserialize)]
pub enum Error {
	/// Wraps an io error produced when reading or writing
	IOErr(
		String,
		#[serde(
			serialize_with = "serialize_error_kind",
			deserialize_with = "deserialize_error_kind"
		)]
		io::ErrorKind,
	),
	/// Expected a given value that wasn't found
	UnexpectedData {
		/// What we wanted
		expected: Vec<u8>,
		/// What we got
		received: Vec<u8>,
	},
	/// Data wasn't in a consumable format
	CorruptedData,
	/// Incorrect number of elements (when deserializing a vec via read_multi say).
	CountError,
	/// When asked to read too much data
	TooLargeReadErr,
	/// Error from from_hex deserialization
	HexError(String),
	/// Inputs/outputs/kernels must be sorted lexicographically.
	SortError,
	/// Inputs/outputs/kernels must be unique.
	DuplicateError,
	/// Block header version (hard-fork schedule).
	InvalidBlockVersion,
}

impl From<io::Error> for Error {
	fn from(e: io::Error) -> Error {
		Error::IOErr(format!("{}", e), e.kind())
	}
}

impl fmt::Display for Error {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match *self {
			Error::IOErr(ref e, ref _k) => write!(f, "{}", e),
			Error::UnexpectedData {
				expected: ref e,
				received: ref r,
			} => write!(f, "expected {:?}, got {:?}", e, r),
			Error::CorruptedData => f.write_str("corrupted data"),
			Error::CountError => f.write_str("count error"),
			Error::SortError => f.write_str("sort order"),
			Error::DuplicateError => f.write_str("duplicate"),
			Error::TooLargeReadErr => f.write_str("too large read"),
			Error::HexError(ref e) => write!(f, "hex error {:?}", e),
			Error::InvalidBlockVersion => f.write_str("invalid block version"),
		}
	}
}

impl error::Error for Error {
	fn cause(&self) -> Option<&dyn error::Error> {
		match *self {
			Error::IOErr(ref _e, ref _k) => Some(self),
			_ => None,
		}
	}

	fn description(&self) -> &str {
		match *self {
			Error::IOErr(ref e, _) => e,
			Error::UnexpectedData { .. } => "unexpected data",
			Error::CorruptedData => "corrupted data",
			Error::CountError => "count error",
			Error::SortError => "sort order",
			Error::DuplicateError => "duplicate error",
			Error::TooLargeReadErr => "too large read",
			Error::HexError(_) => "hex error",
			Error::InvalidBlockVersion => "invalid block version",
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
}

/// Implementations defined how different numbers and binary structures are
/// written to an underlying stream or container (depending on implementation).
pub trait Writer {
	/// The mode this serializer is writing in
	fn serialization_mode(&self) -> SerializationMode;

	/// Protocol version for version specific serialization rules.
	fn protocol_version(&self) -> ProtocolVersion;

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

	/// Writes a u32 as bytes
	fn write_i32(&mut self, n: i32) -> Result<(), Error> {
		let mut bytes = [0; 4];
		BigEndian::write_i32(&mut bytes, n);
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
		self.write_u64(bytes.as_ref().len() as u64)?;
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
	fn read_i32(&mut self) -> Result<i32, Error>;
	/// Read a i64 from the underlying Read
	fn read_i64(&mut self) -> Result<i64, Error>;
	/// Read a u64 len prefix followed by that number of exact bytes.
	fn read_bytes_len_prefix(&mut self) -> Result<Vec<u8>, Error>;
	/// Read a fixed number of bytes from the underlying reader.
	fn read_fixed_bytes(&mut self, length: usize) -> Result<Vec<u8>, Error>;
	/// Consumes a byte from the reader, producing an error if it doesn't have
	/// the expected value
	fn expect_u8(&mut self, val: u8) -> Result<u8, Error>;
	/// Access to underlying protocol version to support
	/// version specific deserialization logic.
	fn protocol_version(&self) -> ProtocolVersion;
}

/// Trait that every type that can be serialized as binary must implement.
/// Writes directly to a Writer, a utility type thinly wrapping an
/// underlying Write implementation.
pub trait Writeable {
	/// Write the data held by this Writeable to the provided writer
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), Error>;
}

/// Reader that exposes an Iterator interface.
pub struct IteratingReader<'a, T> {
	count: u64,
	curr: u64,
	reader: &'a mut dyn Reader,
	_marker: marker::PhantomData<T>,
}

impl<'a, T> IteratingReader<'a, T> {
	/// Constructor to create a new iterating reader for the provided underlying reader.
	/// Takes a count so we know how many to iterate over.
	pub fn new(reader: &'a mut dyn Reader, count: u64) -> IteratingReader<'a, T> {
		let curr = 0;
		IteratingReader {
			count,
			curr,
			reader,
			_marker: marker::PhantomData,
		}
	}
}

impl<'a, T> Iterator for IteratingReader<'a, T>
where
	T: Readable,
{
	type Item = T;

	fn next(&mut self) -> Option<T> {
		if self.curr >= self.count {
			return None;
		}
		self.curr += 1;
		T::read(self.reader).ok()
	}
}

/// Reads multiple serialized items into a Vec.
pub fn read_multi<T>(reader: &mut dyn Reader, count: u64) -> Result<Vec<T>, Error>
where
	T: Readable,
{
	// Very rudimentary check to ensure we do not overflow anything
	// attempting to read huge amounts of data.
	// Probably better than checking if count * size overflows a u64 though.
	if count > 1_000_000 {
		return Err(Error::TooLargeReadErr);
	}

	let res: Vec<T> = IteratingReader::new(reader, count).collect();
	if res.len() as u64 != count {
		return Err(Error::CountError);
	}
	Ok(res)
}

/// Protocol version for serialization/deserialization.
/// Note: This is used in various places including but limited to
/// the p2p layer and our local db storage layer.
/// We may speak multiple versions to various peers and a potentially *different*
/// version for our local db.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialOrd, PartialEq, Serialize)]
pub struct ProtocolVersion(pub u32);

impl ProtocolVersion {
	/// Our default "local" protocol version.
	pub fn local() -> ProtocolVersion {
		ProtocolVersion(PROTOCOL_VERSION)
	}

	/// We need to specify a protocol version for our local database.
	/// Regardless of specific version used when sending/receiving data between peers
	/// we need to take care with serialization/deserialization of data locally in the db.
	pub fn local_db() -> ProtocolVersion {
		ProtocolVersion(1)
	}
}

impl fmt::Display for ProtocolVersion {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "{}", self.0)
	}
}

impl From<ProtocolVersion> for u32 {
	fn from(v: ProtocolVersion) -> u32 {
		v.0
	}
}

impl Writeable for ProtocolVersion {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), Error> {
		writer.write_u32(self.0)
	}
}

impl Readable for ProtocolVersion {
	fn read(reader: &mut dyn Reader) -> Result<ProtocolVersion, Error> {
		let version = reader.read_u32()?;
		Ok(ProtocolVersion(version))
	}
}

/// Trait that every type that can be deserialized from binary must implement.
/// Reads directly to a Reader, a utility type thinly wrapping an
/// underlying Read implementation.
pub trait Readable
where
	Self: Sized,
{
	/// Reads the data necessary to this Readable from the provided reader
	fn read(reader: &mut dyn Reader) -> Result<Self, Error>;
}

/// Deserializes a Readable from any std::io::Read implementation.
pub fn deserialize<T: Readable>(
	source: &mut dyn Read,
	version: ProtocolVersion,
) -> Result<T, Error> {
	let mut reader = BinReader::new(source, version);
	T::read(&mut reader)
}

/// Deserialize a Readable based on our default "local" protocol version.
pub fn deserialize_default<T: Readable>(source: &mut dyn Read) -> Result<T, Error> {
	deserialize(source, ProtocolVersion::local())
}

/// Serializes a Writeable into any std::io::Write implementation.
pub fn serialize<W: Writeable>(
	sink: &mut dyn Write,
	version: ProtocolVersion,
	thing: &W,
) -> Result<(), Error> {
	let mut writer = BinWriter::new(sink, version);
	thing.write(&mut writer)
}

/// Serialize a Writeable according to our default "local" protocol version.
pub fn serialize_default<W: Writeable>(sink: &mut dyn Write, thing: &W) -> Result<(), Error> {
	serialize(sink, ProtocolVersion::local(), thing)
}

/// Utility function to serialize a writeable directly in memory using a
/// Vec<u8>.
pub fn ser_vec<W: Writeable>(thing: &W, version: ProtocolVersion) -> Result<Vec<u8>, Error> {
	let mut vec = vec![];
	serialize(&mut vec, version, thing)?;
	Ok(vec)
}

/// Utility to read from a binary source
pub struct BinReader<'a> {
	source: &'a mut dyn Read,
	version: ProtocolVersion,
}

impl<'a> BinReader<'a> {
	/// Constructor for a new BinReader for the provided source and protocol version.
	pub fn new(source: &'a mut dyn Read, version: ProtocolVersion) -> BinReader<'a> {
		BinReader { source, version }
	}
}

fn map_io_err(err: io::Error) -> Error {
	Error::IOErr(format!("{}", err), err.kind())
}

/// Utility wrapper for an underlying byte Reader. Defines higher level methods
/// to read numbers, byte vectors, hashes, etc.
impl<'a> Reader for BinReader<'a> {
	fn read_u8(&mut self) -> Result<u8, Error> {
		self.source.read_u8().map_err(map_io_err)
	}
	fn read_u16(&mut self) -> Result<u16, Error> {
		self.source.read_u16::<BigEndian>().map_err(map_io_err)
	}
	fn read_u32(&mut self) -> Result<u32, Error> {
		self.source.read_u32::<BigEndian>().map_err(map_io_err)
	}
	fn read_i32(&mut self) -> Result<i32, Error> {
		self.source.read_i32::<BigEndian>().map_err(map_io_err)
	}
	fn read_u64(&mut self) -> Result<u64, Error> {
		self.source.read_u64::<BigEndian>().map_err(map_io_err)
	}
	fn read_i64(&mut self) -> Result<i64, Error> {
		self.source.read_i64::<BigEndian>().map_err(map_io_err)
	}
	/// Read a variable size vector from the underlying Read. Expects a usize
	fn read_bytes_len_prefix(&mut self) -> Result<Vec<u8>, Error> {
		let len = self.read_u64()?;
		self.read_fixed_bytes(len as usize)
	}

	/// Read a fixed number of bytes.
	fn read_fixed_bytes(&mut self, len: usize) -> Result<Vec<u8>, Error> {
		// not reading more than 100k bytes in a single read
		if len > 100_000 {
			return Err(Error::TooLargeReadErr);
		}
		let mut buf = vec![0; len];
		self.source
			.read_exact(&mut buf)
			.map(move |_| buf)
			.map_err(map_io_err)
	}

	fn expect_u8(&mut self, val: u8) -> Result<u8, Error> {
		let b = self.read_u8()?;
		if b == val {
			Ok(b)
		} else {
			Err(Error::UnexpectedData {
				expected: vec![val],
				received: vec![b],
			})
		}
	}

	fn protocol_version(&self) -> ProtocolVersion {
		self.version
	}
}

/// A reader that reads straight off a stream.
/// Tracks total bytes read so we can verify we read the right number afterwards.
pub struct StreamingReader<'a> {
	total_bytes_read: u64,
	version: ProtocolVersion,
	stream: &'a mut dyn Read,
}

impl<'a> StreamingReader<'a> {
	/// Create a new streaming reader with the provided underlying stream.
	/// Also takes a duration to be used for each individual read_exact call.
	pub fn new(stream: &'a mut dyn Read, version: ProtocolVersion) -> StreamingReader<'a> {
		StreamingReader {
			total_bytes_read: 0,
			version,
			stream,
		}
	}

	/// Returns the total bytes read via this streaming reader.
	pub fn total_bytes_read(&self) -> u64 {
		self.total_bytes_read
	}
}

/// Note: We use read_fixed_bytes() here to ensure our "async" I/O behaves as expected.
impl<'a> Reader for StreamingReader<'a> {
	fn read_u8(&mut self) -> Result<u8, Error> {
		let buf = self.read_fixed_bytes(1)?;
		Ok(buf[0])
	}
	fn read_u16(&mut self) -> Result<u16, Error> {
		let buf = self.read_fixed_bytes(2)?;
		Ok(BigEndian::read_u16(&buf[..]))
	}
	fn read_u32(&mut self) -> Result<u32, Error> {
		let buf = self.read_fixed_bytes(4)?;
		Ok(BigEndian::read_u32(&buf[..]))
	}
	fn read_i32(&mut self) -> Result<i32, Error> {
		let buf = self.read_fixed_bytes(4)?;
		Ok(BigEndian::read_i32(&buf[..]))
	}
	fn read_u64(&mut self) -> Result<u64, Error> {
		let buf = self.read_fixed_bytes(8)?;
		Ok(BigEndian::read_u64(&buf[..]))
	}
	fn read_i64(&mut self) -> Result<i64, Error> {
		let buf = self.read_fixed_bytes(8)?;
		Ok(BigEndian::read_i64(&buf[..]))
	}

	/// Read a variable size vector from the underlying stream. Expects a usize
	fn read_bytes_len_prefix(&mut self) -> Result<Vec<u8>, Error> {
		let len = self.read_u64()?;
		self.total_bytes_read += 8;
		self.read_fixed_bytes(len as usize)
	}

	/// Read a fixed number of bytes.
	fn read_fixed_bytes(&mut self, len: usize) -> Result<Vec<u8>, Error> {
		let mut buf = vec![0u8; len];
		self.stream.read_exact(&mut buf)?;
		self.total_bytes_read += len as u64;
		Ok(buf)
	}

	fn expect_u8(&mut self, val: u8) -> Result<u8, Error> {
		let b = self.read_u8()?;
		if b == val {
			Ok(b)
		} else {
			Err(Error::UnexpectedData {
				expected: vec![val],
				received: vec![b],
			})
		}
	}

	fn protocol_version(&self) -> ProtocolVersion {
		self.version
	}
}

impl Readable for Commitment {
	fn read(reader: &mut dyn Reader) -> Result<Commitment, Error> {
		let a = reader.read_fixed_bytes(PEDERSEN_COMMITMENT_SIZE)?;
		let mut c = [0; PEDERSEN_COMMITMENT_SIZE];
		c[..PEDERSEN_COMMITMENT_SIZE].clone_from_slice(&a[..PEDERSEN_COMMITMENT_SIZE]);
		Ok(Commitment(c))
	}
}

impl Writeable for Commitment {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), Error> {
		writer.write_fixed_bytes(self)
	}
}

impl Writeable for BlindingFactor {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), Error> {
		writer.write_fixed_bytes(self)
	}
}

impl Readable for BlindingFactor {
	fn read(reader: &mut dyn Reader) -> Result<BlindingFactor, Error> {
		let bytes = reader.read_fixed_bytes(BlindingFactor::LEN)?;
		Ok(BlindingFactor::from_slice(&bytes))
	}
}

impl FixedLength for BlindingFactor {
	const LEN: usize = SECRET_KEY_SIZE;
}

impl Writeable for Identifier {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), Error> {
		writer.write_fixed_bytes(self)
	}
}

impl Readable for Identifier {
	fn read(reader: &mut dyn Reader) -> Result<Identifier, Error> {
		let bytes = reader.read_fixed_bytes(IDENTIFIER_SIZE)?;
		Ok(Identifier::from_bytes(&bytes))
	}
}

impl Writeable for RangeProof {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), Error> {
		writer.write_bytes(self)
	}
}

impl Readable for RangeProof {
	fn read(reader: &mut dyn Reader) -> Result<RangeProof, Error> {
		let len = reader.read_u64()?;
		let max_len = cmp::min(len as usize, MAX_PROOF_SIZE);
		let p = reader.read_fixed_bytes(max_len)?;
		let mut proof = [0; MAX_PROOF_SIZE];
		proof[..p.len()].clone_from_slice(&p[..]);
		Ok(RangeProof {
			plen: proof.len(),
			proof,
		})
	}
}

impl FixedLength for RangeProof {
	const LEN: usize = 8 // length prefix
		+ MAX_PROOF_SIZE;
}

impl PMMRable for RangeProof {
	type E = Self;

	fn as_elmt(&self) -> Self::E {
		self.clone()
	}
}

impl Readable for Signature {
	fn read(reader: &mut dyn Reader) -> Result<Signature, Error> {
		let a = reader.read_fixed_bytes(Signature::LEN)?;
		let mut c = [0; Signature::LEN];
		c[..Signature::LEN].clone_from_slice(&a[..Signature::LEN]);
		Ok(Signature::from_raw_data(&c).unwrap())
	}
}

impl Writeable for Signature {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), Error> {
		writer.write_fixed_bytes(self)
	}
}

impl FixedLength for Signature {
	const LEN: usize = AGG_SIGNATURE_SIZE;
}

impl FixedLength for PublicKey {
	const LEN: usize = COMPRESSED_PUBLIC_KEY_SIZE;
}

impl Writeable for PublicKey {
	// Write the public key in compressed form
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), Error> {
		let secp = Secp256k1::with_caps(ContextFlag::None);
		writer.write_fixed_bytes(&self.serialize_vec(&secp, true).as_ref())?;
		Ok(())
	}
}

impl Readable for PublicKey {
	// Read the public key in compressed form
	fn read(reader: &mut dyn Reader) -> Result<Self, Error> {
		let buf = reader.read_fixed_bytes(PublicKey::LEN)?;
		let secp = Secp256k1::with_caps(ContextFlag::None);
		let pk = PublicKey::from_slice(&secp, &buf).map_err(|_| Error::CorruptedData)?;
		Ok(pk)
	}
}

/// Collections of items must be sorted lexicographically and all unique.
pub trait VerifySortedAndUnique<T> {
	/// Verify a collection of items is sorted and all unique.
	fn verify_sorted_and_unique(&self) -> Result<(), Error>;
}

impl<T: Hashed> VerifySortedAndUnique<T> for Vec<T> {
	fn verify_sorted_and_unique(&self) -> Result<(), Error> {
		let hashes = self.iter().map(|item| item.hash()).collect::<Vec<_>>();
		let pairs = hashes.windows(2);
		for pair in pairs {
			if pair[0] > pair[1] {
				return Err(Error::SortError);
			} else if pair[0] == pair[1] {
				return Err(Error::DuplicateError);
			}
		}
		Ok(())
	}
}

/// Utility wrapper for an underlying byte Writer. Defines higher level methods
/// to write numbers, byte vectors, hashes, etc.
pub struct BinWriter<'a> {
	sink: &'a mut dyn Write,
	version: ProtocolVersion,
}

impl<'a> BinWriter<'a> {
	/// Wraps a standard Write in a new BinWriter
	pub fn new(sink: &'a mut dyn Write, version: ProtocolVersion) -> BinWriter<'a> {
		BinWriter { sink, version }
	}

	/// Constructor for BinWriter with default "local" protocol version.
	pub fn default(sink: &'a mut dyn Write) -> BinWriter<'a> {
		BinWriter::new(sink, ProtocolVersion::local())
	}
}

impl<'a> Writer for BinWriter<'a> {
	fn serialization_mode(&self) -> SerializationMode {
		SerializationMode::Full
	}

	fn write_fixed_bytes<T: AsFixedBytes>(&mut self, fixed: &T) -> Result<(), Error> {
		let bs = fixed.as_ref();
		self.sink.write_all(bs)?;
		Ok(())
	}

	fn protocol_version(&self) -> ProtocolVersion {
		self.version
	}
}

macro_rules! impl_int {
	($int:ty, $w_fn:ident, $r_fn:ident) => {
		impl Writeable for $int {
			fn write<W: Writer>(&self, writer: &mut W) -> Result<(), Error> {
				writer.$w_fn(*self)
			}
		}

		impl Readable for $int {
			fn read(reader: &mut dyn Reader) -> Result<$int, Error> {
				reader.$r_fn()
			}
		}
	};
}

impl_int!(u8, write_u8, read_u8);
impl_int!(u16, write_u16, read_u16);
impl_int!(u32, write_u32, read_u32);
impl_int!(i32, write_i32, read_i32);
impl_int!(u64, write_u64, read_u64);
impl_int!(i64, write_i64, read_i64);

impl<T> Readable for Vec<T>
where
	T: Readable,
{
	fn read(reader: &mut dyn Reader) -> Result<Vec<T>, Error> {
		let mut buf = Vec::new();
		loop {
			let elem = T::read(reader);
			match elem {
				Ok(e) => buf.push(e),
				Err(Error::IOErr(ref _d, ref kind)) if *kind == io::ErrorKind::UnexpectedEof => {
					break;
				}
				Err(e) => return Err(e),
			}
		}
		Ok(buf)
	}
}

impl<T> Writeable for Vec<T>
where
	T: Writeable,
{
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), Error> {
		for elmt in self {
			elmt.write(writer)?;
		}
		Ok(())
	}
}

impl<'a, A: Writeable> Writeable for &'a A {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), Error> {
		Writeable::write(*self, writer)
	}
}

impl<A: Writeable, B: Writeable> Writeable for (A, B) {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), Error> {
		Writeable::write(&self.0, writer)?;
		Writeable::write(&self.1, writer)
	}
}

impl<A: Readable, B: Readable> Readable for (A, B) {
	fn read(reader: &mut dyn Reader) -> Result<(A, B), Error> {
		Ok((Readable::read(reader)?, Readable::read(reader)?))
	}
}

impl<A: Writeable, B: Writeable, C: Writeable> Writeable for (A, B, C) {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), Error> {
		Writeable::write(&self.0, writer)?;
		Writeable::write(&self.1, writer)?;
		Writeable::write(&self.2, writer)
	}
}

impl<A: Writeable, B: Writeable, C: Writeable, D: Writeable> Writeable for (A, B, C, D) {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), Error> {
		Writeable::write(&self.0, writer)?;
		Writeable::write(&self.1, writer)?;
		Writeable::write(&self.2, writer)?;
		Writeable::write(&self.3, writer)
	}
}

impl<A: Readable, B: Readable, C: Readable> Readable for (A, B, C) {
	fn read(reader: &mut dyn Reader) -> Result<(A, B, C), Error> {
		Ok((
			Readable::read(reader)?,
			Readable::read(reader)?,
			Readable::read(reader)?,
		))
	}
}

impl<A: Readable, B: Readable, C: Readable, D: Readable> Readable for (A, B, C, D) {
	fn read(reader: &mut dyn Reader) -> Result<(A, B, C, D), Error> {
		Ok((
			Readable::read(reader)?,
			Readable::read(reader)?,
			Readable::read(reader)?,
			Readable::read(reader)?,
		))
	}
}

impl Writeable for [u8; 4] {
	fn write<W: Writer>(&self, writer: &mut W) -> Result<(), Error> {
		writer.write_bytes(self)
	}
}

/// Trait for types that serialize to a known fixed length.
pub trait FixedLength {
	/// The length in bytes
	const LEN: usize;
}

/// Trait for types that can be added to a PMMR.
pub trait PMMRable: Writeable + Clone + Debug + DefaultHashable {
	/// The type of element actually stored in the MMR data file.
	/// This allows us to store Hash elements in the header MMR for variable size BlockHeaders.
	type E: FixedLength + Readable + Writeable + Debug;

	/// Convert the pmmrable into the element to be stored in the MMR data file.
	fn as_elmt(&self) -> Self::E;
}

/// Generic trait to ensure PMMR elements can be hashed with an index
pub trait PMMRIndexHashable {
	/// Hash with a given index
	fn hash_with_index(&self, index: u64) -> Hash;
}

impl<T: DefaultHashable> PMMRIndexHashable for T {
	fn hash_with_index(&self, index: u64) -> Hash {
		(index, self).hash()
	}
}

/// Useful marker trait on types that can be sized byte slices
pub trait AsFixedBytes: Sized + AsRef<[u8]> {
	/// The length in bytes
	fn len(&self) -> usize;
}

impl<'a> AsFixedBytes for &'a [u8] {
	fn len(&self) -> usize {
		1
	}
}
impl AsFixedBytes for Vec<u8> {
	fn len(&self) -> usize {
		self.len()
	}
}
impl AsFixedBytes for [u8; 1] {
	fn len(&self) -> usize {
		1
	}
}
impl AsFixedBytes for [u8; 2] {
	fn len(&self) -> usize {
		2
	}
}
impl AsFixedBytes for [u8; 4] {
	fn len(&self) -> usize {
		4
	}
}
impl AsFixedBytes for [u8; 6] {
	fn len(&self) -> usize {
		6
	}
}
impl AsFixedBytes for [u8; 8] {
	fn len(&self) -> usize {
		8
	}
}
impl AsFixedBytes for [u8; 20] {
	fn len(&self) -> usize {
		20
	}
}
impl AsFixedBytes for [u8; 32] {
	fn len(&self) -> usize {
		32
	}
}
impl AsFixedBytes for String {
	fn len(&self) -> usize {
		self.len()
	}
}
impl AsFixedBytes for crate::core::hash::Hash {
	fn len(&self) -> usize {
		32
	}
}
impl AsFixedBytes for crate::util::secp::pedersen::RangeProof {
	fn len(&self) -> usize {
		self.plen
	}
}
impl AsFixedBytes for crate::util::secp::Signature {
	fn len(&self) -> usize {
		64
	}
}
impl AsFixedBytes for crate::util::secp::pedersen::Commitment {
	fn len(&self) -> usize {
		PEDERSEN_COMMITMENT_SIZE
	}
}
impl AsFixedBytes for BlindingFactor {
	fn len(&self) -> usize {
		SECRET_KEY_SIZE
	}
}
impl AsFixedBytes for crate::keychain::Identifier {
	fn len(&self) -> usize {
		IDENTIFIER_SIZE
	}
}

// serializer for io::Errorkind, originally auto-generated by serde-derive
// slightly modified to handle the #[non_exhaustive] tag on io::ErrorKind
fn serialize_error_kind<S>(
	kind: &io::ErrorKind,
	serializer: S,
) -> serde::export::Result<S::Ok, S::Error>
where
	S: serde::Serializer,
{
	match *kind {
		io::ErrorKind::NotFound => {
			serde::Serializer::serialize_unit_variant(serializer, "ErrorKind", 0u32, "NotFound")
		}
		io::ErrorKind::PermissionDenied => serde::Serializer::serialize_unit_variant(
			serializer,
			"ErrorKind",
			1u32,
			"PermissionDenied",
		),
		io::ErrorKind::ConnectionRefused => serde::Serializer::serialize_unit_variant(
			serializer,
			"ErrorKind",
			2u32,
			"ConnectionRefused",
		),
		io::ErrorKind::ConnectionReset => serde::Serializer::serialize_unit_variant(
			serializer,
			"ErrorKind",
			3u32,
			"ConnectionReset",
		),
		io::ErrorKind::ConnectionAborted => serde::Serializer::serialize_unit_variant(
			serializer,
			"ErrorKind",
			4u32,
			"ConnectionAborted",
		),
		io::ErrorKind::NotConnected => {
			serde::Serializer::serialize_unit_variant(serializer, "ErrorKind", 5u32, "NotConnected")
		}
		io::ErrorKind::AddrInUse => {
			serde::Serializer::serialize_unit_variant(serializer, "ErrorKind", 6u32, "AddrInUse")
		}
		io::ErrorKind::AddrNotAvailable => serde::Serializer::serialize_unit_variant(
			serializer,
			"ErrorKind",
			7u32,
			"AddrNotAvailable",
		),
		io::ErrorKind::BrokenPipe => {
			serde::Serializer::serialize_unit_variant(serializer, "ErrorKind", 8u32, "BrokenPipe")
		}
		io::ErrorKind::AlreadyExists => serde::Serializer::serialize_unit_variant(
			serializer,
			"ErrorKind",
			9u32,
			"AlreadyExists",
		),
		io::ErrorKind::WouldBlock => {
			serde::Serializer::serialize_unit_variant(serializer, "ErrorKind", 10u32, "WouldBlock")
		}
		io::ErrorKind::InvalidInput => serde::Serializer::serialize_unit_variant(
			serializer,
			"ErrorKind",
			11u32,
			"InvalidInput",
		),
		io::ErrorKind::InvalidData => {
			serde::Serializer::serialize_unit_variant(serializer, "ErrorKind", 12u32, "InvalidData")
		}
		io::ErrorKind::TimedOut => {
			serde::Serializer::serialize_unit_variant(serializer, "ErrorKind", 13u32, "TimedOut")
		}
		io::ErrorKind::WriteZero => {
			serde::Serializer::serialize_unit_variant(serializer, "ErrorKind", 14u32, "WriteZero")
		}
		io::ErrorKind::Interrupted => {
			serde::Serializer::serialize_unit_variant(serializer, "ErrorKind", 15u32, "Interrupted")
		}
		io::ErrorKind::Other => {
			serde::Serializer::serialize_unit_variant(serializer, "ErrorKind", 16u32, "Other")
		}
		io::ErrorKind::UnexpectedEof => serde::Serializer::serialize_unit_variant(
			serializer,
			"ErrorKind",
			17u32,
			"UnexpectedEof",
		),
		// #[non_exhaustive] is used on the definition of ErrorKind for future compatability
		// That means match statements always need to match on _.
		// The downside here is that rustc won't be able to warn us if io::ErrorKind another
		// field is added to io::ErrorKind
		_ => serde::Serializer::serialize_unit_variant(serializer, "ErrorKind", 16u32, "Other"),
	}
}

// deserializer for io::Errorkind, originally auto-generated by serde-derive
fn deserialize_error_kind<'de, D>(deserializer: D) -> serde::export::Result<io::ErrorKind, D::Error>
where
	D: serde::Deserializer<'de>,
{
	#[allow(non_camel_case_types)]
	enum Field {
		field0,
		field1,
		field2,
		field3,
		field4,
		field5,
		field6,
		field7,
		field8,
		field9,
		field10,
		field11,
		field12,
		field13,
		field14,
		field15,
		field16,
		field17,
	}
	struct FieldVisitor;
	impl<'de> serde::de::Visitor<'de> for FieldVisitor {
		type Value = Field;
		fn expecting(
			&self,
			formatter: &mut serde::export::Formatter,
		) -> serde::export::fmt::Result {
			serde::export::Formatter::write_str(formatter, "variant identifier")
		}
		fn visit_u64<E>(self, value: u64) -> serde::export::Result<Self::Value, E>
		where
			E: serde::de::Error,
		{
			match value {
				0u64 => serde::export::Ok(Field::field0),
				1u64 => serde::export::Ok(Field::field1),
				2u64 => serde::export::Ok(Field::field2),
				3u64 => serde::export::Ok(Field::field3),
				4u64 => serde::export::Ok(Field::field4),
				5u64 => serde::export::Ok(Field::field5),
				6u64 => serde::export::Ok(Field::field6),
				7u64 => serde::export::Ok(Field::field7),
				8u64 => serde::export::Ok(Field::field8),
				9u64 => serde::export::Ok(Field::field9),
				10u64 => serde::export::Ok(Field::field10),
				11u64 => serde::export::Ok(Field::field11),
				12u64 => serde::export::Ok(Field::field12),
				13u64 => serde::export::Ok(Field::field13),
				14u64 => serde::export::Ok(Field::field14),
				15u64 => serde::export::Ok(Field::field15),
				16u64 => serde::export::Ok(Field::field16),
				17u64 => serde::export::Ok(Field::field17),
				_ => serde::export::Err(serde::de::Error::invalid_value(
					serde::de::Unexpected::Unsigned(value),
					&"variant index 0 <= i < 18",
				)),
			}
		}
		fn visit_str<E>(self, value: &str) -> serde::export::Result<Self::Value, E>
		where
			E: serde::de::Error,
		{
			match value {
				"NotFound" => serde::export::Ok(Field::field0),
				"PermissionDenied" => serde::export::Ok(Field::field1),
				"ConnectionRefused" => serde::export::Ok(Field::field2),
				"ConnectionReset" => serde::export::Ok(Field::field3),
				"ConnectionAborted" => serde::export::Ok(Field::field4),
				"NotConnected" => serde::export::Ok(Field::field5),
				"AddrInUse" => serde::export::Ok(Field::field6),
				"AddrNotAvailable" => serde::export::Ok(Field::field7),
				"BrokenPipe" => serde::export::Ok(Field::field8),
				"AlreadyExists" => serde::export::Ok(Field::field9),
				"WouldBlock" => serde::export::Ok(Field::field10),
				"InvalidInput" => serde::export::Ok(Field::field11),
				"InvalidData" => serde::export::Ok(Field::field12),
				"TimedOut" => serde::export::Ok(Field::field13),
				"WriteZero" => serde::export::Ok(Field::field14),
				"Interrupted" => serde::export::Ok(Field::field15),
				"Other" => serde::export::Ok(Field::field16),
				"UnexpectedEof" => serde::export::Ok(Field::field17),
				_ => serde::export::Err(serde::de::Error::unknown_variant(value, VARIANTS)),
			}
		}
		fn visit_bytes<E>(self, value: &[u8]) -> serde::export::Result<Self::Value, E>
		where
			E: serde::de::Error,
		{
			match value {
				b"NotFound" => serde::export::Ok(Field::field0),
				b"PermissionDenied" => serde::export::Ok(Field::field1),
				b"ConnectionRefused" => serde::export::Ok(Field::field2),
				b"ConnectionReset" => serde::export::Ok(Field::field3),
				b"ConnectionAborted" => serde::export::Ok(Field::field4),
				b"NotConnected" => serde::export::Ok(Field::field5),
				b"AddrInUse" => serde::export::Ok(Field::field6),
				b"AddrNotAvailable" => serde::export::Ok(Field::field7),
				b"BrokenPipe" => serde::export::Ok(Field::field8),
				b"AlreadyExists" => serde::export::Ok(Field::field9),
				b"WouldBlock" => serde::export::Ok(Field::field10),
				b"InvalidInput" => serde::export::Ok(Field::field11),
				b"InvalidData" => serde::export::Ok(Field::field12),
				b"TimedOut" => serde::export::Ok(Field::field13),
				b"WriteZero" => serde::export::Ok(Field::field14),
				b"Interrupted" => serde::export::Ok(Field::field15),
				b"Other" => serde::export::Ok(Field::field16),
				b"UnexpectedEof" => serde::export::Ok(Field::field17),
				_ => {
					let value = &serde::export::from_utf8_lossy(value);
					serde::export::Err(serde::de::Error::unknown_variant(value, VARIANTS))
				}
			}
		}
	}
	impl<'de> serde::Deserialize<'de> for Field {
		#[inline]
		fn deserialize<D>(deserializer: D) -> serde::export::Result<Self, D::Error>
		where
			D: serde::Deserializer<'de>,
		{
			serde::Deserializer::deserialize_identifier(deserializer, FieldVisitor)
		}
	}
	struct Visitor<'de> {
		marker: serde::export::PhantomData<io::ErrorKind>,
		lifetime: serde::export::PhantomData<&'de ()>,
	}
	impl<'de> serde::de::Visitor<'de> for Visitor<'de> {
		type Value = io::ErrorKind;
		fn expecting(
			&self,
			formatter: &mut serde::export::Formatter,
		) -> serde::export::fmt::Result {
			serde::export::Formatter::write_str(formatter, "enum io::ErrorKind")
		}
		fn visit_enum<A>(self, data: A) -> serde::export::Result<Self::Value, A::Error>
		where
			A: serde::de::EnumAccess<'de>,
		{
			match match serde::de::EnumAccess::variant(data) {
				serde::export::Ok(val) => val,
				serde::export::Err(err) => {
					return serde::export::Err(err);
				}
			} {
				(Field::field0, variant) => {
					match serde::de::VariantAccess::unit_variant(variant) {
						serde::export::Ok(val) => val,
						serde::export::Err(err) => {
							return serde::export::Err(err);
						}
					};
					serde::export::Ok(io::ErrorKind::NotFound)
				}
				(Field::field1, variant) => {
					match serde::de::VariantAccess::unit_variant(variant) {
						serde::export::Ok(val) => val,
						serde::export::Err(err) => {
							return serde::export::Err(err);
						}
					};
					serde::export::Ok(io::ErrorKind::PermissionDenied)
				}
				(Field::field2, variant) => {
					match serde::de::VariantAccess::unit_variant(variant) {
						serde::export::Ok(val) => val,
						serde::export::Err(err) => {
							return serde::export::Err(err);
						}
					};
					serde::export::Ok(io::ErrorKind::ConnectionRefused)
				}
				(Field::field3, variant) => {
					match serde::de::VariantAccess::unit_variant(variant) {
						serde::export::Ok(val) => val,
						serde::export::Err(err) => {
							return serde::export::Err(err);
						}
					};
					serde::export::Ok(io::ErrorKind::ConnectionReset)
				}
				(Field::field4, variant) => {
					match serde::de::VariantAccess::unit_variant(variant) {
						serde::export::Ok(val) => val,
						serde::export::Err(err) => {
							return serde::export::Err(err);
						}
					};
					serde::export::Ok(io::ErrorKind::ConnectionAborted)
				}
				(Field::field5, variant) => {
					match serde::de::VariantAccess::unit_variant(variant) {
						serde::export::Ok(val) => val,
						serde::export::Err(err) => {
							return serde::export::Err(err);
						}
					};
					serde::export::Ok(io::ErrorKind::NotConnected)
				}
				(Field::field6, variant) => {
					match serde::de::VariantAccess::unit_variant(variant) {
						serde::export::Ok(val) => val,
						serde::export::Err(err) => {
							return serde::export::Err(err);
						}
					};
					serde::export::Ok(io::ErrorKind::AddrInUse)
				}
				(Field::field7, variant) => {
					match serde::de::VariantAccess::unit_variant(variant) {
						serde::export::Ok(val) => val,
						serde::export::Err(err) => {
							return serde::export::Err(err);
						}
					};
					serde::export::Ok(io::ErrorKind::AddrNotAvailable)
				}
				(Field::field8, variant) => {
					match serde::de::VariantAccess::unit_variant(variant) {
						serde::export::Ok(val) => val,
						serde::export::Err(err) => {
							return serde::export::Err(err);
						}
					};
					serde::export::Ok(io::ErrorKind::BrokenPipe)
				}
				(Field::field9, variant) => {
					match serde::de::VariantAccess::unit_variant(variant) {
						serde::export::Ok(val) => val,
						serde::export::Err(err) => {
							return serde::export::Err(err);
						}
					};
					serde::export::Ok(io::ErrorKind::AlreadyExists)
				}
				(Field::field10, variant) => {
					match serde::de::VariantAccess::unit_variant(variant) {
						serde::export::Ok(val) => val,
						serde::export::Err(err) => {
							return serde::export::Err(err);
						}
					};
					serde::export::Ok(io::ErrorKind::WouldBlock)
				}
				(Field::field11, variant) => {
					match serde::de::VariantAccess::unit_variant(variant) {
						serde::export::Ok(val) => val,
						serde::export::Err(err) => {
							return serde::export::Err(err);
						}
					};
					serde::export::Ok(io::ErrorKind::InvalidInput)
				}
				(Field::field12, variant) => {
					match serde::de::VariantAccess::unit_variant(variant) {
						serde::export::Ok(val) => val,
						serde::export::Err(err) => {
							return serde::export::Err(err);
						}
					};
					serde::export::Ok(io::ErrorKind::InvalidData)
				}
				(Field::field13, variant) => {
					match serde::de::VariantAccess::unit_variant(variant) {
						serde::export::Ok(val) => val,
						serde::export::Err(err) => {
							return serde::export::Err(err);
						}
					};
					serde::export::Ok(io::ErrorKind::TimedOut)
				}
				(Field::field14, variant) => {
					match serde::de::VariantAccess::unit_variant(variant) {
						serde::export::Ok(val) => val,
						serde::export::Err(err) => {
							return serde::export::Err(err);
						}
					};
					serde::export::Ok(io::ErrorKind::WriteZero)
				}
				(Field::field15, variant) => {
					match serde::de::VariantAccess::unit_variant(variant) {
						serde::export::Ok(val) => val,
						serde::export::Err(err) => {
							return serde::export::Err(err);
						}
					};
					serde::export::Ok(io::ErrorKind::Interrupted)
				}
				(Field::field16, variant) => {
					match serde::de::VariantAccess::unit_variant(variant) {
						serde::export::Ok(val) => val,
						serde::export::Err(err) => {
							return serde::export::Err(err);
						}
					};
					serde::export::Ok(io::ErrorKind::Other)
				}
				(Field::field17, variant) => {
					match serde::de::VariantAccess::unit_variant(variant) {
						serde::export::Ok(val) => val,
						serde::export::Err(err) => {
							return serde::export::Err(err);
						}
					};
					serde::export::Ok(io::ErrorKind::UnexpectedEof)
				}
			}
		}
	}
	const VARIANTS: &'static [&'static str] = &[
		"NotFound",
		"PermissionDenied",
		"ConnectionRefused",
		"ConnectionReset",
		"ConnectionAborted",
		"NotConnected",
		"AddrInUse",
		"AddrNotAvailable",
		"BrokenPipe",
		"AlreadyExists",
		"WouldBlock",
		"InvalidInput",
		"InvalidData",
		"TimedOut",
		"WriteZero",
		"Interrupted",
		"Other",
		"UnexpectedEof",
	];
	serde::Deserializer::deserialize_enum(
		deserializer,
		"ErrorKind",
		VARIANTS,
		Visitor {
			marker: serde::export::PhantomData::<io::ErrorKind>,
			lifetime: serde::export::PhantomData,
		},
	)
}
