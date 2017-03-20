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

//! # FFI bindings
//! Direct bindings to the underlying C library functions. These should
//! not be needed for most users.
use std::mem;
use std::hash;

use libc::{c_int, c_uchar, c_uint, c_void, size_t, int64_t, uint64_t};

/// Flag for context to enable no precomputation
pub const SECP256K1_START_NONE: c_uint = (1 << 0) | 0;
/// Flag for context to enable verification precomputation
pub const SECP256K1_START_VERIFY: c_uint = (1 << 0) | (1 << 8);
/// Flag for context to enable signing precomputation
pub const SECP256K1_START_SIGN: c_uint = (1 << 0) | (1 << 9);
/// Flag for keys to indicate uncompressed serialization format
pub const SECP256K1_SER_UNCOMPRESSED: c_uint = (1 << 1) | 0;
/// Flag for keys to indicate compressed serialization format
pub const SECP256K1_SER_COMPRESSED: c_uint = (1 << 1) | (1 << 8);

/// A nonce generation function. Ordinary users of the library
/// never need to see this type; only if you need to control
/// nonce generation do you need to use it. I have deliberately
/// made this hard to do: you have to write your own wrapper
/// around the FFI functions to use it. And it's an unsafe type.
/// Nonces are generated deterministically by RFC6979 by
/// default; there should be no need to ever change this.
pub type NonceFn = unsafe extern "C" fn(nonce32: *mut c_uchar,
                                        msg32: *const c_uchar,
                                        key32: *const c_uchar,
                                        algo16: *const c_uchar,
                                        attempt: c_uint,
                                        data: *const c_void);


/// A Secp256k1 context, containing various precomputed values and such
/// needed to do elliptic curve computations. If you create one of these
/// with `secp256k1_context_create` you MUST destroy it with
/// `secp256k1_context_destroy`, or else you will have a memory leak.
#[derive(Clone, Debug)]
#[repr(C)]
pub struct Context(c_int);

/// Library-internal representation of a Secp256k1 public key
#[repr(C)]
pub struct PublicKey([c_uchar; 64]);
impl_array_newtype!(PublicKey, c_uchar, 64);
impl_raw_debug!(PublicKey);

impl PublicKey {
    /// Create a new (zeroed) public key usable for the FFI interface
    pub fn new() -> PublicKey {
        PublicKey([0; 64])
    }
    /// Create a new (uninitialized) public key usable for the FFI interface
    pub unsafe fn blank() -> PublicKey {
        mem::uninitialized()
    }
}

/// Library-internal representation of a Secp256k1 signature
#[repr(C)]
pub struct Signature([c_uchar; 64]);
impl_array_newtype!(Signature, c_uchar, 64);
impl_raw_debug!(Signature);

/// Library-internal representation of a Secp256k1 signature + recovery ID
#[repr(C)]
pub struct RecoverableSignature([c_uchar; 65]);
impl_array_newtype!(RecoverableSignature, c_uchar, 65);
impl_raw_debug!(RecoverableSignature);

impl Signature {
    /// Create a new (zeroed) signature usable for the FFI interface
    pub fn new() -> Signature {
        Signature([0; 64])
    }
    /// Create a new (uninitialized) signature usable for the FFI interface
    pub unsafe fn blank() -> Signature {
        mem::uninitialized()
    }
}

impl RecoverableSignature {
    /// Create a new (zeroed) signature usable for the FFI interface
    pub fn new() -> RecoverableSignature {
        RecoverableSignature([0; 65])
    }
    /// Create a new (uninitialized) signature usable for the FFI interface
    pub unsafe fn blank() -> RecoverableSignature {
        mem::uninitialized()
    }
}

/// Library-internal representation of an ECDH shared secret
#[repr(C)]
pub struct SharedSecret([c_uchar; 32]);
impl_array_newtype!(SharedSecret, c_uchar, 32);
impl_raw_debug!(SharedSecret);

impl SharedSecret {
    /// Create a new (zeroed) signature usable for the FFI interface
    pub fn new() -> SharedSecret {
        SharedSecret([0; 32])
    }
    /// Create a new (uninitialized) signature usable for the FFI interface
    pub unsafe fn blank() -> SharedSecret {
        mem::uninitialized()
    }
}

extern "C" {
    pub static secp256k1_nonce_function_rfc6979: NonceFn;

    pub static secp256k1_nonce_function_default: NonceFn;

    // Contexts
    pub fn secp256k1_context_create(flags: c_uint) -> *mut Context;

    pub fn secp256k1_context_clone(cx: *mut Context) -> *mut Context;

    pub fn secp256k1_context_destroy(cx: *mut Context);

    pub fn secp256k1_context_randomize(cx: *mut Context, seed32: *const c_uchar) -> c_int;

    pub fn secp256k1_pedersen_context_initialize(ctx: *mut Context);
    pub fn secp256k1_rangeproof_context_initialize(ctx: *mut Context);

    // TODO secp256k1_context_set_illegal_callback
    // TODO secp256k1_context_set_error_callback
    // (Actually, I don't really want these exposed; if either of these
    // are ever triggered it indicates a bug in rust-secp256k1, since
    // one goal is to use Rust's type system to eliminate all possible
    // bad inputs.)

    // Pubkeys
    pub fn secp256k1_ec_pubkey_parse(cx: *const Context,
                                     pk: *mut PublicKey,
                                     input: *const c_uchar,
                                     in_len: size_t)
                                     -> c_int;

    pub fn secp256k1_ec_pubkey_serialize(cx: *const Context,
                                         output: *const c_uchar,
                                         out_len: *mut size_t,
                                         pk: *const PublicKey,
                                         compressed: c_uint)
                                         -> c_int;

    // Signatures
    pub fn secp256k1_ecdsa_signature_parse_der(cx: *const Context,
                                               sig: *mut Signature,
                                               input: *const c_uchar,
                                               in_len: size_t)
                                               -> c_int;

    pub fn ecdsa_signature_parse_der_lax(cx: *const Context,
                                         sig: *mut Signature,
                                         input: *const c_uchar,
                                         in_len: size_t)
                                         -> c_int;

    pub fn secp256k1_ecdsa_signature_serialize_der(cx: *const Context,
                                                   output: *const c_uchar,
                                                   out_len: *mut size_t,
                                                   sig: *const Signature)
                                                   -> c_int;

    pub fn secp256k1_ecdsa_recoverable_signature_parse_compact(cx: *const Context,
                                                               sig: *mut RecoverableSignature,
                                                               input64: *const c_uchar,
                                                               recid: c_int)
                                                               -> c_int;

    pub fn secp256k1_ecdsa_recoverable_signature_serialize_compact(cx: *const Context, output64: *const c_uchar,
                                                                   recid: *mut c_int, sig: *const RecoverableSignature)
                                                                   -> c_int;

    pub fn secp256k1_ecdsa_recoverable_signature_convert(cx: *const Context,
                                                         sig: *mut Signature,
                                                         input: *const RecoverableSignature)
                                                         -> c_int;

    pub fn secp256k1_ecdsa_signature_normalize(cx: *const Context,
                                               out_sig: *mut Signature,
                                               in_sig: *const Signature)
                                               -> c_int;

    // ECDSA
    pub fn secp256k1_ecdsa_verify(cx: *const Context,
                                  sig: *const Signature,
                                  msg32: *const c_uchar,
                                  pk: *const PublicKey)
                                  -> c_int;

    pub fn secp256k1_ecdsa_sign(cx: *const Context,
                                sig: *mut Signature,
                                msg32: *const c_uchar,
                                sk: *const c_uchar,
                                noncefn: NonceFn,
                                noncedata: *const c_void)
                                -> c_int;

    pub fn secp256k1_ecdsa_sign_recoverable(cx: *const Context,
                                            sig: *mut RecoverableSignature,
                                            msg32: *const c_uchar,
                                            sk: *const c_uchar,
                                            noncefn: NonceFn,
                                            noncedata: *const c_void)
                                            -> c_int;

    pub fn secp256k1_ecdsa_recover(cx: *const Context,
                                   pk: *mut PublicKey,
                                   sig: *const RecoverableSignature,
                                   msg32: *const c_uchar)
                                   -> c_int;

    // Schnorr
    pub fn secp256k1_schnorr_sign(cx: *const Context,
                                  sig64: *mut c_uchar,
                                  msg32: *const c_uchar,
                                  sk: *const c_uchar,
                                  noncefn: NonceFn,
                                  noncedata: *const c_void)
                                  -> c_int;

    pub fn secp256k1_schnorr_verify(cx: *const Context,
                                    sig64: *const c_uchar,
                                    msg32: *const c_uchar,
                                    pk: *const PublicKey)
                                    -> c_int;

    pub fn secp256k1_schnorr_recover(cx: *const Context,
                                     pk: *mut PublicKey,
                                     sig64: *const c_uchar,
                                     msg32: *const c_uchar)
                                     -> c_int;

    // EC
    pub fn secp256k1_ec_seckey_verify(cx: *const Context, sk: *const c_uchar) -> c_int;

    pub fn secp256k1_ec_pubkey_create(cx: *const Context,
                                      pk: *mut PublicKey,
                                      sk: *const c_uchar)
                                      -> c_int;

    // TODO secp256k1_ec_privkey_export
    // TODO secp256k1_ec_privkey_import

    pub fn secp256k1_ec_privkey_tweak_add(cx: *const Context,
                                          sk: *mut c_uchar,
                                          tweak: *const c_uchar)
                                          -> c_int;

    pub fn secp256k1_ec_pubkey_tweak_add(cx: *const Context,
                                         pk: *mut PublicKey,
                                         tweak: *const c_uchar)
                                         -> c_int;

    pub fn secp256k1_ec_privkey_tweak_mul(cx: *const Context,
                                          sk: *mut c_uchar,
                                          tweak: *const c_uchar)
                                          -> c_int;

    pub fn secp256k1_ec_pubkey_tweak_mul(cx: *const Context,
                                         pk: *mut PublicKey,
                                         tweak: *const c_uchar)
                                         -> c_int;

    pub fn secp256k1_ec_pubkey_combine(cx: *const Context,
                                       out: *mut PublicKey,
                                       ins: *const *const PublicKey,
                                       n: c_int)
                                       -> c_int;

    pub fn secp256k1_ecdh(cx: *const Context,
                          out: *mut SharedSecret,
                          point: *const PublicKey,
                          scalar: *const c_uchar)
                          -> c_int;

    // Generates a pedersen commitment: *commit = blind * G + value * G2.
    // The commitment is 33 bytes, the blinding factor is 32 bytes.
    pub fn secp256k1_pedersen_commit(ctx: *const Context,
                                     commit: *mut c_uchar,
                                     blind: *const c_uchar,
                                     value: uint64_t)
                                     -> c_int;

    // Takes a list of n pointers to 32 byte blinding values, the first negs
    // of which are treated with positive sign and the rest negative, then
    // calculates an additional blinding value that adds to zero.
    pub fn secp256k1_pedersen_blind_sum(ctx: *const Context,
                                        blind_out: *const c_uchar,
                                        blinds: *const *const c_uchar,
                                        n: c_int,
                                        npositive: c_int)
                                        -> c_int;

    // Takes two list of 33-byte commitments and sums the first set, subtracts
    // the second and returns the resulting commitment.
    pub fn secp256k1_pedersen_commit_sum(ctx: *const Context,
                                         commit_out: *const c_uchar,
                                         commits: *const *const c_uchar,
                                         pcnt: c_int,
                                         ncommits: *const *const c_uchar,
                                         ncnt: c_int)
                                         -> c_int;

    // Takes two list of 33-byte commitments and sums the first set and
    // subtracts the second and verifies that they sum to excess.
    pub fn secp256k1_pedersen_verify_tally(ctx: *const Context,
                                           commits: *const *const c_uchar,
                                           pcnt: c_int,
                                           ncommits: *const *const c_uchar,
                                           ncnt: c_int,
                                           excess: int64_t)
                                           -> c_int;

    pub fn secp256k1_rangeproof_info(ctx: *const Context,
                                     exp: *mut c_int,
                                     mantissa: *mut c_int,
                                     min_value: *mut uint64_t,
                                     max_value: *mut uint64_t,
                                     proof: *const c_uchar,
                                     plen: c_int)
                                     -> c_int;

    pub fn secp256k1_rangeproof_rewind(ctx: *const Context,
                                       blind_out: *mut c_uchar,
                                       value_out: *mut uint64_t,
                                       message_out: *mut c_uchar,
                                       outlen: *mut c_int,
                                       nonce: *const c_uchar,
                                       min_value: *mut uint64_t,
                                       max_value: *mut uint64_t,
                                       commit: *const c_uchar,
                                       proof: *const c_uchar,
                                       plen: c_int)
                                       -> c_int;

    pub fn secp256k1_rangeproof_verify(ctx: *const Context,
                                       min_value: &mut uint64_t,
                                       max_value: &mut uint64_t,
                                       commit: *const c_uchar,
                                       proof: *const c_uchar,
                                       plen: c_int)
                                       -> c_int;

    pub fn secp256k1_rangeproof_sign(ctx: *const Context,
                                     proof: *mut c_uchar,
                                     plen: *mut c_int,
                                     min_value: uint64_t,
                                     commit: *const c_uchar,
                                     blind: *const c_uchar,
                                     nonce: *const c_uchar,
                                     exp: c_int,
                                     min_bits: c_int,
                                     value: uint64_t)
                                     -> c_int;
}
