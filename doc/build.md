# Grin - Build, Configuration, and Running

## Supported Platforms

Longer term, most platforms will likely be supported to some extent.
Grin's programming language `rust` has build targets for most platforms.

What's working so far?
* Linux x86_64 and MacOS [grin + mining + development]
* Not Windows 10 yet [grin kind-of builds. No mining yet. Help wanted!]


## Requirements

See [Requirements on the wiki](https://github.com/mimblewimble/docs/wiki/Building).

But basically:
- rust 1.21+ (use [rustup]((https://www.rustup.rs/))- i.e. `curl https://sh.rustup.rs -sSf | sh; source $HOME/.cargo/env`)
- cmake 3.2+ (for [Cuckoo mining plugins]((https://github.com/mimblewimble/cuckoo-miner)))
- rocksdb + libs for compiling rocksdb:
  - clang (clanglib or clang-devel or libclang-dev)
  - llvm (Fedora llvm-devel, Debian llvm-dev)
- ncurses and libs (ncurses ncurses5w)
- linux-headers (reported needed on Alpine linux)


## Build steps

```sh
    git clone https://github.com/mimblewimble/grin.git
    cd grin
    # Decide yourself if you want master, another branch, or tag
    cargo update # first time, in case you have some old stuff
    cargo build
    # or: cargo install && cargo clean # if you don't plan to build again soon
```


### Cross-builds

Rust (cargo) can build grin for many platforms, so in theory running `grin`
as a validating node on your low powered device might be possible.
To cross-compile `grin` on a x86 Linux platform and produce ARM binaries,
say, for a Raspberry Pi, uncomment or add the `no-plugin-build` feature in
`grin.toml` to avoid the mining plugins.


### Building the Cuckoo-Miner plugins

Building `grin_pow` might fail if you're not on a x86_64 system,
because that crate also builds external Cuckoo mining plugins.

To avoid building mining plugins, ensure your `pow/Cargo.toml' has a line

```
features=["no-plugin-build"]
```

and that it's not commented out.

## Build errors
See [Troubleshooting](https://github.com/mimblewimble/docs/wiki/Troubleshooting)

## What was built?

A successful build gets you:

 - `target/debug/grin` - the main grin binary

 - `target/debug/plugins/*` - mining plugins (optional)

With the included `grin.toml` unchanged,
if you execute `cargo run`
you get a `.grin` subfolder that grin starts filling up.

While testing, put the grin binary on your path like this:

```
export PATH=/path/to/grin/dir/target/debug:$PATH
```

# Configuration

Grin has a good defaults, a configuration file `grin.toml` that's documented inline that can override the defaults,
and command line switches that has top priority and overrides all others.

The `grin.toml` file can placed in one of several locations, using the first one it finds:

1. The current working directory
2. In the directory that holds the grin executable
3. {USER_HOME}/.grin

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

## Basic usage

Running just `grin` with no command line switches starts `grin server` using defaults and any settings from `grin.toml` if found.


## Simulating: a chain and a few nodes

For a basic example, make a directory 'node1' and enter it.
We'll run a wallet listener and a server that creates a new local blockchain.

Copy over the grin.toml file from the grin folder and into your new node1 folder.

The miner needs a wallet to send mining rewards to, or else it can't mine.

So begin with the wallet listener in node1:

```
node1$ grin wallet init
node1$ grin wallet -p "password" listen
```

Also try `grin wallet help`.

The above created a wallet listener on the default port 13415,
using the wallet.seed and the password "password".

Now open another terminal window on the same machine.
Go to the 'node1' directory, and run a node that is mining:

```
node1$ grin server -m run
```

A new .grin folder is created, and starts filling up with new blocks having
just one coinbase transaction

Note that `server run` starts two services listening on two default ports:

 - port 13414 for the peer-to-peer (P2P) service which keeps all nodes synchronized
 - and 13413 for the Rest API service that can verify transactions and post new transactions to the pool.

The port numbers can be configured in grin.toml or on the command line, explained above.

Let the mining server find a few blocks, then stop (just ctrl-c) the mining server and the wallet server.

Now take a look in .grin. Here grin built the blockchain, peer data, and more.
You should also have a wallet.dat file which contains some coinbase mining rewards.
They are created each time the server starts on a new block.
If you have multiple miners active, you might see unused coinbase rewards,
and/or if your transaction gets orphaned, a similar situation can occur.
The not usable outputs are expected to be cleaned out over time, so it's ok
and nothing to worry about.

And if you see "slogger: dropped messages" in your mining node window it's
nothing to worry. It means your grin is so busy that it can't show on screen
(or in grin.log) all the details, unless you slow it down.

## Advanced Examples

See [usage](usage.md) and on the wiki
[How to use grin](https://github.com/mimblewimble/docs/wiki/How-to-use-grin).
