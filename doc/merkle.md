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

### Sum Tree Disk Storage

The sum tree is split in chunks that are handled independently and stored in
separate files.

    3         G
             / \
    2       M   \
          /   \  \
    1    X     Y  \  ---- cutoff height H=1
        / \   / \  \
    0  A   B C   D  E

      [----] [----] 
     chunk1 chunk2

Each chunk is a full tree rooted at height H, lesser than R, the height of the
tree root. Because our MMR is append-only, each chunk is guaranteed to never
change on additions. The remaining nodes are captured in a root chunk that
contains the top nodes (above H) in the MMR as well as the leftover nodes on
its right side.

In the example above, we have 2 chunks X[A,B] and Y[C,D] and a root chunk
G[M,E]. The cutoff height H=1 and the root height R=3.

Note that each non-root chunk is a complete and fully valid MMR sum tree in
itself. The root chunk, with each chunk replaced with a single pruned node,
is also a complete and fully valid MMR.

As new leaves get inserted in the tree, more chunks get extracted, reducing the
size of the root chunk.

Assuming a cutoff height of H and a root height of R, the size (in nodes) of
each chunk is:

    chunk_size = 2^(H+1)-1

The maximum size of the root chunk is:

    max_root_size = 2^(R-H)-1 + 2^(H+1)-2

If we set the cutoff height H=15 and assume a node size of 50 bytes, for a tree
with a root at height 26 (capable of containing all Bitcoin UTXOs as this time)
we obtain a chunk size of about 3.3MB (without pruning) and a maximum root chunk
size of about 3.4MB.

### Tombstone Log

Deleting a leaf in a given tree can be expensive if done naively, especially
if spread on multiple chunks that aren't stored in memory. It would require
loading the affected chunks, removing the node (and possibly pruning parents)
and re-saving the whole chunks back.

To avoid this, we maintain a simple append-only log of deletion operations that
tombstone a given leaf node. When the tombstone log becomes too large, we can
easily, in the background, apply it as a whole on affected chunks.

Note that our sum MMR never actually fully deletes a key (i.e. output
commitment) as subsequent leaf nodes aren't shifted and parents don't need
rebalancing. Deleting a node just makes its storage in the tree unnecessary,
allowing for potential additional pruning of parent nodes.

### Key to Tree Insertion Position Index

For its operation, our sum MMR needs an index from key (i.e. an output
commitment) to the position of that key in insertion order. From that
position, the tree can be walked down to find the corresponding leaf node.

To hold that index without having to keep it all in memory, we store it in a
fast KV store (rocksdb, a leveldb fork). This reduces the implementation effort
while still keeping great performance. In the future we may adopt a more
specialized storage to hold this index.

### Design Notes

We chose explicitly to not try to save the whole tree into a KV store. While
this may sound attractive, mapping a sum tree structure onto a KV store is
non-trivial. Having a separate storage mechanism for the MMR introduces
multiple advantages:

* Storing all nodes in a KV store makes it impossible to fully separate
the implementation of the tree and its persistence. The tree implementation
gets more complex to include persistence concerns, making the whole system
much harder to understand, debug and maintain.
* The state of the tree is consensus critical. We want to minimize the
dependency on 3rd party storages whose change in behavior could impact our
consensus (the position index is less critical than the tree, being layered
above).
* The overall system can be simpler and faster: because of some particular
properties of our MMR (append-only, same size keys, composable), the storage
solution is actually rather straightforward and allows us to do multiple
optimizations (i.e. bulk operations, no updates, etc.).

### Operations

We list here most main operations that the combined sum tree structure and its
storage logic have to implement. Operations that have side-effects (push, prune,
truncate) need to be reversible in case the result of the modification is deemed
invalid (root or sum don't match). 

* Bulk Push (new block):
  1. Partially clone last in-memory chunk (full subtrees will not change).
  2. Append all new hashes to the clone.
  3. New root hash and sum can be checked immediately.
  4. On commit, insert new hashes to position index, merge the clone in the
  latest in-memory chunk, save.
* Prune (new block): 
  1. On commit, delete from position index, add to append-only tombstone file.
  2. When append-only tombstone files becomes too large, apply fully and delete
  (in background).
* Exists (new block or tx): directly check the key/position index.
* Truncate (fork): usually combined with a bulk push.
  1. Partially clone truncated last (or before last) in-memory chunk (again, full subtrees before the truncation position will not change).
  2. Proceed with bulk push as normal.

