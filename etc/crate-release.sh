#!/usr/bin/env bash

# check we're in the grin root
if [ ! -f "LICENSE" ] ; then
	echo "Script must be run from Grin's root directory"
	exit 1
fi

echo "Going to package and publish each crate, if you're not logged in crates.io (missing ~/.cargo/credentials, this will fail."
echo "Also check that rust-secp256k1-zkp has been published with the latest version. "

read -p "Continue? " -n 1 -r
if [[ ! $REPLY =~ ^[Yy]$ ]]
then
	printf "\nbye\n"
	exit 1
fi

echo
crates=( util keychain core store chain pool p2p api wallet servers config )

for crate in "${crates[@]}"
do
	echo "** Publishing $crate"
	cd $crate
	cargo package --allow-dirty
	cargo publish --allow-dirty
	cd ..
done

cargo package
cargo publish

echo "Done."
