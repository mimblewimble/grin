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

//! Wire-format tests for HEADER_PROOF_HIST messages (types 31/32).

use std::sync::OnceLock;

use grin_core::core::hash::{Hash, ZERO_HASH};
use grin_core::core::BlockHeader;
use grin_core::global;
use grin_core::pow;
use grin_core::ser::{self, DeserializationMode, ProtocolVersion, Readable, Writeable};
use grin_p2p as p2p;
use grin_p2p::header_proof::verify_memberships;
use grin_p2p::msg::{GetHeaderProofs, HeaderProofItem, HeaderProofs};

fn init_chain() {
	global::set_local_chain_type(global::ChainTypes::AutomatedTesting);
}

/// A header with a *valid* PoW so that `UntrustedBlockHeader` deserialization
/// (used by the `Header`/`HeaderProofs` wire format) accepts it.
///
/// Mined once per process for the AutomatedTesting chain (small PoW graph).
fn valid_header() -> BlockHeader {
	static HEADER: OnceLock<BlockHeader> = OnceLock::new();
	HEADER
		.get_or_init(|| {
			init_chain();
			pow::mine_genesis_block().expect("mine test header").header
		})
		.clone()
}

fn ser<T: Writeable>(value: &T) -> Vec<u8> {
	ser::ser_vec(value, ProtocolVersion::local()).expect("serialize")
}

fn de<T: Readable>(bytes: &[u8]) -> Result<T, ser::Error> {
	ser::deserialize(
		&mut &bytes[..],
		ProtocolVersion::local(),
		DeserializationMode::default(),
	)
}

#[test]
fn type_ids_are_31_and_32() {
	use num::FromPrimitive;
	assert_eq!(
		p2p::msg::Type::from_u8(31),
		Some(p2p::msg::Type::GetHeaderProofs)
	);
	assert_eq!(
		p2p::msg::Type::from_u8(32),
		Some(p2p::msg::Type::HeaderProofs)
	);
}

#[test]
fn capability_and_limits() {
	assert_eq!(p2p::Capabilities::HEADER_PROOF_HIST.bits(), 0x100);
	assert_eq!(p2p::MAX_HEADER_PROOF_ITEMS, 64);
	assert_eq!(p2p::MAX_HEADER_PROOF_PATH, 64);
}

#[test]
fn get_header_proofs_wire_roundtrip() {
	init_chain();
	let anchor = Hash::from_vec(&[0x11; 32]);
	let req = GetHeaderProofs {
		anchor,
		heights: vec![0, 1, 7, 42],
	};
	let bytes = ser(&req);
	// anchor(32) + count(2) + 4 * height(8)
	assert_eq!(bytes.len(), 32 + 2 + 4 * 8);
	let decoded: GetHeaderProofs = de(&bytes).expect("decode request");
	assert_eq!(decoded.anchor, anchor);
	assert_eq!(decoded.heights, req.heights);
}

#[test]
fn get_header_proofs_parser_enforces_wire_bounds_only() {
	init_chain();

	// Not strictly increasing / duplicate heights are *semantic* problems:
	// the parser accepts them, the server handler ignores the request (no ban).
	let unsorted = ser(&GetHeaderProofs {
		anchor: ZERO_HASH,
		heights: vec![5, 5],
	});
	let decoded: GetHeaderProofs = de(&unsorted).expect("parser accepts unsorted heights");
	assert_eq!(decoded.heights, vec![5, 5]);

	// Zero count is accepted by the parser (wire bound only); the handler
	// ignores the empty request.
	let mut empty = vec![0u8; 32];
	empty.extend_from_slice(&0u16.to_be_bytes());
	let decoded: GetHeaderProofs = de(&empty).expect("parser accepts zero count");
	assert!(decoded.heights.is_empty());

	// count > MAX_HEADER_PROOF_ITEMS is a wire bound violation.
	let mut too_many = ser(&GetHeaderProofs {
		anchor: ZERO_HASH,
		heights: vec![1, 2],
	});
	too_many[32..34].copy_from_slice(&(p2p::MAX_HEADER_PROOF_ITEMS + 1).to_be_bytes());
	assert!(de::<GetHeaderProofs>(&too_many).is_err());

	// Truncated height array is a wire error.
	let mut truncated = ser(&GetHeaderProofs {
		anchor: ZERO_HASH,
		heights: vec![1, 2, 3],
	});
	truncated.truncate(truncated.len() - 3);
	assert!(de::<GetHeaderProofs>(&truncated).is_err());
}

#[test]
fn header_proofs_wire_roundtrip() {
	init_chain();
	let anchor = Hash::from_vec(&[0x22; 32]);
	let items = vec![
		HeaderProofItem {
			header: valid_header(),
			path: vec![Hash::from_vec(&[0x33; 32])],
		},
		HeaderProofItem {
			header: valid_header(),
			path: vec![Hash::from_vec(&[0x44; 32]), Hash::from_vec(&[0x55; 32])],
		},
	];
	let proofs = HeaderProofs {
		anchor,
		mmr_size: 9,
		items,
	};
	let bytes = ser(&proofs);
	let decoded: HeaderProofs = de(&bytes).expect("decode proofs");
	assert_eq!(decoded.anchor, anchor);
	assert_eq!(decoded.mmr_size, 9);
	assert_eq!(decoded.items.len(), 2);
	assert_eq!(decoded.items[0].header.height, 0);
	assert_eq!(decoded.items[0].path, proofs.items[0].path);
	assert_eq!(decoded.items[1].header.height, 0);
	assert_eq!(decoded.items[1].path, proofs.items[1].path);

	// The item header uses the same serialization as the Header message.
	let header_bytes = ser(&proofs.items[1].header);
	assert_eq!(
		&bytes[32 + 8 + 2..][..header_bytes.len()],
		&header_bytes[..]
	);
}

#[test]
fn header_proofs_parser_enforces_wire_bounds_only() {
	init_chain();
	let header = valid_header();
	let header_bytes = ser(&header);

	// path_len == MAX_HEADER_PROOF_PATH is accepted.
	let mut path = vec![0u8; p2p::MAX_HEADER_PROOF_PATH as usize * 32];
	// Each hash is a valid 32-byte value (zero hash) - no further validation.
	let mut ok_body = header_bytes.clone();
	ok_body.push(p2p::MAX_HEADER_PROOF_PATH as u8);
	ok_body.append(&mut path);
	let ok_item: HeaderProofItem = de(&ok_body).expect("max path accepted");
	assert_eq!(ok_item.path.len(), p2p::MAX_HEADER_PROOF_PATH as usize);

	// path_len == MAX_HEADER_PROOF_PATH + 1 is a wire bound violation.
	let mut too_long = header_bytes.clone();
	too_long.push((p2p::MAX_HEADER_PROOF_PATH + 1) as u8);
	let err = de::<HeaderProofItem>(&too_long).err();
	assert_eq!(err, Some(ser::Error::TooLargeReadErr));

	// Truncated path is a wire error.
	let mut truncated = ok_body.clone();
	truncated.truncate(truncated.len() - 5);
	assert!(de::<HeaderProofItem>(&truncated).is_err());

	// Zero-count response is accepted by the parser; the client verifier
	// rejects it (semantic validation is not done on the wire).
	let zero_count: HeaderProofs = de(&{
		let mut bytes = vec![0u8; 40];
		// anchor (32) + mmr_size (8) + count = 0
		bytes.extend_from_slice(&0u16.to_be_bytes());
		bytes
	})
	.expect("parser accepts zero count");
	assert!(zero_count.items.is_empty());
	let anchor = BlockHeader {
		height: 1,
		..Default::default()
	};
	assert!(verify_memberships(&anchor, &zero_count).is_err());

	// count > max is a wire bound violation.
	let mut too_many = ser(&HeaderProofs {
		anchor: ZERO_HASH,
		mmr_size: 1,
		items: vec![HeaderProofItem {
			header: valid_header(),
			path: vec![],
		}],
	});
	too_many[40..42].copy_from_slice(&(p2p::MAX_HEADER_PROOF_ITEMS + 1).to_be_bytes());
	assert!(de::<HeaderProofs>(&too_many).is_err());
}

#[test]
fn max_response_size_under_documented_bound() {
	// Documented bound: 64 headers + paths ~= 75 KiB in practice; the wire
	// max_msg_size() bound is 42 + 64 * (max_header_size(578) + 1 + 64 * 32)
	// = 168170 bytes, comfortably below the 4x read slack used by the codec.
	let max_item = 578u64 + 1 + 64 * 32;
	let max_response = 32 + 8 + 2 + max_item * p2p::MAX_HEADER_PROOF_ITEMS as u64;
	assert_eq!(max_response, 168170);
	assert!(max_response < 200_000);
}
