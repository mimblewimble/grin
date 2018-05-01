Dandelion Simulation
==================
This document describes a network of node with Dandelion.

In this scenario, we simulate a successfull aggregation but a failed transaction cut-through forcing a node to revert its stempool state.

This document also helps visualizing all the timers in a simple way.

## Initial Situation

![Initial situation](images/ti.png)

## T = 0

A sends grins to B. B adds the transaction to its stempool and starts its patience timer.

![t = 0](images/t0.png)

## T = 10

B waits until he runs out of patience.

![t = 10](images/t10.png)

## T = 30

B runs out of patience, flips a coin, broadcasts the transaction to its stem relay and starts the embargo timer for this transaction.

![t = 30](images/t30.png)

## T = 35

B and H wait.

![t = 35](images/t35.png)

## T = 40

G sends grins to E.
E adds the transaction to its stempool and starts its patience timer.

![t = 40](images/t40.png)

## T = 50

B, H and E wait.

![t = 50](images/t50.png)

## T = 55

B spends B1 to D.
D adds the transaction to its stempool and starts its patience timer.

![t = 55](images/t55.png)

## T = 60

H runs out of patience, flips a coin, broadcasts the transaction to its stem relay and starts the embargo timer for this transaction.

![t = 60](images/t60.png)

## T = 65

Waiting.

![t = 65](images/t65.png)

## T = 70

E runs out of patience, flips a coin, broadcasts the aggregated transaction to its stem relay and starts the embargo timer for this transaction.

![t = 70](images/t70.png)

## T = 75

Waiting.

![t = 75](images/t75.png)

## T = 85

D runs out of patience, flips a coin, broadcasts the aggregated transaction to its stem relay and starts the embargo timer for this transaction.
E receives the stem transaction, aggregates them (thus removing duplicate input/output pair B1) and starts its patience timer.

![t = 85](images/t85.png)

## T = 100

F runs out of patience, flips a coin, broadcasts the aggregated transaction to all its peers (fluff in the mempool).
E receives the transaction in its mempool and reverts the state of its stempool to avoid conflicting transactions.

![t = 100](images/t100.png)
