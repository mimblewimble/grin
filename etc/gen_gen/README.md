# Genesis Genesis

N.B: This crate's `Cargo.toml` file has been disabled by renaming it to `_Cargo.toml`. It no longer builds due to changes in the project structure.

This crate isn't strictly part of grin but allows the generation and release of a new Grin Genesis in an automated fashion. The process is the following:

* Prepare a multisig output and kernel to use as coinbase. In the case of Grin mainnet, this is done and owned by the council treasurers. This can be down a few days prior.
* Grab the latest bitcoin block hash from publicly available APIs.
* Build a genesis block with the prepared coinbase and the bitcoin block hash as `prev_root`. The timestamp of the block is set to 30 min ahead to leave enough time to run a build and start a node.
* Mine the block so we have at least a valid Cuckatoo Cycle. We don't require a given difficulty for that solution.
* Finalize the block with the proof-of-work, setting a high-enough difficulty (to be agreed upon separately).
* Commit the block information to github.
* Tag version 1.0.0 (scary).

N.B. This was written while listening to Genesis. Unfortunately, I'm not rich enough to do it while driving a Genesis. And that'd be dangerous.

# Usage

1. Build this crate.
2. From its root run `./target/release/gen-gen --coinbase <file> --difficulty <u64> --tag <version>`
