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

//! Custom impls of read_exact and write_all to work around async stream restrictions.

use std::io;
use std::io::prelude::*;
use std::thread;
use std::time::Duration;

/// The default implementation of read_exact is useless with an async stream (TcpStream) as
/// it will return as soon as something has been read, regardless of
/// whether the buffer has been filled (and then errors). This implementation
/// will block until it has read exactly `len` bytes and returns them as a
/// `vec<u8>`. Except for a timeout, this implementation will never return a
/// partially filled buffer.
///
/// The timeout in milliseconds aborts the read when it's met. Note that the
/// time is not guaranteed to be exact. To support cases where we want to poll
/// instead of blocking, a `block_on_empty` boolean, when false, ensures
/// `read_exact` returns early with a `io::ErrorKind::WouldBlock` if nothing
/// has been read from the socket.
pub fn read_exact(
	stream: &mut Read,
	mut buf: &mut [u8],
	timeout: Duration,
	block_on_empty: bool,
) -> io::Result<()> {
	let sleep_time = Duration::from_micros(10);
	let mut count = Duration::new(0, 0);

	let mut read = 0;
	loop {
		match stream.read(buf) {
			Ok(0) => {
				return Err(io::Error::new(
					io::ErrorKind::ConnectionAborted,
					"read_exact",
				));
			}
			Ok(n) => {
				let tmp = buf;
				buf = &mut tmp[n..];
				read += n;
			}
			Err(ref e) if e.kind() == io::ErrorKind::Interrupted => {}
			Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
				if read == 0 && !block_on_empty {
					return Err(io::Error::new(io::ErrorKind::WouldBlock, "read_exact"));
				}
			}
			Err(e) => return Err(e),
		}
		if !buf.is_empty() {
			thread::sleep(sleep_time);
			count += sleep_time;
		} else {
			break;
		}
		if count > timeout {
			return Err(io::Error::new(
				io::ErrorKind::TimedOut,
				"reading from stream",
			));
		}
	}
	Ok(())
}

/// Same as `read_exact` but for writing.
pub fn write_all(stream: &mut Write, mut buf: &[u8], timeout: Duration) -> io::Result<()> {
	let sleep_time = Duration::from_micros(10);
	let mut count = Duration::new(0, 0);

	while !buf.is_empty() {
		match stream.write(buf) {
			Ok(0) => {
				return Err(io::Error::new(
					io::ErrorKind::WriteZero,
					"failed to write whole buffer",
				))
			}
			Ok(n) => buf = &buf[n..],
			Err(ref e) if e.kind() == io::ErrorKind::Interrupted => {}
			Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
			Err(e) => return Err(e),
		}
		if !buf.is_empty() {
			thread::sleep(sleep_time);
			count += sleep_time;
		} else {
			break;
		}
		if count > timeout {
			return Err(io::Error::new(io::ErrorKind::TimedOut, "writing to stream"));
		}
	}
	Ok(())
}
