// Bitcoin secp256k1 bindings
// Written in 2014 by
//   Dawid Ciężarkiewicz
//   Andrew Poelstra
//
// To the extent possible under law, the author(s) have dedicated all
// copyright and related and neighboring rights to this software to
// the public domain worldwide. This software is distributed without
// any warranty.
//
// You should have received a copy of the CC0 Public Domain Dedication
// along with this software.
// If not, see <http://creativecommons.org/publicdomain/zero/1.0/>.
//

//! # Constants
//! Constants related to the API and the underlying curve

/// The size (in bytes) of a message
pub const MESSAGE_SIZE: usize = 32;

/// The size (in bytes) of a secret key
pub const SECRET_KEY_SIZE: usize = 32;

/// The size (in bytes) of a public key array. This only needs to be 65
/// but must be 72 for compatibility with the `ArrayVec` library.
pub const PUBLIC_KEY_SIZE: usize = 72;

/// The size (in bytes) of an uncompressed public key
pub const UNCOMPRESSED_PUBLIC_KEY_SIZE: usize = 65;

/// The size (in bytes) of a compressed public key
pub const COMPRESSED_PUBLIC_KEY_SIZE: usize = 33;

/// The maximum size of a signature
pub const MAX_SIGNATURE_SIZE: usize = 72;

/// The size of a Schnorr signature
pub const SCHNORR_SIGNATURE_SIZE: usize = 64;

/// The maximum size of a compact signature
pub const COMPACT_SIGNATURE_SIZE: usize = 64;

/// The size of a blinding factor used in a Pedersen commitment
pub const BLINDING_FACTOR_SIZE: usize = 32;

/// The size of a Pedersen commitment
pub const PEDERSEN_COMMITMENT_SIZE: usize = 33;

/// The max size of a range proof
pub const MAX_PROOF_SIZE: usize = 5134;

/// The maximum size of a message embedded in a range proof
pub const PROOF_MSG_SIZE: usize = 4096;

/// The order of the secp256k1 curve
pub const CURVE_ORDER: [u8; 32] = [0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
                                   0xff, 0xff, 0xff, 0xff, 0xff, 0xfe, 0xba, 0xae, 0xdc, 0xe6,
                                   0xaf, 0x48, 0xa0, 0x3b, 0xbf, 0xd2, 0x5e, 0x8c, 0xd0, 0x36,
                                   0x41, 0x41];

/// The X coordinate of the generator
pub const GENERATOR_X: [u8; 32] = [0x79, 0xbe, 0x66, 0x7e, 0xf9, 0xdc, 0xbb, 0xac, 0x55, 0xa0,
                                   0x62, 0x95, 0xce, 0x87, 0x0b, 0x07, 0x02, 0x9b, 0xfc, 0xdb,
                                   0x2d, 0xce, 0x28, 0xd9, 0x59, 0xf2, 0x81, 0x5b, 0x16, 0xf8,
                                   0x17, 0x98];

/// The Y coordinate of the generator
pub const GENERATOR_Y: [u8; 32] = [0x48, 0x3a, 0xda, 0x77, 0x26, 0xa3, 0xc4, 0x65, 0x5d, 0xa4,
                                   0xfb, 0xfc, 0x0e, 0x11, 0x08, 0xa8, 0xfd, 0x17, 0xb4, 0x48,
                                   0xa6, 0x85, 0x54, 0x19, 0x9c, 0x47, 0xd0, 0x8f, 0xfb, 0x10,
                                   0xd4, 0xb8];
