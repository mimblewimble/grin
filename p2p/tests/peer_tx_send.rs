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
extern crate mioco;
extern crate env_logger;
extern crate rand;
extern crate secp256k1zkp as secp;

mod common;

use rand::Rng;
use rand::os::OsRng;
use std::io;
use std::sync::Arc;
use std::time;

use mioco::tcp::TcpStream;
use secp::Secp256k1;
use secp::key::SecretKey;

use core::core::*;
use p2p::Peer;
use common::*;

// Connects a client peer and send a transaction.
#[test]
fn peer_tx_send() {
  with_server(|server| -> io::Result<()> {
    // connect a client peer to the server
    let peer = try!(connect_peer());
		let tx1 = tx2i1o();

    peer.send_transaction(&tx1);
    mioco::sleep(time::Duration::from_millis(50));
    let (sent,_) = peer.transmitted_bytes();
    assert!(sent > 1000);

    let s_peer = server.get_any_peer();
    let (_, recv) = s_peer.transmitted_bytes();
    assert!(recv > 1000);

    peer.stop();

    Ok(())
  });
}

// utility producing a transaction with 2 inputs and a single outputs
pub fn tx2i1o() -> Transaction {
  let mut rng = OsRng::new().unwrap();
	let ref secp = secp::Secp256k1::with_caps(secp::ContextFlag::Commit);

  let outh = core::core::hash::ZERO_HASH;
  Transaction::new(vec![Input::OvertInput {
                          output: outh,
                          value: 10,
                          blindkey: SecretKey::new(secp, &mut rng),
                        },
                        Input::OvertInput {
                          output: outh,
                          value: 11,
                          blindkey: SecretKey::new(secp, &mut rng),
                        }],
                   vec![Output::OvertOutput {
                          value: 20,
                          blindkey: SecretKey::new(secp, &mut rng),
                        }],
                   1).blind(&secp).unwrap()
}
