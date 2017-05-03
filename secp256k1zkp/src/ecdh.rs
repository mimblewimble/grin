// Bitcoin secp256k1 bindings
// Written in 2015 by
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

//! # ECDH
//! Support for shared secret computations
//!
use std::ops;

use super::Secp256k1;
use key::{SecretKey, PublicKey};
use ffi;

/// A tag used for recovering the public key from a compact signature
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct SharedSecret(ffi::SharedSecret);

impl SharedSecret {
    /// Creates a new shared secret from a pubkey and secret key
    #[inline]
    pub fn new(secp: &Secp256k1, point: &PublicKey, scalar: &SecretKey) -> SharedSecret {
        unsafe {
            let mut ss = ffi::SharedSecret::blank();
            let res = ffi::secp256k1_ecdh(secp.ctx, &mut ss, point.as_ptr(), scalar.as_ptr());
            debug_assert_eq!(res, 1);
            SharedSecret(ss)
        }
    }

    /// Obtains a raw pointer suitable for use with FFI functions
    #[inline]
    pub fn as_ptr(&self) -> *const ffi::SharedSecret {
        &self.0 as *const _
    }
}

/// Creates a new shared secret from a FFI shared secret
impl From<ffi::SharedSecret> for SharedSecret {
    #[inline]
    fn from(ss: ffi::SharedSecret) -> SharedSecret {
        SharedSecret(ss)
    }
}


impl ops::Index<usize> for SharedSecret {
    type Output = u8;

    #[inline]
    fn index(&self, index: usize) -> &u8 {
        &self.0[index]
    }
}

impl ops::Index<ops::Range<usize>> for SharedSecret {
    type Output = [u8];

    #[inline]
    fn index(&self, index: ops::Range<usize>) -> &[u8] {
        &self.0[index]
    }
}

impl ops::Index<ops::RangeFrom<usize>> for SharedSecret {
    type Output = [u8];

    #[inline]
    fn index(&self, index: ops::RangeFrom<usize>) -> &[u8] {
        &self.0[index.start..]
    }
}

impl ops::Index<ops::RangeFull> for SharedSecret {
    type Output = [u8];

    #[inline]
    fn index(&self, _: ops::RangeFull) -> &[u8] {
        &self.0[..]
    }
}

#[cfg(test)]
mod tests {
    use rand::thread_rng;
    use super::SharedSecret;
    use super::super::Secp256k1;

    #[test]
    fn ecdh() {
        let s = Secp256k1::with_caps(::ContextFlag::SignOnly);
        let (sk1, pk1) = s.generate_keypair(&mut thread_rng()).unwrap();
        let (sk2, pk2) = s.generate_keypair(&mut thread_rng()).unwrap();

        let sec1 = SharedSecret::new(&s, &pk1, &sk2);
        let sec2 = SharedSecret::new(&s, &pk2, &sk1);
        let sec_odd = SharedSecret::new(&s, &pk1, &sk1);
        assert_eq!(sec1, sec2);
        assert!(sec_odd != sec2);
    }
}

#[cfg(all(test, feature = "unstable"))]
mod benches {
    use rand::thread_rng;
    use test::{Bencher, black_box};

    use super::SharedSecret;
    use super::super::Secp256k1;

    #[bench]
    pub fn bench_ecdh(bh: &mut Bencher) {
        let s = Secp256k1::with_caps(::ContextFlag::SignOnly);
        let (sk, pk) = s.generate_keypair(&mut thread_rng()).unwrap();

        let s = Secp256k1::new();
        bh.iter(|| {
            let res = SharedSecret::new(&s, &pk, &sk);
            black_box(res);
        });
    }
}
