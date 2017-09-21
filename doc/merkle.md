# Merkle Structures

MimbleWimble is designed for users to verify the state of the system given
only pruned data. To achieve this goal, all transaction data is committed
to the blockchain by means of Merkle trees which should support efficient
updates and serialization even when pruned.

Also, almost all transaction data (inputs, outputs, excesses and excess
proofs) have the ability to be summed in some way, so it makes sense to
treat Merkle sum trees as the default option, and address the sums here.

A design goal of Grin is that all structures be as easy to implement and
as simple as possible. MimbleWimble introduces a lot of new cryptography
so it should be made as easy to understand as possible. Its validation rules
are simple to specify (no scripts) and Grin is written in a language with
very explicit semantics, so simplicity is also good to achieve well-understood
consensus rules.

## Merkle Trees

There are four Merkle trees committed to by each block:

### Total Output Set

Each object is one of two things: a commitment indicating an unspent output
or a NULL marker indicating a spent one. It is a sum-tree over all unspent
outputs (spent ones contribute nothing to the sum). The output set should
reflect the state of the chain *after* the current block has taken effect.

The root sum should be equal to the sum of all excesses since the genesis.

Design requirements:

1. Efficient additions and updating from unspent to spent.
2. Efficient proofs that a specific output was spent.
3. Efficient storage of diffs between UTXO roots.
4. Efficient tree storage even with missing data, even with millions of entries.
5. If a node commits to NULL, it has no unspent children and its data should
   eventually be able to be dropped forever.
6. Support serializating and efficient merging of pruned trees from partial
   archival nodes.

### Output witnesses

This tree mirrors the total output set but has rangeproofs in place of commitments.
It is never updated, only appended to, and does not sum over anything. When an
output is spent it is sufficient to prune its rangeproof from the tree rather
than deleting it.

Design requirements:

1. Support serializating and efficient merging of pruned trees from partial
   archival nodes.

### Inputs and Outputs

Each object is one of two things: an input (unambiguous reference to an old
transaction output), or an output (a (commitment, rangeproof) pair). It is
a sum-tree over the commitments of outputs, and the negatives of the commitments
of inputs.

Input references are hashes of old commitments. It is a consensus rule that
there are never two identical unspent outputs.

The root sum should be equal to the sum of excesses for this block. See the
next section.

In general, validators will see either 100% of this Merkle tree or 0% of it,
so it is compatible with any design. Design requirements:

1. Efficient inclusion proofs, for proof-of-publication.

### Excesses

Each object is of the form (excess, signature). It is a sum tree over the
excesses.

In general, validators will always see 100% of this tree, so it is not even
necessary to have a Merkle structure at all. However, to support partial
archival nodes in the future we want to support efficient pruning.

Design requirements:

1. Support serializating and efficient merging of pruned trees from partial
   archival nodes.


## Proposed Merkle Structure

**The following design is proposed for all trees: a sum-MMR where every node
sums a count of its children _as well as_ the data it is supposed to sum.
The result is that every node commits to the count of all its children.**

[MMRs, or Merkle Mountain Ranges](https://github.com/opentimestamps/opentimestamps-server/blob/master/doc/merkle-mountain-range.md)

The six design criteria for the output set are:

### Efficient insert/updates

Immediate (as is proof-of-inclusion). This is true for any balanced Merkle
tree design.

### Efficient proof-of-spentness

Grin itself does not need proof-of-spentness but it is a good thing to support
in the future for SPV clients.

The children-counts imply an index of each object in the tree, which does not
change because insertions happen only at the far right of the tree.

This allows permanent proof-of-spentness, even if an identical output is later
added to the tree, and prevents false proofs even for identical outputs. These
properties are hard to achieve for a non-insertion-ordered tree.

### Efficient storage of diffs

Storing complete blocks should be sufficient for this. Updates are obviously
as easy to undo as they are to do, and since blocks are always processed in
order, rewinding them during reorgs is as simple as removing a contiguous
set of outputs from the right of the tree. (This should be even faster than
repeated deletions in a tree designed to support deletions.)

### Efficient tree storage even with missing data

To update the root hash when random outputs are spent, we do not want to need
to store or compute the entire tree. Instead we can store only the hashes at
depth 20, say, of which there will be at most a million. Then each update only
needs to recompute hashes above this depth (Bitcoin has less than 2^29 outputs
in its history, so this means computing a tree of size 2^9 = 512 for each update)
and after all updates are done, the root hash can be recomputed.

This depth is configurable and may be changed as the output set grows, or
depending on available disk space.

This is doable for any Merkle tree but may be complicated by PATRICIA trees or
other prefix trees, depending how depth is computed.

### Dropping spent coins

Since coins never go from spent to unspent, the data on spent coins is not needed
for any more updates or lookups.

### Efficient serialization of pruned trees

Since every node has a count of its children, validators can determine the
structure of the tree without needing all the hashes, and can determine which
nodes are siblings, and so on.

In the output set each node also commits to a sum of its unspent children, so
a validator knows if it is missing data on unspent coins by checking whether or
not this sum on a pruned node is zero.


## Algorithms

(To appear alongside an implementation.)

## Storage

The sum tree data structure allows the efficient storage of the output set and
output witnesses while allowing immediate retrieval of a root hash or root sum
(when applicable). However, the tree must contain every output commitment and
witness hash in the system. This data too big to be permanently stored in
memory and too costly to be rebuilt from scratch at every restart, even if we
consider pruning (at this time, Bitcoin has over 50M UTXOs which would require
at least 3.2GB, assuming a couple hashes per UTXO). So we need an efficient way
to store this data structure on disk.

Another limitation of a hash tree is that, given a key (i.e. an output
commitment), it's impossible to find the leaf in the tree associated with that
key. We can't walk down the tree from the root in any meaningful way. So an
additional index over the whole key space is required. As an MMR is an append
only binary tree, we can find a key in the tree by its insertion position. So a
full index of keys inserted in the tree (i.e. an output commitment) to their
insertion positions is also required.

