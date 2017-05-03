# Basic Build Instructions on Linux/Unix

## Install Rust

    curl https://sh.rustup.rs -sSf | sh
    source $HOME/.cargo/env

or see instructions at:
https://www.rust-lang.org

## Clone Grin

    git clone https://github.com/ignopeverell/grin.git

## Build Grin

    cd grin
    cargo build

## Exec Grin

After compiling you'll get a binary at target/debug/grin. Place that in your path. Running 'grin help' should print a helpful message. Then create 3 directories for each server (the .grin db dir is created wherever you run the command from for now). Run the first server:

    serv1$ RUST_LOG#info grin server run --mine run

Let it find a few blocks. Then open a new terminal in the directory for the 2nd server and run (check 'grin help server' for the options):

    serv2$ RUST_LOG#info grin server -p 13424 --seed "127.0.0.1:13414" run

You'll need to give it a little bit, as it hangs for 10 sec trying to get more peers before deciding it'll only get one. Then it will sync and process and validate new blocks that serv1 may find. The "coinbase" for each block is generated with a random private key right now. You can then run a 3rd server, seeding either with the 1st or 2nd and it should connect to both and sync as well.


