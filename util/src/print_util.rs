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

use chrono::Local;

/// Format an f64 to same lengths. 5 decimals.
pub fn format_f64(value: f64) -> String {
	let value = value * 100_000 as f64;
	let value = value.floor();
	let mut str = format!("{}", value / 100_000 as f64);
	loop {
		if str.len() >= 7 {
			break;
		}

		str = format!("{}0", str);
	}
	str
}

/// Print this message to stdout in a nice formatted way
pub fn print(msg: String) {
	let date = Local::now();
	let formatted_ts = date.format("%Y-%m-%d %H:%M:%S");
	println!("[{}]: {}", formatted_ts, msg);
}
