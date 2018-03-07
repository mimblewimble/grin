# Grin Style Guide

Grin uses [rustfmt](https://github.com/rust-lang-nursery/rustfmt) to maintain consistent formatting.

## Install rustfmt (nightly)

Note: we assume Rust has been installed via [Rustup](https://www.rustup.rs/).
See [build docs](./build.md) for more info.

```
rustup component add rustfmt-preview
rustup update
```

## Install git pre-commit hook

There is a basic git [pre-commit](../.hooks/pre-commit) hook in the repo.

The pre-commit hook will not prevent commits if style issues are present but it will
indicate any files that need formatting.

To enable this, create a symlink in `.git/hooks` (note the relative path) -

```
cd .git/hooks
ln -s -f ../../.hooks/pre-commit
```

## Running rustfmt

To run rustfmt against a single file, this __new__ command works with latest rust and after having done `rustup component add rustfmt-preview` and by setting --write-mode it doesn't overwrite files.

First maybe try a dry-run to see what changes would be made:
`rustfmt --write-mode diff -- client.rs`

Then if you don't want to do any other cleanups manually, make rustfmt make the changes 

`rustfmt -- client.rs`

and add that as a separate commit at the end of your Pull Request.


The old method would typically change formatting in _nearly every file_ in the grin repo. If you feel adventurous, try this:

`cargo +nightly fmt -- ./core/src/lib.rs`

(and please take care, since the ending `-- file/names.rs` actually doesn't have any effect)
