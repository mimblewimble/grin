# Fuzz testing

## Installation
You have to use Rust nightly at the moment.
Cargo-fuzz (https://github.com/rust-fuzz/cargo-fuzz) has been used. 
To install it:

```
cargo install cargo-fuzz
```

## Pattern generation for corpus
This step is optional, libFuzz will generate random patterns to populate
corpus (in folder `corpus`). However we can genearete more meaningful pattern 
e.g. use serialized form of a real block or transaction. To generate them:

```
cd fuzz

cargo run --bin  gen-corpus
```

## Run tests
Fuzz test is basically infinite test, run it for some period of time then
stop if no failures are found.
To run the tests make sure youre in folder `core` otherwise you may get 
some misleading errors, then run one of the following tests:

```
cargo fuzz run transaction_read

cargo fuzz run block_read

cargo fuzz run compact_block_read

```

Run
```
cargo fuzz list
```
or check `fuzz/Cargo.toml` for the full list of targets.
