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

//! Provides wrappers for throttling readers and writers

use std::time::Instant;
use std::io;

use futures::*;
use tokio_io::*;

/// A Rate Limited Reader
#[derive(Debug)]
pub struct ThrottledReader<R: AsyncRead> {
	reader: R,
	/// Max Bytes per second
	max: u32,
	/// Stores a count of last request and last request time
	allowed: usize,
	last_check: Instant,
}

#[allow(dead_code)]
impl<R: AsyncRead> ThrottledReader<R> {
	/// Adds throttling to a reader.
	/// The resulting reader will read at most `max` amount of bytes per second
	pub fn new(reader: R, max: u32) -> Self {
		ThrottledReader {
			reader: reader,
			max: max,
			allowed: max as usize,
			last_check: Instant::now(),
		}
	}

	/// Get a shared reference to the inner sink.
	pub fn get_ref(&self) -> &R {
		&self.reader
	}

	/// Get a mutable reference to the inner sink.
	pub fn get_mut(&mut self) -> &mut R {
		&mut self.reader
	}

	/// Consumes this combinator, returning the underlying sink.
	///
	/// Note that this may discard intermediate state of this combinator, so
	/// care should be taken to avoid losing resources when this is called.
	pub fn into_inner(self) -> R {
		self.reader
	}
}

impl<R: AsyncRead> io::Read for ThrottledReader<R> {
	fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
		// Check passed Time
		let time_passed = self.last_check.elapsed();
		self.last_check = Instant::now();
		self.allowed += time_passed.as_secs() as usize * self.max as usize;

		// Throttle
		if self.allowed > self.max as usize {
			self.allowed = self.max as usize;
		}

		// Check if Allowed
		if self.allowed < 1 {
			return Err(io::Error::new(
				io::ErrorKind::WouldBlock,
				"Reached Allowed Read Limit",
			));
		}

		// Read Max Allowed
		let buf = if buf.len() > self.allowed {
			&mut buf[0..self.allowed]
		} else {
			buf
		};
		let res = self.reader.read(buf);

		// Decrement Allowed amount written
		if let Ok(n) = res {
			self.allowed -= n;
		}
		res
	}
}

impl<R: AsyncRead> AsyncRead for ThrottledReader<R> {}

/// A Rate Limited Writer
#[derive(Debug)]
pub struct ThrottledWriter<W: AsyncWrite> {
	writer: W,
	/// Max Bytes per second
	max: u32,
	/// Stores a count of last request and last request time
	allowed: usize,
	last_check: Instant,
}

#[allow(dead_code)]
impl<W: AsyncWrite> ThrottledWriter<W> {
	/// Adds throttling to a writer.
	/// The resulting writer will write at most `max` amount of bytes per second
	pub fn new(writer: W, max: u32) -> Self {
		ThrottledWriter {
			writer: writer,
			max: max,
			allowed: max as usize,
			last_check: Instant::now(),
		}
	}

	/// Get a shared reference to the inner sink.
	pub fn get_ref(&self) -> &W {
		&self.writer
	}

	/// Get a mutable reference to the inner sink.
	pub fn get_mut(&mut self) -> &mut W {
		&mut self.writer
	}

	/// Consumes this combinator, returning the underlying sink.
	///
	/// Note that this may discard intermediate state of this combinator, so
	/// care should be taken to avoid losing resources when this is called.
	pub fn into_inner(self) -> W {
		self.writer
	}
}

impl<W: AsyncWrite> io::Write for ThrottledWriter<W> {
	fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
		// Check passed Time
		let time_passed = self.last_check.elapsed();
		self.last_check = Instant::now();
		self.allowed += time_passed.as_secs() as usize * self.max as usize;

		// Throttle
		if self.allowed > self.max as usize {
			self.allowed = self.max as usize;
		}

		// Check if Allowed
		if self.allowed < 1 {
			return Err(io::Error::new(
				io::ErrorKind::WouldBlock,
				"Reached Allowed Write Limit",
			));
		}

		// Write max allowed
		let buf = if buf.len() > self.allowed {
			&buf[0..self.allowed]
		} else {
			buf
		};
		let res = self.writer.write(buf);

		// Decrement Allowed amount written
		if let Ok(n) = res {
			self.allowed -= n;
		}
		res
	}

	fn flush(&mut self) -> io::Result<()> {
		self.writer.flush()
	}
}

impl<T: AsyncWrite> AsyncWrite for ThrottledWriter<T> {
	fn shutdown(&mut self) -> Poll<(), io::Error> {
		self.writer.shutdown()
	}
}

#[cfg(test)]
mod test {
	use super::*;
	use std::io::Cursor;

	#[test]
	fn should_throttle_write() {
		let buf = vec![0; 64];
		let mut t_buf = ThrottledWriter::new(Cursor::new(buf), 8);

		for _ in 0..16 {
			let _ = t_buf.write_buf(&mut Cursor::new(vec![1; 8]));
		}

		let cursor = t_buf.into_inner();
		assert_eq!(cursor.position(), 8);
	}

	#[test]
	fn should_throttle_read() {
		let buf = vec![1; 64];
		let mut t_buf = ThrottledReader::new(Cursor::new(buf), 8);

		let mut dst = Cursor::new(vec![0; 64]);

		for _ in 0..16 {
			let _ = t_buf.read_buf(&mut dst);
		}

		assert_eq!(dst.position(), 8);
	}
}
