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
// Connects 2 peers and spams Ping/Pong requests at (X+ MB/s)
#[test]
fn test_receive_and_send_rate() {
	env_logger::init().unwrap();

	let mut evtlp = Core::new().unwrap();
	let handle = evtlp.handle();

    // Set a max of 8 bytes per second for recieving and sending
	let mut p2p_conf = p2p::P2PConfig::default();
    p2p_conf.max_send_rate = 8;
    p2p_conf.max_receive_rate = 8;

	let net_adapter = Arc::new(p2p::DummyAdapter {});
	let server = p2p::Server::new(p2p::UNKNOWN, p2p_conf, net_adapter.clone());

	let run_server = server.start(handle.clone());
	let my_addr = "127.0.0.1:5000".parse().unwrap();

	let phandle = handle.clone();
	let rhandle = handle.clone();

	let timeout = reactor::Timeout::new(time::Duration::new(1, 0), &handle).unwrap();
	let timeout_send = reactor::Timeout::new(time::Duration::new(2, 0), &handle).unwrap();

	handle.spawn(timeout.from_err().and_then(move |_| {
		let addr = SocketAddr::new(p2p_conf.host, p2p_conf.port);

		let socket = TcpStream::connect(&addr, &phandle).map_err(|e| p2p::Error::Connection(e));

        socket.and_then(move |socket| {
				Peer::connect(socket,
				              p2p::UNKNOWN,
				              Difficulty::one(),
				              my_addr,
				              &p2p::handshake::Handshake::new())
			})
			.and_then(move |(socket, peer)| {
				rhandle.spawn(peer.run(socket, net_adapter.clone()).map_err(|e| {
					panic!("Client run failed: {:?}", e);
				}));
                
                // Assuming the MsgHeader size is 8 bytes.
                // Send 100 pings or 800 bytes
                // We assume this loop takes less than 1000ms
                for _ in 0..100 {
                    peer.send_ping().unwrap();
                }
                
                // After 1 sec...
				timeout_send.from_err().map(|_| peer)
			})
            .and_then(move |peer| {
                // Amounts received and sent should be no more than 8 bytes
                let (sent, recv) = peer.transmitted_bytes();
                assert!(sent < p2p_conf.max_send_rate, "Send rate not throttled");
                assert!(recv < p2p_conf.max_receive_rate, "Recieve rate not throttled");
                Ok(())
            })
	    }).map_err(|e| {
            panic!("Client connection failed: {:?}", e);
        }));

  evtlp.run(run_server).unwrap();

}