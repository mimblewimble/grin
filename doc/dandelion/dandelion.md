# Dandelion++ in Grin: Privacy-Preserving Transaction Aggregation and Propagation

*Read this document in other languages: [Korean](dandelion_KR.md). [out of date]*

## Introduction

The Dandelion++ protocol for broadcasting transactions, proposed by Fanti et al. (Sigmetrics 2018)[1], intends to defend against deanonymization attacks during transaction propagation. In Grin, it also provides an opportunity to aggregate transactions before they are broadcasted to the entire network. This document describes the protocol and the simplified version of it that is implemented in Grin.

In the following section, past research on the protocol is summarized. This is then followed by describing details of the Grin implementation; the objectives behind its inclusion, how the current implementation differs from the original paper, what some of the known limitations are, and outlining some areas of improvement for future work.

## Previous research

The original version of Dandelion was introduced by Fanti et al. and presented at ACM Sigmetrics 2017 [2]. On June 2017, a BIP [3] was proposed introducing a more practical and robust variant of Dandelion called Dandelion++, which was formalized into a paper in 2018. [1] The protocols are outlined at a high level here. For a more in-depth presentation with extensive literature references, please refer to the original papers.

### Motivation

Dandelion was conceived as a way to mitigate against large scale deanonymization attacks on the network layer of Bitcoin, made possible by the diffusion method for propagating transactions on the network. By deploying "super-nodes" that connect to a large number of honest nodes on the network, adversaries can listen to the transactions relayed by the honest nodes as they get diffused symmetrically on the network using epidemic flooding or diffusion. By observing the spreading dynamic of a transaction, it has been proven possible to link it (and therefore also the sender's Bitcoin address) to the originating IP address with a high degree of accuracy, and as a result deanonymize users.

### Original Dandelion

In the original paper [2], a **dandelion spreading protocol** is introduced. Dandelion spreading propagation consists of two phases: first the anonymity phase, or the **“stem”** phase, and second the spreading phase, or the **“fluff”** phase, as illustrated in Figure 1. 

**Figure 1.** Dandelion phase illustration.

```
                                                   ┌-> F ...
                                           ┌-> D --┤
                                           |       └-> G ...
  A --[stem]--> B --[stem]--> C --[fluff]--┤
                                           |       ┌-> H ...
                                           └-> E --┤
                                                   └-> I ...
```

In the initial **stem-phase**, each node relays the transaction to a *single randomly selected peer*, constructing a line graph. Users then forward transactions along the *same* path on the graph. After a random number of hops along the single stem, the transaction enters the **fluff-phase**, which behaves like ordinary diffusion. This means that even when an attacker can identify the originator of the fluff phase, it becomes more difficult to identify the source of the stem (and thus the original broadcaster of the transaction). The constructed line graph is periodically re-generated randomly, at the expiry of each _epoch_, limiting an adversary's possibility to build knowledge of graph. Epochs are asynchronous, with each individual node keeping its own internal clock and starting a new epoch once a certain threshold has been reached.   

The 'dandelion' name is derived from how the protocol resembles the spreading of the seeds of a dandelion.

### Dandelion++

In the Dandelion++ paper[1], the authors build on the original concept further, by defending against stronger adversaries that are allowed to disobey protocol. 

The original paper makes three idealistic assumptions: 
1. All nodes obey protocol;
2. Each node generates exactly one transaction; and
3. All nodes on the network run Dandelion. 

An adversary can violate these rules, and by doing so break some of the anonymity properties. 

The modified Dandelion++ protocol makes small changes to most of the Dandelion choices, resulting in an exponentially more complex information space. This in turn makes it harder for an adversary to deanonymize the network.

The paper describes five types of attacks, and proposes specific updates to the original Dandelion protocol to mitigate against these, presented in Table A (here in summarized form).

**Table A.** Summary of Dandelion++ changes

| Attack | Solution |
|---|---|
| Graph-learning | 4-regular anonymity graph |
| Intersection | Pseudorandom forwarding |
| Graph-construction | Non-interactive construction |
| Black-hole | Random stem timers |
| Partial deployment | Blind stem selection |

#### The Dandelion++ algorithm

As with the original Dandelion protocol epochs are asynchronous, each node keeping track of its own epoch, which the suggested duration being in the order of 10 minutes.

##### 1. Anonymity Graph
 Rather than a line graph as per the original paper (which is 2-regular), a *quasi-4-regular graph* (Figure 2) is constructed by a node at the beginning of each epoch: the node chooses (up to) two of its outbound edges uniformly at random as its _dandelion++ relays_. As a node enters into a new epoch, new dandelion++ relays are chosen.

**Figure 2.** A 4-regular graph.
```
in1        out1
  \       /
   \     /
    NodeX
   /     \
  /       \
in2        out2
```
*`NodeX` has four connections to other nodes, input nodes `in1` and `in2`, and output nodes `out1` and `out2`.*

***Note on using 4-regular vs 2-regular graphs***

The choice between using 4-regular or 2-regular (line) graphs is not obvious. The authors note that it is difficult to construct an exact 4-regular graph within a fully-distributed network in practice. They outline a method to construct an approximate 4-regular graph in the paper. They also write:

> [...] We recommend making the design decision between 4-regular graphs and line graphs based on the priorities of the system builders. **If linkability of transactions is a first-order concern, then line graphs may be a better choice.** Otherwise, we find that 4-regular graphs can give constant- order privacy benefits against adversaries with knowledge of the graph.

##### 2. Transaction forwarding (own)

At the beginning of each epoch, `NodeX` picks one of `out1` and `out2` to use as a route to broadcast its own transactions through as a stem-phase transaction. The _same route_ is used throughout the duration epoch, and `NodeX` _always_ forwards (stems) its own transaction.

##### 3. Transaction forwarding (relay)

At the start of each epoch, `NodeX` makes a choice to be either in fluff-mode or in stem-mode. This choice is made in pseudorandom fashion, with the paper suggesting it being computed from a hash of the node's own identity and epoch number. The probability of choosing to be in fluff-mode (or as the paper calls it, *the path length parameter `q`*) is recommended to be q ≤ 0.2.

Once the choice has been made whether to stem or to fluff, it applies to *all relayed transactions* during the epoch. 

If `NodeX` is in **fluff-mode**, it will broadcast any received transactions to the network using diffusion.

If `NodeX` is in **stem-mode**, then at the beginning of each epoch it will map `in1` to either `out1` or `out2` pseudorandomly, and similarly map `in2` to either `out1` or `out2` in the same fashion. Based on this mapping, it will then forward *all* txs from `in1` along the chosen route, and similarly forward all transactions from `in2` along that route. The mapping persists throughout the duration of the epoch.

##### 4. Fail-safe mechanism

For each stem-phase transaction that was sent or relayed, `NodeX` tracks whether it is seen again as a fluff-phase transaction within some random amount of time. If not, the node fluffs the transaction itself.

This expiration timer is set by each stem-node upon receiving a transaction to forward, and is chosen randomly. Nodes are initialized with a timeout parameter T<sub>base</sub>. As per equation (7) in the paper, when a stem-node *v* receives a transaction, it sets an expiration time T<sub>out</sub>(v):

T<sub>out</sub>(v) ~ current_time + exp(1/T<sub>base</sub>)

If the transaction is not received again by relay v before the expiry of T<sub>out</sub>(v), it broadcasts the message using diffusion. This approach means that the first stem-node to broadcast is approximately uniformly selected among all stem-nodes who have seen the message, rather than the originating node.

The paper also proceeds to specify the size of the initiating time out parameter T<sub>base</sub> as part of `Proposition 3` in the paper:

> Proposition3. For a timeout parameter
> 
> T<sub>base</sub> ≥ (−k(k−1)δ<sub>hop</sub>) / 2 log(1−ε ),
> 
>  where `k`, `ε` are parameters and δ<sub>hop</sub> is 
the time between each hop (e.g., network and/or internal node latency), transactions travel for `k` hops without any peer initiating diffusion with a probability of at least `1 − ε`.


## Dandelion in Grin

### Objectives

There are two main motives behind why Dandelion is included in Grin:

1. **Act as a countermeasure against mass de-anonymization attacks.** Similar to Bitcoin, the Grin P2P network would be vulnerable to attackers deploying malicious "super-nodes" connecting to most peers on the network and monitoring transactions as they become diffused by their honest peers. This would allow a motivated actor to infer with a high degree of probability from which peer (IP address) transactions originate from, having negative privacy consequences.
2. **Aggregate transactions before they are being broadcasted to the entire network.** This is a benefit to blockchains that enable non-interactive CoinJoins on the protocol level, such as Mimblewimble. Despite its good privacy features, some input and output linking is still possible in Mimblewimble and Grin.[4] If you know which input spends to which output, it is possible to construct a (very limited) transaction graph and follow a chain of transaction outputs (TXOs) as they are being spent. Aggregating transactions make this more difficult to carry out, as it becomes less clear which input spends to which output (Figure 3). In order for this to be effective, there needs to be a large anonymity set, i.e. many transactions to aggregate a transaction with. Dandelion enables this aggregation to occur before transactions are fluffed and diffused to the entire network. This adds obfuscation to the transaction graph, as a malicious observer who is not participating in the stemming or fluffing would not only need to figure out from where a transaction originated, but also which TXOs out of a larger group should be attributed to the originating transaction.

**Figure 3.** Aggregating transactions
```
3.1 Transactions (not aggregated)
---------------------------------------------
TX1     INPUT_A ______________ OUTPUT_X
                        |_____ OUTPUT_Y

                        KERNEL 1                                
---------------------------------------------
TX2     INPUT_B ______________ OUTPUT_Z
        INPUT_C ________|

                        KERNEL 2
---------------------------------------------

3.2 Transactions (aggregated)
---------------------------------------------
TX1+2   INPUT_A ______________ OUTPUT_X
        INPUT_B ________|_____ OUTPUT_Y
        INPUT_C ________|_____ OUTPUT_Z

                        KERNEL 1
                        KERNEL 2
---------------------------------------------
```

### Current implementation

Grin implements a simplified version of the Dandelion++ protocol. It's been improved several times, most recently in version 1.1.0 [5].

1. `DandelionEpoch` tracks a node's current epoch. This is configurable via `epoch_secs` with default epoch set to last for 10 minutes. Epochs are set and tracked by nodes individually.
2. At the beginning of an epoch, the node chooses a single connected peer at random to use as their outbound relay.
3. At the beginning of an epoch, the node makes a decision whether to be in stem mode or in fluff mode. This decision lasts for the duration of the epoch. By default, this is a random choice, with the probability to be in stem mode set to 90%, which implies a fluff mode probability, `q` of 10%. The probability is configurable via `DANDELION_STEM_PROBABILITY`.  The number of expected stem hops a transaction does before arriving to a fluff node is `1/q = 1/0.1 = 10`.
4. Any transactions received from inbound connected nodes or transactions originated from the node itself are first added to the node's `stempool`, which is a list of stem transactions, that each node keeps track of individually. Transactions are  removed from the stempool if: 
   * The node fluffs the transaction itself.
   * The node sees the transaction in question propagated through regular diffusion, i.e. from a different peer having "fluffed" it.
   * The node receives a block containing this transaction, meaning that the transaction was propagated and included in a block.
5. For each transaction added to the stempool, the node sets an *embargo timer*. This is set by default to 180 seconds, and is configurable via `DANDELION_EMBARGO_SECS`.
6. Regardless of whether the node is in fluff or stem mode, any transactions generated from the node itself are forwarded onwards to their relay node as a stem transaction.[6]
7. A `dandelion_monitor` runs every 10 seconds and handles tasks.
8. If the node is in **stem mode**, then:
   1. After being added to the stempool, received stem transactions are forwarded onto the their relay node as a stem transaction. 
   2. As peers connect at random, it is possible they create a circular loop of connected stem mode nodes (i.e. `A -> B -> C -> A`). Therefore, if a node receives a stem transaction from an inbound node that already exists in its own stempool, it will fluff it, broadcasting it using regular diffusion.
   3. `dandelion_monitor` checks for transactions in the node's stempool with an expired embargo timer, and broadcast those individually.
9. If the node is in **fluff mode**, then:
   1. Transactions received from inbound nodes are kept in the stempool. 
   2. `dandelion_monitor` checks in the stempool whether any  transactions are older than 30 seconds (configurable as `DANDELION_AGGREGATION_SECS`). If so, these are aggregated and then fluffed. Otherwise no action is taken, allowing for more stem transactions to aggregate in the stempool in time for the next triggering of `dandelion_monitor`.   
   3. At the expiry of an epoch, all stem transactions remaining in the stem pool are aggregated and fluffed.    

### Known limitations

* 2-regular graphs are used rather than 4-regular graphs as proposed by the paper. It's not clear what impact this has, the paper suggests a trade-off between general linkability of transactions and protection against adversaries who know the entire network graph.
* Unlike the Dandelion++ paper, the embargo timer is by default identical across all nodes. This means that during a black-hole attack where a malicious node withholds transactions, the node most likely to have its embargo timer expire and fluff the transaction will be the originating node, therefore exposing itself.

### Future work

* Randomized embargo timer according to the recommendations of the paper to make it more random which node fluffs an expired transaction.
* Evaluation of whether 4-regular graphs are preferred over 2-regular line graphs.
* Simulation of the current implementation to understand performance.
* Improved understanding of the benefits of transaction aggregation prior to fluffing.

## References
* [1] (Sigmetrics 2018) [Dandelion++: Lightweight Cryptocurrency Networking with Formal Anonymity Guarantees](https://arxiv.org/abs/1805.11060)
* [2] (Sigmetrics 2017) [Dandelion: Redesigning the Bitcoin Network for Anonymity](https://arxiv.org/abs/1701.04439)
* [3] [Dandelion BIP](https://github.com/dandelion-org/bips/blob/master/bip-dandelion.mediawiki)
* [4] [Grin Privacy Primer](https://github.com/mimblewimble/docs/wiki/Grin-Privacy-Primer)
* [5] [#2628: Dandelion++ Rewrite](https://github.com/mimblewimble/grin/pull/2628)
* [6] [#2876: Always stem local txs if configured that way (unless explicitly fluffed)](https://github.com/mimblewimble/grin/pull/2876)