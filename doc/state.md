# State and Storage

*Read this in other languages: [Korean](translations/state_KR.md), [日本語](translations/state_JP.md), [简体中文](translations/state_ZH-CN.md).*

## The Grin State

### Structure

The full state of a Grin chain consists of all the following data:

1. The full unspent output (UTXO) set.
1. The range proof for each output.
1. All the transaction kernels.
1. A MMR for each of the above (with the exception that the output MMR includes
   hashes for *all* outputs, not only the unspent ones).

In addition, all headers in the chain are required to anchor the above state
with a valid proof of work (the state corresponds to the most worked chain).
We note that once each range proof is validated and the sum of all kernels
commitment is computed, range proofs and kernels are not strictly necessary for
a node to function anymore.

### Validation

With a full Grin state, we can validate the following:

1. The kernel signature is valid against its commitment (public key). This
   proves the kernel is valid.
1. The sum of all kernel commitments equals the sum of all UTXO commitments
   minus the total supply. This proves that kernels and output commitments are all
   valid and no coins have unexpectedly been created.
1. All UTXOs, range proofs and kernels hashes are present in their respective
   MMR and those MMRs hash to a valid root.
1. A known block header with the most work at a given point in time includes
   the roots of the 3 MMRs. This validates the MMRs and proves that the whole
   state has been produced by the most worked chain.

### MMRs and Pruning

The data used to produce the hashes for leaf nodes in each MMR (in addition to
their position is the following:

* The output MMR hashes the feature field and the commitments of all outputs
  since genesis.
* The range proof MMR hashes the whole range proof data.
* The kernel MMR hashes all fields of the kernel: feature, fee, lock height,
  excess commitment and excess signature.

Note that all outputs, range proofs and kernels are added in their respective
MMRs in the order they occur in each block (recall that block data is required
to be sorted).

As outputs get spent, both their commitment and range proof data can be
removed. In addition, the corresponding output and range proof MMRs can be
pruned.

## State Storage

Data storage for outputs, range proofs and kernels in Grin is simple: a plain
append-only file that's memory-mapped for data access. As outputs get spent,
a remove log maintains which positions can be removed. Those positions nicely
match MMR node positions as they're all inserted in the same order. When the
remove log gets large, corresponding files can be occasionally compacted by
rewriting them without the removed pieces (also append-only) and the remove
log can be emptied. As for MMRs, we need to add a little more complexity.
