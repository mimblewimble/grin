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

use p2p::msg::{PROTOCOL_VERSION, USER_AGENT};
use term;

pub fn show_status() {
    println!();
    let title=format!("Grin Server Status ");
    let mut t = term::stdout().unwrap();
    let mut e = term::stdout().unwrap();
    t.fg(term::color::MAGENTA).unwrap();
    writeln!(t, "{}", title).unwrap();
    writeln!(t, "--------------------------").unwrap();
    t.reset().unwrap();
    writeln!(e, "Protocol version: {}", PROTOCOL_VERSION).unwrap();
    writeln!(e, "User agent: {}", USER_AGENT).unwrap();
    e.reset().unwrap();
    println!();
}
