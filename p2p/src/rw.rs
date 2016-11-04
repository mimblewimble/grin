use std::io::{self, Read, Write, Result};
use core::ser;

/// A Read implementation that counts the number of bytes consumed from an
/// underlying Read.
pub struct CountingRead<'a> {
	counter: usize,
	source: &'a mut Read,
}

impl<'a> CountingRead<'a> {
	/// Creates a new Read wrapping the underlying one, counting bytes consumed
	pub fn new(source: &mut Read) -> CountingRead {
		CountingRead {
			counter: 0,
			source: source,
		}
	}

	/// Number of bytes that have been read from the underlying reader
	pub fn bytes_read(&self) -> usize {
		self.counter
	}
}

impl<'a> Read for CountingRead<'a> {
	fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
		let r = self.source.read(buf);
		if let Ok(sz) = r {
			self.counter += sz;
		}
		r
	}
}

/// A Read implementation that errors out past a maximum number of bytes read.
pub struct LimitedRead<'a> {
	counter: usize,
	max: usize,
	source: &'a mut Read,
}

impl<'a> LimitedRead<'a> {
	/// Creates a new Read wrapping the underlying one, erroring once the
	/// max_read bytes has been reached.
	pub fn new(source: &mut Read, max_read: usize) -> LimitedRead {
		LimitedRead {
			counter: 0,
			max: max_read,
			source: source,
		}
	}
}

impl<'a> Read for LimitedRead<'a> {
	fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
		let r = self.source.read(buf);
		if let Ok(sz) = r {
			self.counter += sz;
		}
		if self.counter > self.max {
			Err(io::Error::new(io::ErrorKind::Interrupted, "Reached read limit."))
		} else {
			r
		}
	}
}

/// A Write implementation that counts the number of bytes wrote to an
/// underlying Write.
struct CountingWrite<'a> {
	counter: usize,
	dest: &'a mut Write,
}

impl<'a> Write for CountingWrite<'a> {
	fn write(&mut self, buf: &[u8]) -> Result<usize> {
		let w = self.dest.write(buf);
		if let Ok(sz) = w {
			self.counter += sz;
		}
		w
	}
	fn flush(&mut self) -> Result<()> {
		self.dest.flush()
	}
}
