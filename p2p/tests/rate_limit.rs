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

extern crate grin_core as core;
extern crate grin_p2p as p2p;
extern crate env_logger;
extern crate futures;
extern crate tokio_core;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time;

use futures::future::Future;
use tokio_core::net::TcpStream;
use tokio_core::reactor::{self, Core};

use core::ser;
use core::core::target::Difficulty;
use p2p::Peer;

// Tests for Rate Limiting on Receive (X MB/s)
// Starts a server and connects a client peer
// Client Peer spams server with (X+ MB/s)
#[test]
fn test_receive_rate() {
    unimplemented!()
}

// Tests for Rate Limiting on Send (Y MB/s)
// Starts a server and connects a client peer
// Server spams peer with (Y+ MB/s)
#[test]
fn test_send_rate() {
    unimplemented!()
}