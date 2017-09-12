// Copyright 2017 The Grin Developers
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

#![feature(test)]

extern crate test;
extern crate rand;
extern crate grin_core as core;
extern crate secp256k1zkp as secp;

use core::core::{DEFAULT_OUTPUT, Output};
use secp::Secp256k1;
use secp::key::SecretKey;
use secp::pedersen::ProofMessage;

use test::Bencher;
use rand::os::OsRng;

#[bench]
fn bench_successful_recovery(b: &mut Bencher) {
    let secp = Secp256k1::with_caps(secp::ContextFlag::Commit);

    let message = ProofMessage::empty();

    let key = SecretKey::new(&secp, &mut OsRng::new().unwrap());

    let commit = secp.commit(1, key).unwrap();
    let range_proof = secp.range_proof(0, 1, key, commit, &message);
    let output = Output {
		features: DEFAULT_OUTPUT,
		commit: commit,
		proof: range_proof,
	};

    b.iter(|| {
         output.recover_value(&secp, key);            
    })
}

#[bench]
fn bench_unsuccessful_recovery(b: &mut Bencher) {
    let secp = Secp256k1::with_caps(secp::ContextFlag::Commit);

    let message = ProofMessage::empty();

    let key = SecretKey::new(&secp, &mut OsRng::new().unwrap());

    let commit = secp.commit(1, key).unwrap();
    let range_proof = secp.range_proof(0, 1, key, commit, &message);
    let output = Output {
		features: DEFAULT_OUTPUT,
		commit: commit,
		proof: range_proof,
	};

    let bad_key = SecretKey::new(&secp, &mut OsRng::new().unwrap());
    b.iter(|| {
        output.recover_value(&secp, bad_key);
    })
}
