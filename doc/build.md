# Grin - Build, Configuration, and Running

# Building

## Supported Platforms

Note that it's still too early in development to declare 'officially supported' plaforms, but at the moment, the situation is:

* Linux - Primary platform (x86 only, at present), as most development and testing is happening here
* Mac OS - Known to work, but may be slight hiccups
* Windows - Known to compile, but working status unknown, and not a focus for the development team at present. Note that no mining plugins will be present on a Windows system after building Grin.

The instructions below will assume a Linux system.

## Build Prerequisites

In order to compile and run Grin on your machine, you should have installed:

* <b>Git</b> - to clone the repository
* <b>cmake</b> - 3.2 or greater should be installed and on your $PATH. Used by the build to compile the mining plugins found in the included [Cuckoo Miner](https://github.com/mimblewimble/cuckoo-miner)
* <b>Rust</b> - 1.21.0 or greater via [Rustup](https://www.rustup.rs/) - Can be installed via your package manager or manually via the following commands:
```
    curl https://sh.rustup.rs -sSf | sh
    source $HOME/.cargo/env
```

## Build Instructions (Linux/Unix)


### Clone Grin

```
    git clone https://github.com/mimblewimble/grin.git
```

### Build Grin
```
    cd grin
    #if running a testnet1 node, check out the correct branch:
    git checkout milestone/testnet1 
    cargo build
```

### Cuckoo-Miner considerations

If you're having issues with building cuckoo-miner plugins (which will usually manifest as a lot of C errors when building the `grin_pow` crate, you can turn mining plugin builds off by editing the file `pow/Cargo.toml' as follows:

```
#uncomment this feature to turn off plugin builds
features=["no-plugin-build"]
```

This may help when building on 32 bit systems or non x86 architectures. You can still use the internal miner to mine by setting:

```
use_cuckoo_miner = false
```

In `grin.toml`

## What have I just built?

Provided all of the prerequisites were installed and there were no issues, there should be 3 things in your project directory that you need to pay attention to in order to configure and run grin. These are:

* The Grin binary, which should be located in your project directory as target/debug/grin

* A set of mining plugins, which should be in the 'plugins' directory located next to the grin executable

* A configuration file in the root project directory named grin.toml

For the time being, it's recommended just to put the built version of grin on your path, e.g. via:

```
export PATH=/path/to/grin/dir/target/debug:$PATH
```

# Configuration

Grin is currently configured via a combination of configuration file and command line switches, with any provided switches overriding the contents of the configuration file. To see a list of commands and switches use:

```
grin help
```

At startup, grin looks for a configuration file called 'grin.toml' in the following places in the following order, using the first one it finds:

* The current working directory
* The directory in which the grin executable is located
* {USER_HOME}/.grin

If no configuration file is found, command line switches must be given to grin in order to start it. If a configuration file is found but no command line switches are provided, grin starts in server mode using the values found in the configuration file.

At present, the relevant modes of operation are 'server' and 'wallet'. When running in server mode, any command line switches provided will override the values found in the configuration file. Running in wallet mode does not currently use any values from the configuration file other than logging output parameters.

# Running a Node

The following are minimal instructions to get a testnet1 node up and running.

After following the instructions above to build a testnet executable and ensuring it's on your system path, create two directories wherever you prefer. Call one 'wallet' and one 'server'.

In the 'wallet' directory (preferably in a separate terminal window), run the following command to create a wallet seed:

```
grin wallet init
```

Then, to run a publicly listening wallet receiver, run the following command:

```
grin wallet -p password -e receive
```

Next, in the 'server' directory in another terminal window, copy the grin.toml file from the project root:

```
cp /path/to/project/root/grin.toml .
```

Then, to start the server node:
```
grin server --mine run
```

The server should start, connect to the seed and any available peers, and place mining rewards into your running wallet listener.

From your 'wallet' directory, you should be able to check your wallet contents with the command:

```
grin wallet -p password info
```
as well as see the individual outputs with:
```
grin wallet -p password outputs
```

See [wallet](wallet.md) for more info on the various Grin wallet commands and options.

For further information on a more complicated internal setup for testing, see the [local net documentation](local_net.md)

The [grin.toml](../grin.toml) configuration file has further information about the various options available.

