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

//! Provides a connection wrapper that handles the lower level tasks in sending
//! or receiving data from the TCP socket, as well as dealing with timeouts.

use std::iter;
use std::ops::Deref;
use std::sync::{Mutex, Arc};
use std::time::{Instant, Duration};
use std::mem;
use std::io;

use futures;
use futures::*;
use futures::sync::mpsc::{Sender, UnboundedSender, UnboundedReceiver};
use tokio_core::io::{Io, WriteHalf, ReadHalf, write_all, read_exact};
use tokio_core::net::TcpStream;
use tokio_timer::{Timer, TimerError};

use core::core::hash::{Hash, ZERO_HASH};
use core::ser;
use msg::*;
use types::Error;

/// A Rate Limited Writer
pub struct ThrottledWriter<W: io::Write> {
    writer: W,
	/// Max Bytes per second
	max: usize,
	/// Stores a count of last request and last request time
	allowed: usize,
	last_check: Instant,
}

impl <W: io::Write> ThrottledWriter<W> {
    /// Adds throttling to a writer
	/// The resulting writer with receive at most `max` amount of bytes per second
	fn from(writer: W, max: usize) -> Self {
		ThrottledWriter {
			writer: writer,
			max: max,
			allowed: max,
			last_check: Instant::now()
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

impl <W: io::Write> io::Write for ThrottledWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let time_passed = self.last_check.elapsed();
		self.last_check = Instant::now();
		self.allowed += time_passed.as_secs() as usize * self.max;

        // Throttle
		if self.allowed > self.max {
			self.allowed = self.max;
		}
        // Write if Allowed
		if self.allowed < 1 {
			return Err(io::Error::new(io::ErrorKind::WouldBlock, "Going over allowed rate limit"));
		}
        
        let buf = if self.allowed < buf.len() { &buf[..self.allowed] } else { buf };
        let n = self.writer.write(buf)?;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}
