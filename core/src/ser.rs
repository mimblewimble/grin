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

use std::time::Duration;

use byteorder::{BigEndian, ByteOrder, ReadBytesExt};
use core::hash::{Hash, Hashed};
use keychain::{BlindingFactor, Identifier, IDENTIFIER_SIZE};
use std::fmt::Debug;
use std::io::{self, Read, Write};
use std::marker;
use std::{cmp, error, fmt};
use util::read_write::read_exact;
use util::secp::constants::{
	AGG_SIGNATURE_SIZE, MAX_PROOF_SIZE, PEDERSEN_COMMITMENT_SIZE, SECRET_KEY_SIZE,
};
use util::secp::pedersen::{Commitment, RangeProof};
use util::secp::Signature;

/// Possible errors deriving from serializing or deserializing.
#[derive(Clone, Eq, PartialEq, Debug)]
pub enum Error {
	/// Wraps an io error produced when reading or writing
	IOErr(String, io::ErrorKind),
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
}

impl From<io::Error> for Error {
	fn from(e: io::Error) -> Error {
		Error::IOErr(format!("{}", e), e.kind())
	}
}

impl fmt::Display for Error {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
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
		}
	}
}

impl error::Error for Error {
	fn cause(&self) -> Option<&error::Error> {
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
	reader: &'a mut Reader,
	_marker: marker::PhantomData<T>,
}

impl<'a, T> IteratingReader<'a, T> {
	/// Constructor to create a new iterating reader for the provided underlying reader.
	/// Takes a count so we know how many to iterate over.
	pub fn new(reader: &'a mut Reader, count: u64) -> IteratingReader<'a, T> {
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
pub fn read_multi<T>(reader: &mut Reader, count: u64) -> Result<Vec<T>, Error>
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

/// Trait that every type that can be deserialized from binary must implement.
/// Reads directly to a Reader, a utility type thinly wrapping an
/// underlying Read implementation.
pub trait Readable
where
	Self: Sized,
{
	/// Reads the data necessary to this Readable from the provided reader
	fn read(reader: &mut Reader) -> Result<Self, Error>;
}

/// Deserializes a Readable from any std::io::Read implementation.
pub fn deserialize<T: Readable>(source: &mut Read) -> Result<T, Error> {
	let mut reader = BinReader { source };
	T::read(&mut reader)
}

/// Serializes a Writeable into any std::io::Write implementation.
pub fn serialize<W: Writeable>(sink: &mut Write, thing: &W) -> Result<(), Error> {
	let mut writer = BinWriter { sink };
	thing.write(&mut writer)
}

/// Utility function to serialize a writeable directly in memory using a
/// Vec<u8>.
pub fn ser_vec<W: Writeable>(thing: &W) -> Result<Vec<u8>, Error> {
	let mut vec = vec![];
	serialize(&mut vec, thing)?;
	Ok(vec)
}

/// Utility to read from a binary source
struct BinReader<'a> {
	source: &'a mut Read,
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
}

/// A reader that reads straight off a stream.
/// Tracks total bytes read so we can verify we read the right number afterwards.
pub struct StreamingReader<'a> {
	total_bytes_read: u64,
	stream: &'a mut Read,
	timeout: Duration,
}

impl<'a> StreamingReader<'a> {
	/// Create a new streaming reader with the provided underlying stream.
	/// Also takes a duration to be used for each individual read_exact call.
	pub fn new(stream: &'a mut Read, timeout: Duration) -> StreamingReader<'a> {
		StreamingReader {
			total_bytes_read: 0,
			stream,
			timeout,
		}
	}

	/// Returns the total bytes read via this streaming reader.
	pub fn total_bytes_read(&self) -> u64 {
		self.total_bytes_read
	}
}

impl<'a> Reader for StreamingReader<'a> {
	fn read_u8(&mut self) -> Result<u8, Error> {
		let buf = self.read_fixed_bytes(1)?;
		deserialize(&mut &buf[..])
	}

	fn read_u16(&mut self) -> Result<u16, Error> {
		let buf = self.read_fixed_bytes(2)?;
		deserialize(&mut &buf[..])
	}

	fn read_u32(&mut self) -> Result<u32, Error> {
		let buf = self.read_fixed_bytes(4)?;
		deserialize(&mut &buf[..])
	}

	fn read_i32(&mut self) -> Result<i32, Error> {
		let buf = self.read_fixed_bytes(4)?;
		deserialize(&mut &buf[..])
	}

	fn read_u64(&mut self) -> Result<u64, Error> {
		let buf = self.read_fixed_bytes(8)?;
		deserialize(&mut &buf[..])
	}

	fn read_i64(&mut self) -> Result<i64, Error> {
		let buf = self.read_fixed_bytes(8)?;
		deserialize(&mut &buf[..])
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
		read_exact(&mut self.stream, &mut buf, self.timeout, true)?;
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
}

impl Readable for Commitment {
	fn read(reader: &mut Reader) -> Result<Commitment, Error> {
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
	fn read(reader: &mut Reader) -> Result<BlindingFactor, Error> {
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
	fn read(reader: &mut Reader) -> Result<Identifier, Error> {
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
	fn read(reader: &mut Reader) -> Result<RangeProof, Error> {
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
	fn read(reader: &mut Reader) -> Result<Signature, Error> {
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
	sink: &'a mut Write,
}

impl<'a> BinWriter<'a> {
	/// Wraps a standard Write in a new BinWriter
	pub fn new(write: &'a mut Write) -> BinWriter<'a> {
		BinWriter { sink: write }
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
}

macro_rules! impl_int {
	($int:ty, $w_fn:ident, $r_fn:ident) => {
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
	fn read(reader: &mut Reader) -> Result<Vec<T>, Error> {
		let mut buf = Vec::new();
		loop {
			let elem = T::read(reader);
			match elem {
				Ok(e) => buf.push(e),
				Err(Error::IOErr(ref _d, ref kind)) if *kind == io::ErrorKind::UnexpectedEof => {
					break
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
	fn read(reader: &mut Reader) -> Result<(A, B), Error> {
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
	fn read(reader: &mut Reader) -> Result<(A, B, C), Error> {
		Ok((
			Readable::read(reader)?,
			Readable::read(reader)?,
			Readable::read(reader)?,
		))
	}
}

impl<A: Readable, B: Readable, C: Readable, D: Readable> Readable for (A, B, C, D) {
	fn read(reader: &mut Reader) -> Result<(A, B, C, D), Error> {
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
pub trait PMMRable: Writeable + Clone + Debug {
	/// The type of element actually stored in the MMR data file.
	/// This allows us to store Hash elements in the header MMR for variable size BlockHeaders.
	type E: FixedLength + Readable + Writeable;

	/// Convert the pmmrable into the element to be stored in the MMR data file.
	fn as_elmt(&self) -> Self::E;
}

/// Generic trait to ensure PMMR elements can be hashed with an index
pub trait PMMRIndexHashable {
	/// Hash with a given index
	fn hash_with_index(&self, index: u64) -> Hash;
}

impl<T: Writeable> PMMRIndexHashable for T {
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
impl AsFixedBytes for ::core::hash::Hash {
	fn len(&self) -> usize {
		32
	}
}
impl AsFixedBytes for ::util::secp::pedersen::RangeProof {
	fn len(&self) -> usize {
		self.plen
	}
}
impl AsFixedBytes for ::util::secp::Signature {
	fn len(&self) -> usize {
		64
	}
}
impl AsFixedBytes for ::util::secp::pedersen::Commitment {
	fn len(&self) -> usize {
		PEDERSEN_COMMITMENT_SIZE
	}
}
impl AsFixedBytes for BlindingFactor {
	fn len(&self) -> usize {
		SECRET_KEY_SIZE
	}
}
impl AsFixedBytes for ::keychain::Identifier {
	fn len(&self) -> usize {
		IDENTIFIER_SIZE
	}
}
