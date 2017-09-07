Blockchain Syncing
==================

We describe here the different methods used by a new node when joining the network
to catch up with the latest chain state. We start with reminding the reader of the
following assumptions, which are all characteristics of Grin or MimbleWimble:

* All block headers include the root hash of all unspent outputs in the chain at
  the time of that block.
* Inputs or outputs cannot be tampered with or forged without invalidating the
  whole block state. 

We're purposefully only focusing on major node types and high level algorithms that
may impact the security model. Detailed heuristics that can provide some additional
improvements (like header first), while useful, will not be mentioned in this
section.

## Full History Syncing

### Description

This model is the one used by "full nodes" on most major public blockchains. The
new node has prior knowledge of the genesis block. It connects to other peers in
the network and starts asking for blocks until it reaches the latest block known to
its peers.

The security model here is similar to bitcoin. We're able to verify the whole
chain, the total work, the validity of each block, their full content, etc.
In addition, with MimbleWimble and full UTXO set commitments, even more integrity
validation can be performed.

We do not try to do any space or bandwidth optimization in this mode (for example,
once validated the range proofs could possibly be deleted). The point here is to
provide history archival and allow later checks and verifications to be made.

### What could go wrong?

Identical to other blockchains:

* If all nodes we're connected to are dishonest (sybil attack or similar), we can
  be lied to about the whole chain state.
* Someone with enormous mining power could rewrite the whole history.
* Etc.

## Partial History Syncing

In this model we try to optimize for very fast syncing while sacrificing as little
security assumptions as possible. As a matter of fact, the security model is almost
identical as a full node, despite requiring orders of magnitude less data to
download.

A new node is pre-configured with a horizon `Z`, which is a distance in number of
blocks from the head. For example, if horizon `Z=5000` and the head is at height
`H=23000`, the block at horizon is the block at height `h=18000` on the most
worked chain.

The new node also has prior knowledge of the genesis block. It connects to other
peers and learns about the head of the most worked chain. It asks for the block
header at the horizon block, requiring peer agreement. If consensus is not reached
at `h = H - Z`, the node gradually increases the horizon `Z`, moving `h` backward
until consensus is reached. Then it gets the full UTXO set at the horizon block.
With this information it can verify: 

* the total difficulty on that chain (present in all block headers)
* the sum of all UTXO commitments equals the expected money supply
* the root hash of all UTXOs match the root hash in the block header

Once the validation is done, the peer can download and validate the blocks content
from the horizon up to the head.

While this algorithm still works for very low values of `Z` (or in the extreme case
where `Z=1`), low values may be problematic due to the normal forking activity that
can occur on any blockchain. To prevent those problems and to increase the amount
of locally validated work, we recommend values of `Z` of at least a few days worth
of blocks, up to a few weeks.

### What could go wrong?

While this sync mode is simple to describe, it may seem non-obvious how it still
can be secure. We describe here some possible attacks, how they're defeated and
other possible failure scenarios.

#### An attacker tries to forge the state at horizon

This range of attacks attempt to have a node believe it is properly synchronized
with the network when it's actually is in a forged state. Multiple strategies can
be attempted:

* Completely fake but valid horizon state (including header and proof of work).
Assuming at least one honest peer, neither the UTXO set root hash nor the block
hash will match other peers' horizon states.
* Valid block header but faked UTXO set. The UTXO set root hash from the header
will not match what the node calculates from the received UTXO set itself.
* Completely valid block with fake total difficulty, which could lead the node down
a fake fork. The block hash changes if the total difficulty is changed, no honest
peer will produce a valid head for that hash.

#### A fork occurs that's older than the local UTXO history

Our node downloaded the full UTXO set at horizon height. If a fork occurs on a block
at an older horizon H+delta, the UTXO set can't be validated. In this situation the
node has no choice but to put itself back in sync mode with a new horizon of
`Z'=Z+delta`.

Note that an alternate fork at Z+delta that has less work than our current head can
safely be ignored, only a winning fork of total work greater than our head would.
To do this resolution, every block header includes the total chain difficulty up to
that block.

#### The chain is permanently forked

If a hard fork occurs, the network may become split, forcing new nodes to always
push their horizon back to when the hard fork occurred. While this is not a problem
for short-term hard forks, it may become an issue for long-term or permanent forks
To prevent this situation, peers should always be checked for hard fork related
capabilities (a bitmask of features a peer exposes) on connection.

### Several nodes continuously give fake horizon blocks

If a peer can't reach consensus on the header at h, it gradually moves back. In the
degenerate case, rogue peers could force all new peers to always become full nodes
(move back until genesis) by systematically preventing consensus and feeding fake
headers.

While this is a valid issue, several mitigation strategies exist:

* Peers must still provide valid block headers at horizon `Z`. This includes the
proof of work.
* A group of block headers around the horizon could be asked to increase the cost
of the attack.
* Differing block headers providing a proof of work significantly lower could be
rejected.
* The user or node operator may be asked to confirm a block hash.
* In last resort, if none of the above strategies are effective, checkpoints could
be used.
