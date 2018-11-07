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

/// Utility to track the rate of data transfers
use std::time::{Duration, SystemTime};

/// A rate counter tracks the number of transfers, the amount of data
/// exchanged and the rate of transfer (via a few timers) over the last
/// minute. The counter does not try to be accurate and update times
/// proactively, instead it only does so lazily. As a result, produced
/// rates are worst-case estimates.
pub struct RateCounter {
	last_min_bytes: Vec<u64>,
	last_min_times: Vec<u64>,
}

impl RateCounter {
	/// Instantiate a new rate counter
	pub fn new() -> RateCounter {
		RateCounter {
			last_min_bytes: vec![],
			last_min_times: vec![],
		}
	}

	/// Increments number of bytes transferred, updating counts and rates.
	pub fn inc(&mut self, bytes: u64) {
		let now_millis = millis_since_epoch();
		self.last_min_times.push(now_millis);
		self.last_min_bytes.push(bytes);
		while self.last_min_times.len() > 0 && self.last_min_times[0] > now_millis + 60000 {
			self.last_min_times.remove(0);
			self.last_min_bytes.remove(0);
		}
	}

	/// Number of bytes counted in the last minute
	pub fn bytes_per_min(&self) -> u64 {
		self.last_min_bytes.iter().sum()
	}

	/// Count of increases in the last minute
	pub fn count_per_min(&self) -> u64 {
		self.last_min_bytes.len() as u64
	}
}

// turns out getting the millisecs since epoch in Rust isn't as easy as it
// could be
fn millis_since_epoch() -> u64 {
	let since_epoch = SystemTime::now()
		.duration_since(SystemTime::UNIX_EPOCH)
		.unwrap_or(Duration::new(0, 0));
	since_epoch.as_secs() * 1000 + since_epoch.subsec_millis() as u64
}
