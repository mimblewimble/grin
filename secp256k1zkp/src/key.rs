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

//! # Public and secret keys

use std::intrinsics::copy_nonoverlapping;
use std::marker;
use arrayvec::ArrayVec;
use rand::Rng;
use serialize::{Decoder, Decodable, Encoder, Encodable};
use serde::{Serialize, Deserialize, Serializer, Deserializer};

use super::{Secp256k1, ContextFlag};
use super::Error::{self, IncapableContext, InvalidPublicKey, InvalidSecretKey};
use constants;
use ffi;

/// Secret 256-bit key used as `x` in an ECDSA signature
pub struct SecretKey([u8; constants::SECRET_KEY_SIZE]);
impl_array_newtype!(SecretKey, u8, constants::SECRET_KEY_SIZE);
impl_pretty_debug!(SecretKey);


/// The number 1 encoded as a secret key
/// Deprecated; `static` is not what I want; use `ONE_KEY` instead
pub static ONE: SecretKey = SecretKey([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                                       0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);

/// The number 0 encoded as a secret key
pub const ZERO_KEY: SecretKey = SecretKey([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                                           0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);

/// The number 1 encoded as a secret key
pub const ONE_KEY: SecretKey = SecretKey([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                                          0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);

/// A Secp256k1 public key, used for verification of signatures
#[derive(Copy, Clone, PartialEq, Eq, Debug, Hash)]
pub struct PublicKey(ffi::PublicKey);


fn random_32_bytes<R: Rng>(rng: &mut R) -> [u8; 32] {
    let mut ret = [0u8; 32];
    rng.fill_bytes(&mut ret);
    ret
}

impl SecretKey {
    /// Creates a new random secret key
    #[inline]
    pub fn new<R: Rng>(secp: &Secp256k1, rng: &mut R) -> SecretKey {
        let mut data = random_32_bytes(rng);
        unsafe {
            while ffi::secp256k1_ec_seckey_verify(secp.ctx, data.as_ptr()) == 0 {
                data = random_32_bytes(rng);
            }
        }
        SecretKey(data)
    }

    /// Converts a `SECRET_KEY_SIZE`-byte slice to a secret key
    #[inline]
    pub fn from_slice(secp: &Secp256k1, data: &[u8]) -> Result<SecretKey, Error> {
        match data.len() {
            constants::SECRET_KEY_SIZE => {
                let mut ret = [0; constants::SECRET_KEY_SIZE];
                unsafe {
                    if ffi::secp256k1_ec_seckey_verify(secp.ctx, data.as_ptr()) == 0 {
                        return Err(InvalidSecretKey);
                    }
                    copy_nonoverlapping(data.as_ptr(), ret.as_mut_ptr(), data.len());
                }
                Ok(SecretKey(ret))
            }
            _ => Err(InvalidSecretKey),
        }
    }

    #[inline]
    /// Adds one secret key to another, modulo the curve order
    pub fn add_assign(&mut self, secp: &Secp256k1, other: &SecretKey) -> Result<(), Error> {
        unsafe {
            if ffi::secp256k1_ec_privkey_tweak_add(secp.ctx, self.as_mut_ptr(), other.as_ptr()) !=
               1 {
                Err(InvalidSecretKey)
            } else {
                Ok(())
            }
        }
    }

    #[inline]
    /// Multiplies one secret key by another, modulo the curve order
    pub fn mul_assign(&mut self, secp: &Secp256k1, other: &SecretKey) -> Result<(), Error> {
        unsafe {
            if ffi::secp256k1_ec_privkey_tweak_mul(secp.ctx, self.as_mut_ptr(), other.as_ptr()) !=
               1 {
                Err(InvalidSecretKey)
            } else {
                Ok(())
            }
        }
    }
}

impl PublicKey {
    /// Creates a new zeroed out public key
    #[inline]
    pub fn new() -> PublicKey {
        PublicKey(ffi::PublicKey::new())
    }

    /// Determines whether a pubkey is valid
    #[inline]
    pub fn is_valid(&self) -> bool {
        // The only invalid pubkey the API should be able to create is
        // the zero one.
        self.0[..].iter().any(|&x| x != 0)
    }

    /// Obtains a raw pointer suitable for use with FFI functions
    #[inline]
    pub fn as_ptr(&self) -> *const ffi::PublicKey {
        &self.0 as *const _
    }

    /// Creates a new public key from a secret key.
    #[inline]
    pub fn from_secret_key(secp: &Secp256k1, sk: &SecretKey) -> Result<PublicKey, Error> {
        if secp.caps == ContextFlag::VerifyOnly || secp.caps == ContextFlag::None {
            return Err(IncapableContext);
        }
        let mut pk = unsafe { ffi::PublicKey::blank() };
        unsafe {
            // We can assume the return value because it's not possible to construct
            // an invalid `SecretKey` without transmute trickery or something
            let res = ffi::secp256k1_ec_pubkey_create(secp.ctx, &mut pk, sk.as_ptr());
            debug_assert_eq!(res, 1);
        }
        Ok(PublicKey(pk))
    }

    /// Creates a public key directly from a slice
    #[inline]
    pub fn from_slice(secp: &Secp256k1, data: &[u8]) -> Result<PublicKey, Error> {

        let mut pk = unsafe { ffi::PublicKey::blank() };
        unsafe {
            if ffi::secp256k1_ec_pubkey_parse(secp.ctx,
                                              &mut pk,
                                              data.as_ptr(),
                                              data.len() as ::libc::size_t) == 1 {
                Ok(PublicKey(pk))
            } else {
                Err(InvalidPublicKey)
            }
        }
    }

    #[inline]
    /// Serialize the key as a byte-encoded pair of values. In compressed form
    /// the y-coordinate is represented by only a single bit, as x determines
    /// it up to one bit.
    pub fn serialize_vec(&self,
                         secp: &Secp256k1,
                         compressed: bool)
                         -> ArrayVec<[u8; constants::PUBLIC_KEY_SIZE]> {
        let mut ret = ArrayVec::new();

        unsafe {
            let mut ret_len = constants::PUBLIC_KEY_SIZE as ::libc::size_t;
            let compressed = if compressed {
                ffi::SECP256K1_SER_COMPRESSED
            } else {
                ffi::SECP256K1_SER_UNCOMPRESSED
            };
            let err = ffi::secp256k1_ec_pubkey_serialize(secp.ctx,
                                                         ret.as_ptr(),
                                                         &mut ret_len,
                                                         self.as_ptr(),
                                                         compressed);
            debug_assert_eq!(err, 1);
            ret.set_len(ret_len as usize);
        }
        ret
    }

    #[inline]
    /// Adds the pk corresponding to `other` to the pk `self` in place
    pub fn add_exp_assign(&mut self, secp: &Secp256k1, other: &SecretKey) -> Result<(), Error> {
        if secp.caps == ContextFlag::SignOnly || secp.caps == ContextFlag::None {
            return Err(IncapableContext);
        }
        unsafe {
            if ffi::secp256k1_ec_pubkey_tweak_add(secp.ctx,
                                                  &mut self.0 as *mut _,
                                                  other.as_ptr()) == 1 {
                Ok(())
            } else {
                Err(InvalidSecretKey)
            }
        }
    }

    #[inline]
    /// Muliplies the pk `self` in place by the scalar `other`
    pub fn mul_assign(&mut self, secp: &Secp256k1, other: &SecretKey) -> Result<(), Error> {
        if secp.caps == ContextFlag::SignOnly || secp.caps == ContextFlag::None {
            return Err(IncapableContext);
        }
        unsafe {
            if ffi::secp256k1_ec_pubkey_tweak_mul(secp.ctx,
                                                  &mut self.0 as *mut _,
                                                  other.as_ptr()) == 1 {
                Ok(())
            } else {
                Err(InvalidSecretKey)
            }
        }
    }
}

impl Decodable for PublicKey {
    fn decode<D: Decoder>(d: &mut D) -> Result<PublicKey, D::Error> {
        d.read_seq(|d, len| {
            let s = Secp256k1::with_caps(::ContextFlag::None);
            if len == constants::UNCOMPRESSED_PUBLIC_KEY_SIZE {
                unsafe {
                    use std::mem;
                    let mut ret: [u8; constants::UNCOMPRESSED_PUBLIC_KEY_SIZE] =
                        mem::uninitialized();
                    for i in 0..len {
                        ret[i] = try!(d.read_seq_elt(i, |d| Decodable::decode(d)));
                    }
                    PublicKey::from_slice(&s, &ret).map_err(|_| d.error("invalid public key"))
                }
            } else if len == constants::COMPRESSED_PUBLIC_KEY_SIZE {
                unsafe {
                    use std::mem;
                    let mut ret: [u8; constants::COMPRESSED_PUBLIC_KEY_SIZE] = mem::uninitialized();
                    for i in 0..len {
                        ret[i] = try!(d.read_seq_elt(i, |d| Decodable::decode(d)));
                    }
                    PublicKey::from_slice(&s, &ret).map_err(|_| d.error("invalid public key"))
                }
            } else {
                Err(d.error("Invalid length"))
            }
        })
    }
}

/// Creates a new public key from a FFI public key
impl From<ffi::PublicKey> for PublicKey {
    #[inline]
    fn from(pk: ffi::PublicKey) -> PublicKey {
        PublicKey(pk)
    }
}


impl Encodable for PublicKey {
    fn encode<S: Encoder>(&self, s: &mut S) -> Result<(), S::Error> {
        let secp = Secp256k1::with_caps(::ContextFlag::None);
        self.serialize_vec(&secp, true).encode(s)
    }
}

impl Deserialize for PublicKey {
    fn deserialize<D>(d: &mut D) -> Result<PublicKey, D::Error>
        where D: Deserializer
    {
        use serde::de;
        struct Visitor {
            marker: marker::PhantomData<PublicKey>,
        }
        impl de::Visitor for Visitor {
            type Value = PublicKey;

            #[inline]
            fn visit_seq<V>(&mut self, mut v: V) -> Result<PublicKey, V::Error>
                where V: de::SeqVisitor
            {
                debug_assert!(constants::UNCOMPRESSED_PUBLIC_KEY_SIZE >=
                              constants::COMPRESSED_PUBLIC_KEY_SIZE);

                let s = Secp256k1::with_caps(::ContextFlag::None);
                unsafe {
                    use std::mem;
                    let mut ret: [u8; constants::UNCOMPRESSED_PUBLIC_KEY_SIZE] =
                        mem::uninitialized();

                    let mut read_len = 0;
                    while read_len < constants::UNCOMPRESSED_PUBLIC_KEY_SIZE {
                        let read_ch = match try!(v.visit()) {
                            Some(c) => c,
                            None => break,
                        };
                        ret[read_len] = read_ch;
                        read_len += 1;
                    }
                    try!(v.end());

                    PublicKey::from_slice(&s, &ret[..read_len])
                        .map_err(|e| de::Error::syntax(&e.to_string()))
                }
            }
        }

        // Begin actual function
        d.visit(Visitor { marker: ::std::marker::PhantomData })
    }
}

impl Serialize for PublicKey {
    fn serialize<S>(&self, s: &mut S) -> Result<(), S::Error>
        where S: Serializer
    {
        let secp = Secp256k1::with_caps(::ContextFlag::None);
        (&self.serialize_vec(&secp, true)[..]).serialize(s)
    }
}

#[cfg(test)]
mod test {
    use super::super::{Secp256k1, ContextFlag};
    use super::super::Error::{InvalidPublicKey, InvalidSecretKey, IncapableContext};
    use super::{PublicKey, SecretKey};
    use super::super::constants;

    use rand::{Rng, thread_rng};

    #[test]
    fn skey_from_slice() {
        let s = Secp256k1::new();
        let sk = SecretKey::from_slice(&s, &[1; 31]);
        assert_eq!(sk, Err(InvalidSecretKey));

        let sk = SecretKey::from_slice(&s, &[1; 32]);
        assert!(sk.is_ok());
    }

    #[test]
    fn pubkey_from_slice() {
        let s = Secp256k1::new();
        assert_eq!(PublicKey::from_slice(&s, &[]), Err(InvalidPublicKey));
        assert_eq!(PublicKey::from_slice(&s, &[1, 2, 3]), Err(InvalidPublicKey));

        let uncompressed =
            PublicKey::from_slice(&s,
                                  &[4, 54, 57, 149, 239, 162, 148, 175, 246, 254, 239, 75, 154,
                                    152, 10, 82, 234, 224, 85, 220, 40, 100, 57, 121, 30, 162,
                                    94, 156, 135, 67, 74, 49, 179, 57, 236, 53, 162, 124, 149,
                                    144, 168, 77, 74, 30, 72, 211, 229, 110, 111, 55, 96, 193,
                                    86, 227, 183, 152, 195, 155, 51, 247, 123, 113, 60, 228, 188]);
        assert!(uncompressed.is_ok());

        let compressed = PublicKey::from_slice(&s,
                                               &[3, 23, 183, 225, 206, 31, 159, 148, 195, 42, 67,
                                                 115, 146, 41, 248, 140, 11, 3, 51, 41, 111, 180,
                                                 110, 143, 114, 134, 88, 73, 198, 174, 52, 184,
                                                 78]);
        assert!(compressed.is_ok());
    }

    #[test]
    fn keypair_slice_round_trip() {
        let s = Secp256k1::new();

        let (sk1, pk1) = s.generate_keypair(&mut thread_rng()).unwrap();
        assert_eq!(SecretKey::from_slice(&s, &sk1[..]), Ok(sk1));
        assert_eq!(PublicKey::from_slice(&s, &pk1.serialize_vec(&s, true)[..]),
                   Ok(pk1));
        assert_eq!(PublicKey::from_slice(&s, &pk1.serialize_vec(&s, false)[..]),
                   Ok(pk1));
    }

    #[test]
    fn invalid_secret_key() {
        let s = Secp256k1::new();
        // Zero
        assert_eq!(SecretKey::from_slice(&s, &[0; 32]), Err(InvalidSecretKey));
        // -1
        assert_eq!(SecretKey::from_slice(&s, &[0xff; 32]),
                   Err(InvalidSecretKey));
        // Top of range
        assert!(SecretKey::from_slice(&s,
                                      &[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
                                        0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFE, 0xBA, 0xAE,
                                        0xDC, 0xE6, 0xAF, 0x48, 0xA0, 0x3B, 0xBF, 0xD2, 0x5E,
                                        0x8C, 0xD0, 0x36, 0x41, 0x40])
            .is_ok());
        // One past top of range
        assert!(SecretKey::from_slice(&s,
                                      &[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
                                        0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFE, 0xBA, 0xAE,
                                        0xDC, 0xE6, 0xAF, 0x48, 0xA0, 0x3B, 0xBF, 0xD2, 0x5E,
                                        0x8C, 0xD0, 0x36, 0x41, 0x41])
            .is_err());
    }

    #[test]
    fn test_pubkey_from_slice_bad_context() {
        let s = Secp256k1::without_caps();
        let sk = SecretKey::new(&s, &mut thread_rng());
        assert_eq!(PublicKey::from_secret_key(&s, &sk), Err(IncapableContext));

        let s = Secp256k1::with_caps(ContextFlag::VerifyOnly);
        assert_eq!(PublicKey::from_secret_key(&s, &sk), Err(IncapableContext));

        let s = Secp256k1::with_caps(ContextFlag::SignOnly);
        assert!(PublicKey::from_secret_key(&s, &sk).is_ok());

        let s = Secp256k1::with_caps(ContextFlag::Full);
        assert!(PublicKey::from_secret_key(&s, &sk).is_ok());
    }

    #[test]
    fn test_add_exp_bad_context() {
        let s = Secp256k1::with_caps(ContextFlag::Full);
        let (sk, mut pk) = s.generate_keypair(&mut thread_rng()).unwrap();

        assert!(pk.add_exp_assign(&s, &sk).is_ok());

        let s = Secp256k1::with_caps(ContextFlag::VerifyOnly);
        assert!(pk.add_exp_assign(&s, &sk).is_ok());

        let s = Secp256k1::with_caps(ContextFlag::SignOnly);
        assert_eq!(pk.add_exp_assign(&s, &sk), Err(IncapableContext));

        let s = Secp256k1::with_caps(ContextFlag::None);
        assert_eq!(pk.add_exp_assign(&s, &sk), Err(IncapableContext));
    }

    #[test]
    fn test_bad_deserialize() {
        use std::io::Cursor;
        use serialize::{json, Decodable};

        let zero31 = "[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]".as_bytes();
        let json31 = json::Json::from_reader(&mut Cursor::new(zero31)).unwrap();
        let zero32 = "[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]".as_bytes();
        let json32 = json::Json::from_reader(&mut Cursor::new(zero32)).unwrap();
        let zero65 = "[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,\
                      0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]"
            .as_bytes();
        let json65 = json::Json::from_reader(&mut Cursor::new(zero65)).unwrap();
        let string = "\"my key\"".as_bytes();
        let json = json::Json::from_reader(&mut Cursor::new(string)).unwrap();

        // Invalid length
        let mut decoder = json::Decoder::new(json31.clone());
        assert!(<PublicKey as Decodable>::decode(&mut decoder).is_err());
        let mut decoder = json::Decoder::new(json31.clone());
        assert!(<SecretKey as Decodable>::decode(&mut decoder).is_err());
        let mut decoder = json::Decoder::new(json32.clone());
        assert!(<PublicKey as Decodable>::decode(&mut decoder).is_err());
        let mut decoder = json::Decoder::new(json32.clone());
        assert!(<SecretKey as Decodable>::decode(&mut decoder).is_ok());
        let mut decoder = json::Decoder::new(json65.clone());
        assert!(<PublicKey as Decodable>::decode(&mut decoder).is_err());
        let mut decoder = json::Decoder::new(json65.clone());
        assert!(<SecretKey as Decodable>::decode(&mut decoder).is_err());

        // Syntax error
        let mut decoder = json::Decoder::new(json.clone());
        assert!(<PublicKey as Decodable>::decode(&mut decoder).is_err());
        let mut decoder = json::Decoder::new(json.clone());
        assert!(<SecretKey as Decodable>::decode(&mut decoder).is_err());
    }

    #[test]
    fn test_serialize() {
        use std::io::Cursor;
        use serialize::{json, Decodable, Encodable};

        macro_rules! round_trip (
            ($var:ident) => ({
                let start = $var;
                let mut encoded = String::new();
                {
                    let mut encoder = json::Encoder::new(&mut encoded);
                    start.encode(&mut encoder).unwrap();
                }
                let json = json::Json::from_reader(&mut Cursor::new(encoded.as_bytes())).unwrap();
                let mut decoder = json::Decoder::new(json);
                let decoded = Decodable::decode(&mut decoder);
                assert_eq!(Ok(Some(start)), decoded);
            })
        );

        let s = Secp256k1::new();
        for _ in 0..500 {
            let (sk, pk) = s.generate_keypair(&mut thread_rng()).unwrap();
            round_trip!(sk);
            round_trip!(pk);
        }
    }

    #[test]
    fn test_bad_serde_deserialize() {
        use serde::Deserialize;
        use json;

        // Invalid length
        let zero31 = "[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]".as_bytes();
        let mut json = json::de::Deserializer::new(zero31.iter().map(|c| Ok(*c)));
        assert!(<PublicKey as Deserialize>::deserialize(&mut json).is_err());
        let mut json = json::de::Deserializer::new(zero31.iter().map(|c| Ok(*c)));
        assert!(<SecretKey as Deserialize>::deserialize(&mut json).is_err());

        let zero32 = "[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]".as_bytes();
        let mut json = json::de::Deserializer::new(zero32.iter().map(|c| Ok(*c)));
        assert!(<PublicKey as Deserialize>::deserialize(&mut json).is_err());
        let mut json = json::de::Deserializer::new(zero32.iter().map(|c| Ok(*c)));
        assert!(<SecretKey as Deserialize>::deserialize(&mut json).is_ok());

        // All zeroes pk is invalid
        let zero65 = "[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,\
                      0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]"
            .as_bytes();
        let mut json = json::de::Deserializer::new(zero65.iter().map(|c| Ok(*c)));
        assert!(<PublicKey as Deserialize>::deserialize(&mut json).is_err());
        let mut json = json::de::Deserializer::new(zero65.iter().map(|c| Ok(*c)));
        assert!(<SecretKey as Deserialize>::deserialize(&mut json).is_err());

        // Syntax error
        let string = "\"my key\"".as_bytes();
        let mut json = json::de::Deserializer::new(string.iter().map(|c| Ok(*c)));
        assert!(<PublicKey as Deserialize>::deserialize(&mut json).is_err());
        let mut json = json::de::Deserializer::new(string.iter().map(|c| Ok(*c)));
        assert!(<SecretKey as Deserialize>::deserialize(&mut json).is_err());
    }


    #[test]
    fn test_serialize_serde() {
        use serde::{Serialize, Deserialize};
        use json;

        macro_rules! round_trip (
            ($var:ident) => ({
                let start = $var;
                let mut encoded = Vec::new();
                {
                    let mut serializer = json::ser::Serializer::new(&mut encoded);
                    start.serialize(&mut serializer).unwrap();
                }
                let mut deserializer = json::de::Deserializer::new(encoded.iter().map(|c| Ok(*c)));
                let decoded = Deserialize::deserialize(&mut deserializer);
                assert_eq!(Some(start), decoded.ok());
            })
        );

        let s = Secp256k1::new();
        for _ in 0..500 {
            let (sk, pk) = s.generate_keypair(&mut thread_rng()).unwrap();
            round_trip!(sk);
            round_trip!(pk);
        }
    }

    #[test]
    fn test_out_of_range() {

        struct BadRng(u8);
        impl Rng for BadRng {
            fn next_u32(&mut self) -> u32 {
                unimplemented!()
            }
            // This will set a secret key to a little over the
            // group order, then decrement with repeated calls
            // until it returns a valid key
            fn fill_bytes(&mut self, data: &mut [u8]) {
                let group_order: [u8; 32] = [0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
                                             0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xfe, 0xba, 0xae,
                                             0xdc, 0xe6, 0xaf, 0x48, 0xa0, 0x3b, 0xbf, 0xd2, 0x5e,
                                             0x8c, 0xd0, 0x36, 0x41, 0x41];
                assert_eq!(data.len(), 32);
                unsafe {
                    use std::intrinsics::copy_nonoverlapping;
                    copy_nonoverlapping(group_order.as_ptr(), data.as_mut_ptr(), 32);
                }
                data[31] = self.0;
                self.0 -= 1;
            }
        }

        let s = Secp256k1::new();
        s.generate_keypair(&mut BadRng(0xff)).unwrap();
    }

    #[test]
    fn test_pubkey_from_bad_slice() {
        let s = Secp256k1::new();
        // Bad sizes
        assert_eq!(PublicKey::from_slice(&s, &[0; constants::COMPRESSED_PUBLIC_KEY_SIZE - 1]),
                   Err(InvalidPublicKey));
        assert_eq!(PublicKey::from_slice(&s, &[0; constants::COMPRESSED_PUBLIC_KEY_SIZE + 1]),
                   Err(InvalidPublicKey));
        assert_eq!(PublicKey::from_slice(&s, &[0; constants::UNCOMPRESSED_PUBLIC_KEY_SIZE - 1]),
                   Err(InvalidPublicKey));
        assert_eq!(PublicKey::from_slice(&s, &[0; constants::UNCOMPRESSED_PUBLIC_KEY_SIZE + 1]),
                   Err(InvalidPublicKey));

        // Bad parse
        assert_eq!(PublicKey::from_slice(&s, &[0xff; constants::UNCOMPRESSED_PUBLIC_KEY_SIZE]),
                   Err(InvalidPublicKey));
        assert_eq!(PublicKey::from_slice(&s, &[0x55; constants::COMPRESSED_PUBLIC_KEY_SIZE]),
                   Err(InvalidPublicKey));
    }

    #[test]
    fn test_debug_output() {
        struct DumbRng(u32);
        impl Rng for DumbRng {
            fn next_u32(&mut self) -> u32 {
                self.0 = self.0.wrapping_add(1);
                self.0
            }
        }

        let s = Secp256k1::new();
        let (sk, _) = s.generate_keypair(&mut DumbRng(0)).unwrap();

        assert_eq!(&format!("{:?}", sk),
                   "SecretKey(0200000001000000040000000300000006000000050000000800000007000000)");
    }

    #[test]
    fn test_pubkey_serialize() {
        struct DumbRng(u32);
        impl Rng for DumbRng {
            fn next_u32(&mut self) -> u32 {
                self.0 = self.0.wrapping_add(1);
                self.0
            }
        }

        let s = Secp256k1::new();
        let (_, pk1) = s.generate_keypair(&mut DumbRng(0)).unwrap();
        assert_eq!(&pk1.serialize_vec(&s, false)[..],
                   &[4, 149, 16, 196, 140, 38, 92, 239, 179, 65, 59, 224, 230, 183, 91, 238, 240, 46, 186, 252, 175, 102, 52, 249, 98, 178, 123, 72, 50, 171, 196, 254, 236, 1, 189, 143, 242, 227, 16, 87, 247, 183, 162, 68, 237, 140, 92, 205, 151, 129, 166, 58, 111, 96, 123, 64, 180, 147, 51, 12, 209, 89, 236, 213, 206][..]);
        assert_eq!(&pk1.serialize_vec(&s, true)[..],
                   &[2, 149, 16, 196, 140, 38, 92, 239, 179, 65, 59, 224, 230, 183, 91, 238, 240, 46, 186, 252, 175, 102, 52, 249, 98, 178, 123, 72, 50, 171, 196, 254, 236][..]);
    }

    #[test]
    fn test_addition() {
        let s = Secp256k1::new();

        let (mut sk1, mut pk1) = s.generate_keypair(&mut thread_rng()).unwrap();
        let (mut sk2, mut pk2) = s.generate_keypair(&mut thread_rng()).unwrap();

        assert_eq!(PublicKey::from_secret_key(&s, &sk1).unwrap(), pk1);
        assert!(sk1.add_assign(&s, &sk2).is_ok());
        assert!(pk1.add_exp_assign(&s, &sk2).is_ok());
        assert_eq!(PublicKey::from_secret_key(&s, &sk1).unwrap(), pk1);

        assert_eq!(PublicKey::from_secret_key(&s, &sk2).unwrap(), pk2);
        assert!(sk2.add_assign(&s, &sk1).is_ok());
        assert!(pk2.add_exp_assign(&s, &sk1).is_ok());
        assert_eq!(PublicKey::from_secret_key(&s, &sk2).unwrap(), pk2);
    }

    #[test]
    fn test_multiplication() {
        let s = Secp256k1::new();

        let (mut sk1, mut pk1) = s.generate_keypair(&mut thread_rng()).unwrap();
        let (mut sk2, mut pk2) = s.generate_keypair(&mut thread_rng()).unwrap();

        assert_eq!(PublicKey::from_secret_key(&s, &sk1).unwrap(), pk1);
        assert!(sk1.mul_assign(&s, &sk2).is_ok());
        assert!(pk1.mul_assign(&s, &sk2).is_ok());
        assert_eq!(PublicKey::from_secret_key(&s, &sk1).unwrap(), pk1);

        assert_eq!(PublicKey::from_secret_key(&s, &sk2).unwrap(), pk2);
        assert!(sk2.mul_assign(&s, &sk1).is_ok());
        assert!(pk2.mul_assign(&s, &sk1).is_ok());
        assert_eq!(PublicKey::from_secret_key(&s, &sk2).unwrap(), pk2);
    }

    #[test]
    fn pubkey_hash() {
        use std::hash::{Hash, SipHasher, Hasher};
        use std::collections::HashSet;

        fn hash<T: Hash>(t: &T) -> u64 {
            let mut s = SipHasher::new();
            t.hash(&mut s);
            s.finish()
        }

        let s = Secp256k1::new();
        let mut set = HashSet::new();
        const COUNT: usize = 1024;
        let count = (0..COUNT)
            .map(|_| {
                let (_, pk) = s.generate_keypair(&mut thread_rng()).unwrap();
                let hash = hash(&pk);
                assert!(!set.contains(&hash));
                set.insert(hash);
            })
            .count();
        assert_eq!(count, COUNT);
    }
}
