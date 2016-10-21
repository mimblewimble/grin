[![Build Status](https://travis-ci.org/apoelstra/rust-secp256k1.png?branch=master)](https://travis-ci.org/apoelstra/rust-secp256k1)

### rust-secp256k1

`rust-secp256k1` is a wrapper around ![libsecp256k1](https://github.com/bitcoin/secp256k1),
a C library by Peter Wuille for producing ECDSA signatures using the SECG curve
`secp256k1`. This library
* exposes type-safe Rust bindings for all `libsecp256k1` functions
* implements key generation
* implements deterministic nonce generation via RFC6979
* implements many unit tests, adding to those already present in `libsecp256k1`
* makes no allocations (except in unit tests) for efficiency and use in freestanding implementations

[Full documentation](https://www.wpsoftware.net/rustdoc/secp256k1/)

