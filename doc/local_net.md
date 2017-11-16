# Grin Local Testing Network

## Basic Execution

For a basic example simulating a single node network, create a directory called 'node1' and change your working directory to it. You'll use this directory to run a wallet and create a new blockchain via a server running in mining mode.

You'll need a config file - the easiest is to copy over the grin.toml file from the root grin directory into the node1 directory you just made.

Before running your mining server, a wallet server needs to be set up and listening so that the mining server knows where to send mining rewards. Do this from the first node directory with the following commands:

	node1$ grin wallet init
	node1$ grin wallet -p "password" receive

See [wallet](wallet.md) for more info on the various Grin wallet commands and options.

This will create a wallet server listening on the default port 13415 with the password "password". Next, in another terminal window in the 'node1' directory, run a full mining node with the following command:

	node1$ grin server -m run

This creates a new .grin database directory in the current directory, and begins mining new blocks (with no transactions, for now). Note this starts two services listening on two default ports,
port 13414 for the peer-to-peer (P2P) service which keeps all nodes synchronized, and 13413 for the Rest API service used to verify transactions and post new transactions to the pool (for example). These ports can be configured via command line switches, or via a grin.toml file in the working directory.

Let the mining server find a few blocks, then stop (just ctrl-c) the mining server and the wallet server. You'll notice grin has created a database directory (.grin) in which the blockchain and peer data is stored. There should also be a wallet.dat file in the current directory, which contains a few coinbase mining rewards created each time the server mines a new block.

## Advanced Example

The following outlines a more advanced example simulating a multi-server network with transactions being posted.

For the sake of example, we're going to run three nodes with varying setups. Create two more directories beside your node1 directory, called node2 and node3. If you want to clear data from your previous run (or anytime you want to reset the blockchain and all peer data) just delete the wallet.dat file in the node1 directory and run rm -rf .grin to remove grin's database.

### Node 1: Genesis and Miner

As before, node 1 will create the blockchain and begin mining. As we'll be running many servers from the same machine, we'll configure specific ports for other servers to explicitly connect to.

First, we run a wallet server to receive rewards on port 15000 (we'll log in debug mode for more information about what's happening)

    node1$ grin wallet -p "password" -a "http://127.0.0.1:10001" -r 15000 receive

Then we start node 1 mining with its P2P server bound to port 10000 and its api server at 10001. We also provide our wallet address where we'll receive mining rewards. In another terminal:

    node1$ grin server -m -p 10000 -a 10001 -w "http://127.0.0.1:15000" run

### Node 2: Regular Node (not mining)

We'll set up Node 2 as a simple validating node (i.e. it won't mine,) but we'll pass in the address of node 1 as a seed. Node 2 will join the network founded by node 1 and then sync its blockchain and peer data.

In a new terminal, tell node 2 to run a server using node 1's P2P address as a seed.  Node 2's P2P server will run on port 20000 and its API server will run on port 20001.

    node2$ grin server -s "127.0.0.1:10000" -p 20000 -a 20001 run

Node 2 will then sync and process and validate new blocks that node 1 may find.

### Node 3: Regular node running wallet listener

Similar to Node 2, we'll set up node 3 as a non-mining node seeded with node 2 (node 1 could also be used). However, we'll also run another wallet in listener mode on this node:

    node3$ grin server -s "127.0.0.1:20000" -p 30000 -a 30001 run

Node 3 is now running it's P2P service on port 30000 and its API server on 30001. You should be able to see it syncing its blockchain and peer data with nodes 1 and 2. Now start up a wallet listener.

    node3$ grin wallet -p "password" -a "http://127.0.0.1:30001" -r 35000 receive

In contrast to other blockchains, a feature of a MimbleWimble is that a transaction cannot just be directly posted to the blockchain. It first needs to be sent from the sender to the receiver,
who will add a blinding factor before posting it to the blockchain. The above command tells the wallet server to listen for transactions on port 35000, and, after applying it's own blinding factor to the transaction, forward them on to the listening API server on node 1. (NB: we should theoretically be able to post transactions to node 3 or 2, but for some reason transactions posted to peers don't seem to propagate properly at present)

### Node 1 - Send money to node 3

With all of your servers happily running and your terminals scrolling away, let's spend some of the coins mined in node 1 by sending them to node 3's listening wallet.

In yet another terminal in node 1's directory, create a new partial transaction spending 20000 coins and send them on to node 3's wallet listener. We'll also specify that we'll
use node 2's API listener to validate our transaction inputs before sending:

    node1$ grin wallet -p "password" -a "http://127.0.0.1:20001" send 20000 -d "http://127.0.0.1:35000"

Your terminal windows should all light up now. Node 1 will check its inputs against node 2, and then send a partial transaction to node 3's wallet listener. Node 3 has been configured to
send signed and finalised transactions to the api listener on node 1, which should then add the transaction to the next block and validate it via mining.

You can feel free to try any number of permutations or combinations of the above, just note that grin is very new and under active development, so your mileage may vary. You can also use a separate 'grin.toml' file in each server directory to simplify command line switches.
