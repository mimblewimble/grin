# How-to: Run a Grin node on Google Cloud for free
Thanks to Google Cloud's [Always Free](https://cloud.google.com/free/docs/frequently-asked-questions#always-free)  program, it's possible to create an instance on Cloud Compute that runs a full Grin node, 24/7, without it costing you anything. This is a cheap (free!) and fool-proof way to get a node up and running since it:
* Is not dependent on the compatibility of your computer or OS;
* Does not require you to open up ports on your network;
* Starts you off from a clean instance; and
* Allows you to follow instructions that are replicable.

The only requirement is that you are willing to sign up for a Google Cloud account and that you have a valid credit or debit card (which will not be charged).

**NOTE: This is for testing purposes only. The node is free to run, but if you make mistakes in the configuration you may incur charges to your card. Make sure you monitor your account and billing status regularly whilst running your instance to avoid any unpleasant surprises.**

## Google Cloud Set up
1. Visit http://cloud.google.com and set up an account as an individual. This will require a debit or credit card, they do a reserve $1 on your card to ensure it's valid. As part of signing up you also get $300 in free trial credit to spend within 12 months.
2. In order to qualify for [Always Free](https://cloud.google.com/free/docs/frequently-asked-questions#always-free) you need to have an upgraded account. So ensure you [upgrade](https://cloud.google.com/free/docs/frequently-asked-questions#what-is-upgrade). Note that this means that you will start to be charged automatically if your spend beyond the $300 in free trial credit. As you will not exceed the [Always Free limits](https://cloud.google.com/free/docs/always-free-usage-limits) here, this point is moot, but keep it in mind for any other projects you use this account for.
3. Launch a [Cloud Shell](https://cloud.google.com/shell/) console from your browser, or install the [Google Cloud SDK](https://cloud.google.com/sdk/) to run Cloud Shell from your local terminal.

## Provisioning an instance
From the cloud shell, run the following command to create `grin-node1`, an always free-compatible instance running Linux Debian 9:
```
gcloud beta compute instances create grin-node1 --zone=us-east1-b --machine-type=f1-micro --tags=grin-node --image=debian-9-stretch-v20180716 --image-project=debian-cloud --boot-disk-size=10GB --boot-disk-type=pd-standard --boot-disk-device-name=grin-disk1
```

## Building
Your newly created `grin-node1` should now be visible in your list of [Cloud Compute Instances](https://console.cloud.google.com/compute/instances). From there, open an SSH session in your browser by clicking the `SSH` button, or SSH to the instance [through your own terminal](https://cloud.google.com/compute/docs/instances/connecting-advanced#thirdpartytools).

As always, first update your system:
```
sudo apt-get update
```
Install some tools:
* git
* nano, a simple text editor
* tmux, which will allow you to run multiple terminal sessions and keep your node running on your instance once you disconnect remotely. See [gentle intro](https://medium.com/actualize-network/a-minimalist-guide-to-tmux-13675fb160fa)  and [cheatsheet](https://gist.github.com/MohamedAlaa/2961058).
```
 sudo apt-get install git nano tmux
```

You can now enter a tmux session by `tmux` and at any time you can close down your connection by `CTRL+b` and then `d` as in detach, and then return to it later by `tmux a` as in attach.

Next install all dependencies:
* clang
* cmake
* ncurse
* zlibs

```
sudo apt-get install clang cmake libncurses5-dev libncursesw5-dev zlib1g-dev
```

Install rust:
```
curl https://sh.rustup.rs -sSf | sh; source $HOME/.cargo/env
```

Clone grin and build a release version
```
git clone https://github.com/mimblewimble/grin.git
cd grin
cargo build --release
```
Building takes ~60 minutes on the `grin-node1` instance. Slow, but it's free. Good time for a coffee break.

## Syncing the Grin node

When the build has completed, move `grin.toml` to the release directory, and launch the grin node:
```
mv grin.toml target/release
cd target/release
RUST_BACKTRACE=1 ./grin
```
The node should automatically connect to peers and begin syncing. This might also take a while, so you might want to go for another break. `tmux` is your friend. Once completed, the node should be at the same block height as http://grinscan.net/.

## Receiving some grins
Have a fully synced node up and running? Congrats, you're a Grin user! Time to receive some grins.

On your physical machine, open a [Cloud Shell](https://cloud.google.com/shell/) console window, and set up a firewall rule to allow ingress tcp connections on port 13415 to `grin-node1` so you can receive communication from other grin wallets:
```
gcloud compute firewall-rules create grin-wallet-port --direction=INGRESS --action=ALLOW --rules=tcp:13415 --target-tags=grin-node
```
Also obtain the internal and external IPs of your instance and write them down:
```
gcloud compute instances list
```

Now ssh back into `grin-node1`. Quit any running Grin node. Edit `grin.toml` to set `api_listen_interface = "INTERNAL_IP"` where `INTERNAL_IP` is the internal IP assigned to your instance by Google, which you obtained in the previous step.
```
cd grin/target/release
nano grin.toml
```

Esit, save, and exit `grin.toml`, and then launch your grin node again:
```
RUST_BACKTRACE=1 ./grin
```

You're now ready to receive grins. Try asking the GrinGod faucet for some. In a new terminal window on `grin-node1` whilst your node is still running:
```
curl gringod.info
```
To check on your wallet, run:
```
RUST_BACKTRACE=1 ./grin wallet info
```
And to view the transaction log:
```
RUST_BACKTRACE=1 ./grin wallet txs
```
Now you can receive grins from any other wallet on the network, simply give them your `http://EXTERNAL_IP:13415` where EXTERNAL_IP is the external IP assigned to your instance by Google, which you obtained at the beginning of this section.
Note: Your external IP is [ephemeral](https://cloud.google.com/compute/docs/ip-addresses/#ephemeraladdress) by default, so the moment you shut down or delete your instance, it will be released and you may not get the same assigned to you the next time you take an instance live. You can optionally choose to [assign a static IP](https://cloud.google.com/compute/docs/ip-addresses/reserve-static-external-ip-address) if you want to avoid, but that's beyond the scope of this document.

## Future sections:
* Mining from your local set up to your Google Cloud node wallet
* Connecting to [Grin-Pool](https://github.com/grin-pool/grin-pool)
