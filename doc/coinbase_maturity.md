# The Coinbase Maturity Rule (aka Output Lock Heights)

*Read this in other languages: [Korean](translations/coinbase_maturity_KR.md), [简体中文](translations/coinbase_maturity_ZH-CN).*

Coinbase outputs (block rewards & fees) are "locked" and require 1,440 confirmations (i.e 24 hours worth of blocks added to the chain) before they mature sufficiently to be spendable. This is to reduce the risk of later txs being reversed if a chain reorganization occurs.

Bitcoin does something very similar, requiring 100 confirmations (Bitcoin blocks are every 10 minutes, Grin blocks are every 60 seconds) before mining rewards can be spent.

Grin enforces coinbase maturity in both the transaction pool and the block validation pipeline. A transaction containing an input spending a coinbase output cannot be added to the transaction pool until it has sufficiently matured (based on current chain height and the height of the block producing the coinbase output).
Similarly a block is invalid if it contains an input spending a coinbase output before it has sufficiently matured, based on the height of the block containing the input and the height of the block that originally produced the coinbase output.

The maturity rule *only* applies to coinbase outputs, regular transaction outputs have an effective lock height of zero.

An output consists of -

* features (currently coinbase vs. non-coinbase)
* commitment `rG+vH`
* rangeproof

To spend a regular transaction output two conditions must be met. We need to show the output has not been previously spent and we need to prove ownership of the output.

A Grin transaction consists of the following -

* A set of inputs, each referencing a previous output being spent.
* A set of new outputs that include -
  * A value `v` and a blinding factor (private key) `r` multiplied on a curve and summed to be `rG+vH`
  * A range proof that shows that v is non-negative.
* An explicit transaction fee in the clear.
* A signature, computed by taking the excess blinding value (the sum of all outputs plus the fee, minus the inputs) and using it as the private key.

We can show the output is unspent by looking for the commitment in the current Output set. The Output set is authoritative; if the output exists in the Output set we know it has not yet been spent. If an output does not exist in the Output set we know it has either never existed, or that it previously existed and has been spent (we will not necessarily know which).

To prove ownership we can verify the transaction signature. We can *only* have signed the transaction if the transaction sums to zero *and* we know both `v` and `r`.

Knowing `v` and `r` we can uniquely identify the output (via its commitment) *and* we can prove ownership of the output by validating the signature on the original coinbase transaction.

Grin does not permit duplicate commitments to exist in the Output set at the same time.
But once an output is spent it is removed from the Output set and a duplicate commitment can be added back into the Output set.
This is not necessarily recommended but Grin must handle this situation in a way that does not break consensus across the network.

Several things complicate this situation -

1. It is possible for two blocks to have identical rewards, particularly for the case of empty blocks, but also possible for non-empty blocks with transaction fees.
1. It is possible for a non-coinbase output to have the same value as a coinbase output.
1. It is possible (but not recommended) for a miner to reuse private keys.

Grin does not allow duplicate commitments to exist in the Output set simultaneously.
But the Output set is specific to the state of a particular chain fork. It *is* possible for duplicate *identical* commitments to exist simultaneously on different concurrent forks.
And these duplicate commitments may have different "lock heights" at which they mature and become spendable on the different forks.

* Output O<sub>1</sub> from block B<sub>1</sub> spendable at height h<sub>1</sub> (on fork f<sub>1</sub>)
* Output O<sub>1</sub>' from block B<sub>2</sub> spendable at height h<sub>2</sub> (on fork f<sub>2</sub>)

The complication here is that input I<sub>1</sub> will spend either O<sub>1</sub> or O<sub>1</sub>' depending on which fork the block containing I<sub>1</sub> exists on. And crucially I<sub>1</sub> may be valid at a particular block height on one fork but not the other.

Said another way - a commitment may refer to multiple outputs, all of which may have different lock heights. And we *must* ensure we correctly identify which output is actually being spent and that the coinbase maturity rules are correctly enforced based on the current chain state.

A coinbase output, locked with the coinbase maturity rule at a specific lock height, *cannot* be uniquely identified, and *cannot* be safely spent by their commitment alone. To spend a coinbase output we need to know one additional piece of information -

* The block the coinbase output originated from

Given this, we can verify the height of the block and derive the "lock height" of the output (+ 1,000 blocks).

## Full Archival Node

Given a full archival node it is a simple task to identify which block the output originated from.
A full archival node stores the following -

* full block data of all blocks in the chain
* full output data for all outputs in these blocks

We can simply look back though all the blocks on the chain and find the block containing the output we care about.

The problem is when we need to account nodes that may not have full block data (pruned nodes, non-archival nodes).
[what kind of nodes?]

How do we verify coinbase maturity if we do not have full block data?

## Non-Archival Node

[terminology? what are these nodes called?]

A node may not have full block data.
A pruned node may only store the following (refer to pruning doc) -

* Block headers chain.
* All transaction kernels.
* All unspent outputs.
* The output MMR and the range proof MMR

Given this minimal set of data how do we know which block an output originated from?

And given we now know multiple outputs (multiple forks, potentially different lock heights) can all have the *same* commitment, what additional information do we need to provide in the input to uniquely identify the output being spent?

And to take it a step further - can we do all this without relying on having access to full output data? Can we use just the output MMR?

### Proposed Approach

We maintain an index mapping commitment to position in the output MMR.

If no entry in the index exists or no entry in the output MMR exists for a given commitment then we now the output is not spendable (either it was spent previously or it never existed).

If we find an entry in the output MMR then we know a spendable output exists in the Output set *but* we do not know if this is the correct one. We do not know if it is a coinbase output or not and we do not know the height of the block it originated from.

If the hash stored in the output MMR covers both the commitment and the output features and we require an input to provide both the commitment and the feature then we can do a further validation step -

* output exists in the output MMR (based on commitment), and
* the hash in the MMR matches the output data included in the input

With this additional step we know if the output was a coinbase output or a regular transaction output based on the provided features.
The hash will not match unless the features in the input match the original output features.

For a regular non-coinbase output we are finished. We know the output is currently spendable and we do not need to check the lock height.

For a coinbase output we can proceed to verify the lock height and maturity. For this we need to identify the block where the output originated.
We cannot determine the block itself, but we can require the input to specify the block (hash) and we can then prove this is actually correct based on the merkle roots in the block header (without needing full block data).

[tbd - overview of merkle proofs and how we will use these to prove inclusion based on merkle root in the block header]

To summarize -

Output MMR stores output hashes based on `commitment|features` (the commitment itself is not sufficient).

We do not need to include the range proof in the generation of the output hash.

To spend an output we continue to need -

* `r` and `v` to build the commitment and to prove ownership

An input must provide -

* the commitment (to lookup the output in the MMR)
* the output features (hash in output MMR dependent on features|commitment)
* a merkle proof showing inclusion of the output in the originating block
* the block hash of originating blocks
  * [tbd - maintain index based on merkle proof?]

From the commitment and the features we can determine if the correct output is currently unspent.
From the block and the output features we can determine the lock height (if any).
