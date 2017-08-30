# Grin Style Guide

Grin uses [rustfmt](https://github.com/rust-lang-nursery/rustfmt) to maintain consist formatting.

## Install rustfmt (nightly)

Note: we assume Rust has been installed via [Rustup](https://www.rustup.rs/).
See [build docs](./build.md) for more info.

rustfmt itself requires the nightly toolchain -

```
rustup update
rustup install nightly
rustup run nightly cargo install rustfmt-nightly
```

## Install git pre-commit hook

There is a basic git [pre-commit](../.hooks/pre-commit) hook in the repo.

The pre-commit hook will not prevent commits if style issues are present but it will
indicate any files that need formatting.

To enable this create a symlink in `.git/hooks` (note the relative path) -

```
cd .git/hooks
ln -s -f ../../.hooks/pre-commit
```

## Running rustfmt

To run rustfmt against a single file in grin -

```
cargo +nightly fmt -- ./core/src/lib.rs
```
