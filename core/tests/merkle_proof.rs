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

mod common;

use self::core::core::merkle_proof::MerkleProof;
use self::core::core::pmmr::{ReadablePMMR, VecBackend, PMMR};
use self::core::ser::{self, PMMRIndexHashable};
use crate::common::TestElem;
use grin_core as core;

#[test]
fn empty_merkle_proof() {
	let proof = MerkleProof::empty();
	assert_eq!(proof.path, vec![]);
	assert_eq!(proof.mmr_size, 0);
}

#[test]
fn merkle_proof_ser_deser() {
	let mut ba = VecBackend::new();
	let mut pmmr = PMMR::new(&mut ba);
	for x in 0..15 {
		pmmr.push(&TestElem([0, 0, 0, x])).unwrap();
	}
	let proof = pmmr.merkle_proof(8).unwrap();

	let mut vec = Vec::new();
	ser::serialize_default(&mut vec, &proof).expect("serialization failed");
	let proof_2: MerkleProof = ser::deserialize_default(&mut &vec[..]).unwrap();

	assert_eq!(proof, proof_2);
}

#[test]
fn pmmr_merkle_proof_prune_and_rewind() {
	let mut ba = VecBackend::new();
	let mut pmmr = PMMR::new(&mut ba);
	pmmr.push(&TestElem([0, 0, 0, 1])).unwrap();
	pmmr.push(&TestElem([0, 0, 0, 2])).unwrap();
	let proof = pmmr.merkle_proof(1).unwrap();

	// now prune an element and check we can still generate
	// the correct Merkle proof for the other element (after sibling pruned)
	pmmr.prune(0).unwrap();
	let proof_2 = pmmr.merkle_proof(1).unwrap();
	assert_eq!(proof, proof_2);
}

#[test]
fn pmmr_merkle_proof() {
	let elems = [
		TestElem([0, 0, 0, 1]),
		TestElem([0, 0, 0, 2]),
		TestElem([0, 0, 0, 3]),
		TestElem([0, 0, 0, 4]),
		TestElem([0, 0, 0, 5]),
		TestElem([0, 0, 0, 6]),
		TestElem([0, 0, 0, 7]),
		TestElem([0, 0, 0, 8]),
		TestElem([1, 0, 0, 0]),
	];

	let mut ba = VecBackend::new();
	let mut pmmr = PMMR::new(&mut ba);

	pmmr.push(&elems[0]).unwrap();
	let pos_0 = elems[0].hash_with_index(0);
	assert_eq!(pmmr.get_hash(0).unwrap(), pos_0);

	let proof = pmmr.merkle_proof(0).unwrap();
	assert_eq!(proof.path, vec![]);
	assert!(proof.verify(pmmr.root().unwrap(), &elems[0], 0).is_ok());

	pmmr.push(&elems[1]).unwrap();
	let pos_1 = elems[1].hash_with_index(1);
	assert_eq!(pmmr.get_hash(1).unwrap(), pos_1);
	let pos_2 = (pos_0, pos_1).hash_with_index(2);
	assert_eq!(pmmr.get_hash(2).unwrap(), pos_2);

	assert_eq!(pmmr.root().unwrap(), pos_2);
	assert_eq!(pmmr.peaks(), vec![pos_2]);

	// single peak, path with single sibling
	let proof = pmmr.merkle_proof(0).unwrap();
	assert_eq!(proof.path, vec![pos_1]);
	assert!(proof.verify(pmmr.root().unwrap(), &elems[0], 0).is_ok());

	let proof = pmmr.merkle_proof(1).unwrap();
	assert_eq!(proof.path, vec![pos_0]);
	assert!(proof.verify(pmmr.root().unwrap(), &elems[1], 1).is_ok());

	// three leaves, two peaks (one also the right-most leaf)
	pmmr.push(&elems[2]).unwrap();
	let pos_3 = elems[2].hash_with_index(3);
	assert_eq!(pmmr.get_hash(3).unwrap(), pos_3);

	assert_eq!(pmmr.root().unwrap(), (pos_2, pos_3).hash_with_index(4));
	assert_eq!(pmmr.peaks(), vec![pos_2, pos_3]);

	let proof = pmmr.merkle_proof(0).unwrap();
	assert_eq!(proof.path, vec![pos_1, pos_3]);
	assert!(proof.verify(pmmr.root().unwrap(), &elems[0], 0).is_ok());

	let proof = pmmr.merkle_proof(1).unwrap();
	assert_eq!(proof.path, vec![pos_0, pos_3]);
	assert!(proof.verify(pmmr.root().unwrap(), &elems[1], 1).is_ok());

	let proof = pmmr.merkle_proof(3).unwrap();
	assert_eq!(proof.path, vec![pos_2]);
	assert!(proof.verify(pmmr.root().unwrap(), &elems[2], 3).is_ok());

	// 7 leaves, 3 peaks, 11 pos in total
	pmmr.push(&elems[3]).unwrap();
	let pos_4 = elems[3].hash_with_index(4);
	assert_eq!(pmmr.get_hash(4).unwrap(), pos_4);
	let pos_5 = (pos_3, pos_4).hash_with_index(5);
	assert_eq!(pmmr.get_hash(5).unwrap(), pos_5);
	let pos_6 = (pos_2, pos_5).hash_with_index(6);
	assert_eq!(pmmr.get_hash(6).unwrap(), pos_6);

	pmmr.push(&elems[4]).unwrap();
	let pos_7 = elems[4].hash_with_index(7);
	assert_eq!(pmmr.get_hash(7).unwrap(), pos_7);

	pmmr.push(&elems[5]).unwrap();
	let pos_8 = elems[5].hash_with_index(8);
	assert_eq!(pmmr.get_hash(8).unwrap(), pos_8);

	let pos_9 = (pos_7, pos_8).hash_with_index(9);
	assert_eq!(pmmr.get_hash(9).unwrap(), pos_9);

	pmmr.push(&elems[6]).unwrap();
	let pos_10 = elems[6].hash_with_index(10);
	assert_eq!(pmmr.get_hash(10).unwrap(), pos_10);

	assert_eq!(pmmr.unpruned_size(), 11);

	let proof = pmmr.merkle_proof(0).unwrap();
	assert_eq!(
		proof.path,
		vec![pos_1, pos_5, (pos_9, pos_10).hash_with_index(11)]
	);
	assert!(proof.verify(pmmr.root().unwrap(), &elems[0], 0).is_ok());

	let proof = pmmr.merkle_proof(1).unwrap();
	assert_eq!(
		proof.path,
		vec![pos_0, pos_5, (pos_9, pos_10).hash_with_index(11)]
	);
	assert!(proof.verify(pmmr.root().unwrap(), &elems[1], 1).is_ok());

	let proof = pmmr.merkle_proof(3).unwrap();
	assert_eq!(
		proof.path,
		vec![pos_4, pos_2, (pos_9, pos_10).hash_with_index(11)]
	);
	assert!(proof.verify(pmmr.root().unwrap(), &elems[2], 3).is_ok());

	let proof = pmmr.merkle_proof(4).unwrap();
	assert_eq!(
		proof.path,
		vec![pos_3, pos_2, (pos_9, pos_10).hash_with_index(11)]
	);
	assert!(proof.verify(pmmr.root().unwrap(), &elems[3], 4).is_ok());

	let proof = pmmr.merkle_proof(7).unwrap();
	assert_eq!(proof.path, vec![pos_8, pos_10, pos_6]);
	assert!(proof.verify(pmmr.root().unwrap(), &elems[4], 7).is_ok());

	let proof = pmmr.merkle_proof(8).unwrap();
	assert_eq!(proof.path, vec![pos_7, pos_10, pos_6]);
	assert!(proof.verify(pmmr.root().unwrap(), &elems[5], 8).is_ok());

	let proof = pmmr.merkle_proof(10).unwrap();
	assert_eq!(proof.path, vec![pos_9, pos_6]);
	assert!(proof.verify(pmmr.root().unwrap(), &elems[6], 10).is_ok());
}
