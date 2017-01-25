Transaction Pool
==================

This document describes some of the basic functionality and requirements of grin's transaction pool.

## Overview of Required Capabilities

The primary purpose of the memory pool is to maintain a list of mineable transactions to be supplied to the miner service while building new blocks. The design will center around ensuring correct behavior here, especially around tricky conditions like head switching.

For standard (non-mining) nodes, the primary purpose of the memory pool is to serve as a moderator for transaction broadcasts by requiring connectivity to the blockchain. Secondary uses include monitoring incoming transactions, for example for giving early notice of an unconfirmed transaction to the user's wallet.

Given the focus of grin (and mimblewimble) on reduced resource consumption, the memory pool should be an optional but recommended component for non-mining nodes.

## Design Overview

The primary structure of the transaction pool is a pair of Directed Acyclic Graphs. Since each transaction is rooted directly by its inputs in a non-cyclic way, this structure naturally encompasses the directionality of the chains of unconfirmed transactions. Defining this structure has a few other nice properties: descendent invalidation (when a conflicting transaction is accepted for a given input) is nearly free, and the mineability of a given transaction is clearly depicted in its location in the heirarchy.

Another, non-obvious reason for the choice of a DAG is that the acyclic nature of transactions is a necessary property but must be explicitly verified in a way that is not true of other UTXO-based cryptocurrencies. Consider the following loop of single-input single-output transactions in BTC:

A->B->C->A

Because each input in Bitcoin specifically references the hash and output index of the output in a preceding transaction, for a loop to exist, a transaction must reference (and know the hash of) a transaction that does not yet exist (C, in the trivial example.) Furthermore, the hash and output index pair (called an "outpoint" in Bitcoin) is covered by the transaction hash of A, such that any change to either causes the hash of A to change. Therefore, attempting to build such a loop by amending A with the proper outpoint in C after C has been built causes A's hash to change, invalidating B, and so forth. 

In grin, an input references an output by the output's own hash. Thus, the backreference does not include the situation the output was generated in, which allows (from a purely mechanical point of view) the creation of a loop without the ability to generate a specific hash from a tightly constrained preimage. 

The pair of graphs represents the connected graph and the orphans graph. (While it is possible to represent both groups of transactions in a single graph, it makes determination of orphan status of a given transaction non-trivial, requiring either the maintainence of a flag or traversal upwards of potentially many inputs.)

A transaction reference in the pool has parents, one for each input. The parents fall into one of four states:

* Unknown
* Blockchain transaction
* Pool transaction
* Orphan transaction

A mineable transaction is defined as a transaction which has met all of its locktime requirements and which all parents are either blockchain transactions are mineable pool transactions. One such requirement is the maturity requirement for spending newly generated coins. This will also include the explicit per-transaction locktime, if adopted.

## Transaction Selection

In terms of needs, preference should be given to older transactions; beyond this, it seems beneficial to target transactions that reduce the maximum depth of the transaction graph, as this reduces the computational complexity of traversing the graph and making changes to it. Since fees are largely static, there is no need for fee preference.

Kahn's algorithm with the parameters above to break ties could provide a efficient mechanism for producing a correctly ordrered transaction list while providing hooks for limited customization.

## Summary of Common Operations

### Adding a Transaction

The most basic task of the transaction pool is to add an incoming transaction to the graph.

The first step is the validation of the transaction itself. This involves the enforcement of all consensus rules surrounding the construction of the transaction itself, and the verification of all relevant signatures and proofs.

The next step is enforcement of node-level transaction acceptability policy. These are generally weaker restrictions governing relay and inclusion that may be adjusted without the need of hard- or soft-forking mechanisms. Additionally, this will include toggles and customizations made by operators or fork maintainers. Bitcoin's "standardness" language is adopted here. 

Note that there are some elements of node-level policy which are not enforced here, for example the maximum size of the pool in memory. 

Next, the state of the transaction and where it would be located in the graph is determined. Each of the transactions' inputs are resolved between the current blockchain UTXO set and the additional set of outputs generated by pool transactions.

## Adversarial Conditions

Under adversarial situations, the primary concerns to the transaction pool are denial-of-service attacks. The greatest concern should be maintaining the ability of the node to provide services to miners, by supplying ready made transactions to the mining service for inclusion in blocks. Resource consumption should be constrained as well. As we've seen on other chains, miners often have little incentive to include transactions if doing so impacts their ability to collect their primary reward.

### 
