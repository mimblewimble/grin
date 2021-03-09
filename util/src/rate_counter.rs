// Copyright 2021 The Grin Developers
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

use std::convert::TryInto;
/// Utility to track the rate of data transfers
use std::time::SystemTime;

struct Entry {
	bytes: u64,
	timestamp: u64,
}

impl Entry {
	fn new(bytes: u64) -> Entry {
		Entry {
			bytes,
			timestamp: millis_since_epoch(),
		}
	}

	// Create new "quiet" entry with zero timestamp.
	// This will count toward total bytes but will not affect the "msg rate".
	fn new_quiet(bytes: u64) -> Entry {
		Entry {
			bytes,
			timestamp: 0,
		}
	}

	// We want to filter out "quiet" entries when calculating the "msg rate".
	fn is_quiet(&self) -> bool {
		self.timestamp == 0
	}
}

/// A rate counter tracks the number of transfers, the amount of data
/// exchanged and the rate of transfer (via a few timers) over the last
/// minute. The counter does not try to be accurate and update times
/// proactively, instead it only does so lazily. As a result, produced
/// rates are worst-case estimates.
pub struct RateCounter {
	last_min_entries: Vec<Entry>,
}

impl RateCounter {
	/// Instantiate a new rate counter
	pub fn new() -> RateCounter {
		RateCounter {
			last_min_entries: vec![],
		}
	}

	/// Increments number of bytes transferred, updating counts and rates.
	pub fn inc(&mut self, bytes: u64) {
		self.last_min_entries.push(Entry::new(bytes));
		self.truncate();
	}

	/// Increments number of bytes without updating the count or rate.
	/// We filter out 0 last_min_times when calculating rate.
	/// Used during txhashset.zip download to track bytes downloaded
	/// without treating a peer as abusive (too high a rate of download).
	pub fn inc_quiet(&mut self, bytes: u64) {
		self.last_min_entries.push(Entry::new_quiet(bytes));
		self.truncate();
	}

	fn truncate(&mut self) {
		let now_millis = millis_since_epoch();
		while !self.last_min_entries.is_empty()
			&& self.last_min_entries[0].timestamp + 60000 < now_millis
		{
			self.last_min_entries.remove(0);
		}
	}

	/// Number of bytes counted in the last minute.
	/// Includes "quiet" byte increments.
	pub fn bytes_per_min(&self) -> u64 {
		self.last_min_entries.iter().map(|x| x.bytes).sum()
	}

	/// Count of increases in the last minute.
	/// Excludes "quiet" byte increments.
	pub fn count_per_min(&self) -> u64 {
		self.last_min_entries
			.iter()
			.filter(|x| !x.is_quiet())
			.count() as u64
	}

	/// Elapsed time in ms since the last entry.
	/// We use this to rate limit when sending.
	pub fn elapsed_since_last_msg(&self) -> Option<u64> {
		self.last_min_entries
			.last()
			.map(|x| millis_since_epoch().saturating_sub(x.timestamp))
	}
}

// turns out getting the millisecs since epoch in Rust isn't as easy as it
// could be
fn millis_since_epoch() -> u64 {
	SystemTime::now()
		.duration_since(SystemTime::UNIX_EPOCH)
		.map(|since_epoch| since_epoch.as_millis().try_into().unwrap_or(0))
		.unwrap_or(0)
}
