# grin code structure
grin is built in [rust](https://www.rust-lang.org/), a memory safe, compiled language. Performance critical parts like the Cuckoo mining algorithm are built as plugins, making it easy to swap between algorithm implementations for various hardware. Grin comes with CPU and experimental GPU support.

## Files in project root
List of files tracked in `git` and some files you'll create when you use grin.
- [CODE_OF_CONDUCT](../CODE_OF_CONDUCT.md) - How to behave if you want to participate. Taken from rust. Slightly modified.
- [CONTRIBUTING](../CONTRIBUTING.md) - How to help and become part of grin.
- [Cargo.toml](../Cargo.toml) and Cargo.lock (locally created, _not_ in git) - defines how to the project code is to be compiled and built
- [LICENSE](../LICENSE) - Apache 2.0 license
- [README](../README.md) - The first document you should read, with pointers to find more detail.
- [rustfmt.toml](../rustfmt.toml) - configuration file for rustfmt. Required before contributing _new_ code.

## Folder structure
List of folders in the grin git repo, and the `wallet` and `server` folders which you're [recommended to create yourself](build.md#running-a-node).
- api
  Code for ApiEndpoints accessible over REST.
- chain
  The blockchain implementation. Accepts a block (see pipe.rs) and adds it to the chain, or reject it.
- config
  Code for handling configuration.
- core
  All core types: Hash, Block, Input, Output, and how to serialize them. Core mining algorith, and more.
- doc
  All documentation.
- grin
  Code for the `grin` binary. Many parts (adapters, lib, miner, seed, server, sync, types) that the `grin` binary needs.
- keychain
  Code for working safely with keys and doing blinding.
- p2p
  All peer to peer connection and protocol-related logic (handshake, block propagation, etc.).
- pool
  Code for the transaction pool implementation.
- pow
  The Proof-of-Work algorithm. Testnet1 uses algo Cuckoo16. Mainnet uses Cuckoo32, the best known choice for GPU mining on 4GB cards.
- server
  A folder you're [supposed to create](build.md#running-a-node), before starting your server: cd to project root; mkdir server; cd server; grin server start (or run) and it will create a subfolder .grin
  - .grin
    - chain - a Rocksdb with the blockchain blocks and related information
    - peers - a Rocksdb with the list of Grin peers you're connected to
    - sumtrees - containts folders kernel, rangeproof and utxo that each have a pmmr_dat.bin
- src
  Code for the `grin` binary.
- store
  Data store - a thin wrapper for Rocksdb, a key-value database forked from LevelDB.
- target
  Where the grin binary ends up, after the compile and build process finishes. In case of trouble, see [troubleshooting](FAQ.md#troubleshooting)
- util
  Low-level rust utilities.
- wallet
  A folder you're [supposed to create](build.md#running-a-node), before creating your wallet: cd to project root; mkdir wallet; cd wallet; grin wallet init
  - wallet.dat - your "outputs", that once confirmed and matured, can be spent with the [`grin wallet send`](wallet.md) command. (locally created, _not_ in git)
  - wallet.seed - your secret wallet seed. (locally created, _not_ in git)

## grin dependencies
- [secp256k1](https://github.com/mimblewimble/rust-secp256k1-zkp)
  Integration and rust bindings for libsecp256l1, and some changes waiting to be upstreamed. Imported in util/Cargo.toml.
