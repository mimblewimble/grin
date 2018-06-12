Dandelion Simulation
==================
This document describes a network of node with Dandelion.

In this scenario, we simulate a successful aggregation but a failed transaction cut-through forcing a node to revert its stempool state.

This document also helps visualizing all the timers in a simple way.

## T = 0 - Initial Situation

![t = 0](images/t0.png)

## T = 5

A sends grins to B. 
B flips a coin and decides to add it to its stempool and starts the embargo timer for this transaction.

![t = 5](images/t5.png)

## T = 10

B waits until he runs out of patience.

![t = 10](images/t10.png)

## T = 30

B runs out of patience and broadcasts the aggregated stem transaction to its Dandelion relay H.
H receives the stem transaction, flips a coin and decides to add it to its stempool and starts the embargo timer for this transaction.

![t = 30](images/t30.png)

## T = 40

G sends grins to E.
E flips a coin and decides to add it to its stempool and starts the embargo timer for this transaction.
B and H wait.

![t = 40](images/t40.png)

## T = 55

B spends B1 to D.
D flips a coin and decides to add it to its stempool and starts the embargo timer for this transaction.
H runs out of patience broadcasts the aggregated stem transaction to its Dandelion relay E.
E receives the stem transaction, flips a coin and decides to add it to its stempool and starts the embargo timer for this transaction.

![t = 55](images/t55.png)

## T = 65

Nodes are waiting.

![t = 65](images/t65.png)

## T = 70

E runs out of patience broadcasts the aggregated stem transaction to its Dandelion relay F.
F receives the stem transaction, flips a coin and decides to add it to its stempool and starts the embargo timer for this transaction.

![t = 70](images/t70.png)

## T = 80

D runs out of patience, broadcasts the aggregated stem transaction to its Dandelion relay.
E receives the stem transaction, flips a coin and decides to add it to its stempool and starts the embargo timer for this transaction. aggregates them (thus removing duplicate input/output pair B1) and starts its patience timer.

![t = 80](images/t80.png)

## T = 85

Nodes are waiting.

![t = 85](images/t85.png)

## T = 95 - Step 1

F runs out of patience, broadcasts the aggregated stem transaction to its Dandelion relay H.
H receives the transaction, flips a coin and decide to broadcast the transaction to all its peers (fluff in the mempool).

![t = 95_1](images/t95_1.png)

## T = 95 - Step 2

All the nodes add this transaction to their mempool and remove the related transactions from their stempool.
E receives the transaction in its mempool and reverts the state of its stempool to avoid conflicting transactions.

![t = 95_2](images/t95_2.png)
