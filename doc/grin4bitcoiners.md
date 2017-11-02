# Grin/MimbleWimble for Bitcoiners

## Privacy and Fungibility

There are 3 main properties of Grin transactions that make them private:

1. There are no addresses.
2. There are no amounts.
3. 2 transactions, one spending the other, can be merged in a block to form only one, removing all intermediary information.

The 2 first properties mean that all transactions look the same. Unless you directly participated in the transaction, all inputs and outputs look like random pieces of data (in lingo, they're all random curve points).

Moreover, there are no more transactions in a block. A Grin block looks just like one giant transaction and all original association between inputs and outputs is lost.

## Scalability

As explained in the previous section, thanks to the MimbleWimble transaction and block format we can merge transactions when an output is directly spent by the input of another. It's as if when Alice gives money to Bob, and then Bob gives it all to Carol, Bob was never involved and his transaction is actually never even seen on the blockchain.

Pushing that further, between blocks, most outputs end up being spent sooner or later by another input. So *all spent outputs can be safely removed*. And the whole blockchain can be stored, downloaded and fully verified in just a few gigabytes or less (assuming a number of transactions similar to bitcoin).

What this all means is the Grin blockchain scales with the number of users (unspent outputs), not the number of transactions. At the moment, there is one caveat to that: a small piece of data (called a kernel, over about 100 bytes) needs to stay around for each transaction. But we're working on optimizing that as well.

## Scripting

Maybe you've heard that MimbleWimble doesn't support scripts. And in some way, that's true. But thanks to cryptographic trickery, many contracts that in bitcoin would require a script can be achieved with Grin using properties of Elliptic Curve Cryptography. So far, we know how to do:

* Multi-signature transactions.
* Atomic swaps.
* Time-locked transaction and outputs.
* Lightning Network

## Emmission Rate

Bitcoin's 10 minute block time has its intitial 50 btc reward cut in half every 4 years until there are 21 million Bitcoin in circulation. Grin's emission rate is linear, meaning it never drops. The block reward is currently set at 50 grin with a block goal of 60 seconds.    

## FAQ

### Wait, what!? No address?

Nope, no address. All outputs in Grin are unique and have no common data with any previous output. Instead of relying on a known address to send money, transactions have to be built interactively, with 2 (or more) wallets exchanging data with one another. Practically, this isn't so much of a problem as there are multiple ways for 2 programs to interact privately and securely. And this interaction could even take place over email or Signal (or carrier pigeons).

### If transactions information get removed, can't I just cheat and create money?

No, and this is where MimbleWimble and Grin shine. Confidential transactions are a form of [homomorphic encryption](https://en.wikipedia.org/wiki/Homomorphic_encryption). Without revealing any amount, Grin can verify that the sum of all transaction inputs equal the sum of transaction outputs, plus the fee. Going even further, comparing the sum of all money created by mining with the total sum of money that's being held, Grin nodes can check the correctness of the total money supply.

### If I listen to transaction relay, can I just figure out who they belong to before being cut-through?

You can figure out which outputs are being spent by which transaction. But the trail of data stops here. All inputs and outputs look like random pieces of data, so you can't tell if the money was transferred, still belongs to the same person, which output is the actual transfer and which is the change, etc. Grin transactions are built with *no identifiable piece of information*.

### What about the quantum computaggedon

In every Grin output, we also include a bit of hashed data, which is quantum safe. If quantum computing was to become a reality, we can safely introduce additional verification that would protect existing coins from being hacked.

### How does all this magic work?

See our [technical introduction](intro.md) to get started.
