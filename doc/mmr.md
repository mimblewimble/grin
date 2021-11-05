# Merkle Mountain Ranges

*Read this in other languages: [Korean](translations/mmr_KR.md), [简体中文](translations/mmr_ZH-CN.md).*

## Structure

Merkle Mountain Ranges [1] are an alternative to Merkle trees [2]. While the
latter relies on perfectly balanced binary trees, the former can be seen
either as list of perfectly balance binary trees or a single binary tree that
would have been truncated from the top right. A Merkle Mountain Range (MMR) is
strictly append-only: elements are added from the left to the right, adding a
parent as soon as 2 children exist, filling up the range accordingly.

This illustrates a range with 11 inserted leaves and total size 19, where each
node is annotated with its order of insertion.

```
Height

3              14
             /    \
            /      \
           /        \
          /          \
2        6            13
       /   \        /    \
1     2     5      9     12     17
     / \   / \    / \   /  \   /  \
0   0   1 3   4  7   8 10  11 15  16 18
```

This can be represented as a flat list, here storing the height of each node
at their position of insertion:

```
0  1  2  3  4  5  6  7  8  9 10 11 12 13 14 15 16 17 18
0  0  1  0  0  1  2  0  0  1  0  0  1  2  3  0  0  1  0
```

This structure can be fully described simply from its size (19). It's also
fairly simple, using fast binary operations, to navigate within a MMR.
Given a node's position `n`, we can compute its height, the position of its
parent, its siblings, etc.

## Hashing and Bagging

Just like with Merkle trees, parent nodes in a MMR have for value the hash of
their 2 children. Grin uses the Blake2b hash function throughout, and always
prepends the node's position in the MMR before hashing to avoid collisions. So
for a leaf `l` at index `n` storing data `D` (in the case of an output, the
data is its Pedersen commitment, for example), we have:

```
Node(l) = Blake2b(n | D)
```

And for any parent `p` at index `m`:

```
Node(p) = Blake2b(m | Node(left_child(p)) | Node(right_child(p)))
```

Contrarily to a Merkle tree, a MMR generally has no single root by construction
so we need a method to compute one (otherwise it would defeat the purpose of
using a hash tree). This process is called "bagging the peaks" for reasons
described in [1].

First, we identify the peaks of the MMR.

The MMR above has 19 nodes and 3 peaks, each of which
is the root of a subtree of size a 2-power minus one.
If the word-size were 8-bits, then 19 is 00010011 in binary, with 3 leading zeros.
Shifting the all 1-bit word 11111111 right by that number 3 gives us 00011111, or 31,
the first candidate peak size.
Since 19 < 31, we have no 31-peak.
The next candidate peak size is 31 >> 1 = 15.
Since 19 >= 15, we have a 15-peak, and the relative position beyond identified
peaks is 19-15=4.
After 2 more right shifts to peak size 3, we find 4 >= 3 and identify the 2nd peak,
reducing relative position to 4-3 = 1.
A final right shift gives a peak size of 1, and with 1 >= 1, we identified the 3rd and final peak.

Finally, once all the positions of the peaks are known, "bagging" the peaks
consists of hashing them iteratively from the right, using the total size of
the MMR as prefix. For a MMR of size N with 3 peaks p1, p2 and p3 we get the
final top peak:

```
P = Blake2b(N | Blake2b(N | Node(p3) | Node(p2)) | Node(p1))
```

## Pruning

In Grin, a lot of the data that gets hashed and stored in MMRs can eventually
be removed. As this happens, the presence of some leaf hashes in the
corresponding MMRs become unnecessary and their hash can be removed. When
enough leaves are removed, the presence of their parents may become unnecessary
as well. We can therefore prune a significant part of a MMR from the removal of
its leaves.

Pruning a MMR relies on a simple iterative process. `X` is first initialized as
the leaf we wish to prune.

1. Prune `X`.
1. If `X` has a sibling, stop here.
1. If 'X' has no sibling, assign the parent of `X` as `X`.

To visualize the result, starting from our first MMR example and removing leaves
[0, 3, 4, 8, 16] leads to the following pruned MMR:

```
Height

3             14
            /    \
           /      \
          /        \
         /          \
2       6            13
       /            /   \
1     2            9     12     17
       \          /     /  \   /
0       1        7     10  11 15     18
```

[1] Peter Todd, [merkle-mountain-range](https://github.com/opentimestamps/opentimestamps-server/blob/master/doc/merkle-mountain-range.md)

[2] [Wikipedia, Merkle Tree](https://en.wikipedia.org/wiki/Merkle_tree)
