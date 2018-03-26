# Grin - Build, Configuration, and Running

## Supported Platforms

Longer term, most platforms will likely be supported to some extent.
Grin's programming language `rust` has build targets for most platforms.

What's working so far?
* Linux x86_64 and MacOS [grin + mining + development]
* Not Windows 10 yet [grin kind-of builds. No mining yet. Help wanted!]


## Requirements

But basically:
- rust 1.21+ (use [rustup]((https://www.rustup.rs/))- i.e. `curl https://sh.rustup.rs -sSf | sh; source $HOME/.cargo/env`)
- cmake 3.2+ (for [Cuckoo mining plugins]((https://github.com/mimblewimble/cuckoo-miner)))
- rocksdb + libs for compiling rocksdb:
  - clang (clanglib or clang-devel or libclang-dev)
  - llvm (Fedora llvm-devel, Debian llvm-dev)
- ncurses and libs (ncurses, ncurses5w)
- linux-headers (reported needed on Alpine linux)


## Build steps

```sh
git clone https://github.com/mimblewimble/grin.git
cd grin
cargo build
```


### Cross-platform builds

Rust (cargo) can build grin for many platforms, so in theory running `grin`
as a validating node on your low powered device might be possible.
To cross-compile `grin` on a x86 Linux platform and produce ARM binaries,
say, for a Raspberry Pi.


### Building the Cuckoo-Miner plugins

Building `grin_pow` might fail if you're not on a x86_64 system,
because that crate also builds external Cuckoo mining plugins.

To avoid building mining plugins, ensure your `pow/Cargo.toml' has a line

```
features=["no-plugin-build"]
```

and that it's not commented out.

### Build errors

See [Troubleshooting](https://github.com/mimblewimble/docs/wiki/Troubleshooting)

## What was built?

A successful build gets you:

 - `target/debug/grin` - the main grin binary
 - `target/debug/plugins/*` - mining plugins (optional)

Grin is still sensitive to the directory from which it's run. Make sure you
always run it within a directory that contains a `grin.toml` configuration and
stay consistent as to where it's run from.

With the included `grin.toml` unchanged, if you execute `cargo run` you get a
`.grin` subfolder that grin starts filling up with blockchain data.

While testing, put the grin binary on your path like this:

```
export PATH=/path/to/grin/dir/target/debug:$PATH
```

You can then run `grin` directly (try `grin help` for more options).

*Important Note*: if you used Grin in testnet1, running the wallet listener
manually isn't requred anymore. Grin will create a seed file and run the
listener automatically on start.

# Configuration

Grin attempts to run with sensible defaults, and can be further configured via
the `grin.toml` file. You should always ensure that this file is available to grin.
The supplied `grin.toml` contains inline documentation on all configuration
options, and should be the first point of reference for all options.

The `grin.toml` file can placed in one of several locations, using the first one it finds:

1. The current working directory
2. In the directory that holds the grin executable
3. {USER_HOME}/.grin

While it's recommended that you perform all grin server configuration via
`grin.toml`, it's also possible to supply command line switches to grin that
override any settings in the `grin.toml` file. 

For help on grin commands and their switches, try:

```
grin help
grin wallet help
grin client help
```

# Using grin

The wiki page [How to use grin](https://github.com/mimblewimble/docs/wiki/How-to-use-grin)
and linked pages have more information on what features we have,
troubleshooting, etc.

