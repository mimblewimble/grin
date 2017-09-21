# Pruning Blockchain Data

One of the principal attractions of MimbleWimble is its theoretical space
efficiency. Indeed, a trusted or pre-validated full blockchain state only
requires unspent transaction outputs, which could be tiny.

The grin blockchain includes the following types of data (we assume prior
understanding of the MimbleWimble protocol):

1. Transaction outputs, which include for each output:
    1. A Pedersen commitment (33 bytes).
    2. A range proof (over 5KB at this time).
2. Transaction inputs, which are just output references (32 bytes).
3. Transaction "proofs", which include for each transaction:
    1. The excess commitment sum for the transaction (33 bytes).
    2. A signature generated with the excess (71 bytes average).
4. A block header includes Merkle trees and proof of work (about 250 bytes).

Assuming a blockchain of a million blocks, 10 million transactions (2 inputs, 2.5
outputs average) and 100,000 unspent outputs, we get the following approximate
sizes with a full chain (no pruning, no cut-through):

* 128GB of transaction data (inputs and outputs).
* 1 GB of transaction proof data.
* 250MB of block headers.
* Total chain size around 130GB.
* Total chain size, after cut-through (but incl. headers) of 1.8GB.
* UTXO size of 520MB.
* Total chain size, without range proofs of 4GB.
* UTXO size, without range proofs of 3.3MB.

We note that out of all that data, once the chain has been fully validated, only
the set of UTXO commitments is strictly required for a node to function.

There may be several contexts in which data can be pruned:

* A fully validating node may get rid of some data it has already validated to
free space.
* A partially validating node (similar to SPV) may not be interested in either
receiving or keeping all the data.
* When a new node joins the network, it may temporarily behave as a partially
validating node to make it available for use faster, even if it ultimately becomes
a fully validating node.
