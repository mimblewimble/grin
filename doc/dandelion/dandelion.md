# Dandelion++ in Grin: Privacy-Preserving Transaction Aggregation and Propagation

*Read this document in other languages: [Korean](dandelion_KR.md). [out of date]*

## Introduction

The Dandelion++ protocol for broadcasting transactions, proposed by Fanti et al. (Sigmetrics 2018)[1], intends to defend against deanonymization attacks during transaction propagation. In Grin, it also provides an opportunity to aggregate transactions before they are broadcasted to the entire network. This document describes the protocol and the simplified version of it that is implemented in Grin.

In the following section, past research on the protocol is summarized. This is then followed by describing details of the Grin implementation; the motivation behind its inclusion, how the current implementation differs from the original paper, what some of the known issues are, and outlining some areas of improvement for future work. The final section concludes with a summary.

## Previous research

The original version of Dandelion was introduced by Fanti et al. and presented at ACM Sigmetrics 2017 [2]. On June 2017, a BIP [3] was proposed introducing a more practical and robust variant of Dandelion called Dandelion++, which was formalized into a paper in 2018. [1] The protocols are outlined at a high level here. For a more in-depth presentation with extensive literature references, please refer to the original papers.

### Problem

Dandelion was conceived as a way to mitigate against large scale deanonymization attacks on the network layer of Bitcoin, made possible by the diffusion method for propagating transactions on the network. By deploying "super-nodes" that connect to a large number of honest nodes on the network, adversaries can listen to the transactions relayed by the honest nodes as they get diffused symmetrically on the network using epidemic flooding or diffusion. By observing the spreading dynamic of a transaction, it has been proven possible to link it (and therefore also the sender's Bitcoin address) to the originating IP address with a high degree of accuracy, and as a result  deanonymize users.

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
1. All nodes obey the protocol;
2. Each node generates exactly one transaction; and
3. All Bitcoin nodes run Dandelion. 

An adversary can violate these rules, and by doing so break some of the anonymity properties. 

The modified Dandelion++ protocol makes small changes to most of the Dandelion choices, resulting in an exponentially more complex information space. This in turn makes it harder for an adversary to  deanonymize the network.

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

The choice between using 4-regular or 2-regular (line) graphs is not obvious. The authors note that it's difficult to construct an exact 4-regular graph within a fully-distributed network in practice. They  outline a method to construct an approximate 4-regular graph in the paper. They also write:

> [...] We recommend making the design decision between 4-regular graphs and line graphs based on the priorities of the system builders. **If linkability of transactions is a first-order concern, then line graphs may be a better choice.** Otherwise, we find that 4-regular graphs can give constant- order privacy benefits against adversaries with knowledge of the graph.

##### 2. Transaction forwarding (own)

At the start of each epoch, `NodeX` picks one of `out1` and `out2` to use as a route to broadcast its own transactions through as a stem-phase transaction. The _same route_ is used throughout the duration epoch, and a node _always_ forwards their own transaction rather than fluffing it directly.

##### 3. Transaction forwarding (relay)

At the start of each epoch, `NodeX` makes a choice to be either in fluff-mode or in stem-mode. This choice is made in pseudorandom fashion, with the paper suggesting it being computed from a hash of the node's own identity and epoch number.

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

### Motivation

### Previous implementation

### Current implementation

### Known limitations

### Future work

* Simulation

## Conclusion

Dandelion++ is a transaction broadcasting mechanism that reduces the risk of eavesdroppers linking transactions to the source IP. Moreover, it allows Grin transactions to be aggregated (removing input-output pairs) before being broadcasted to the entire network giving an additional privacy perk.




### Mechanism



### Specifications

The Dandelion protocol is based on three mechanisms:

1. *Stem/fluff propagation.* Dandelion transactions begin in “stem mode,” during which each node relays the transaction to a single randomly-chosen peer. With some fixed probability, the transaction transitions to “fluff” mode, after which it is relayed according to ordinary flooding/diffusion.

2. *Stem Mempool.* During the stem phase, each stem node (Alice) stores the transaction in a transaction pool containing only stem transactions: the stempool. The content of the stempool is specific to each node and is non shareable. A stem transaction is removed from the stempool if:

    1. Alice receives it "normally" advertising the transaction as being in fluff mode.
    2. Alice receives a block containing this transaction meaning that the transaction was propagated and included in a block.

3. *Robust propagation.* Privacy enhancements should not put transactions at risk of not propagating. To protect against failures (either malicious or accidental) where a stem node fails to relay a transaction (thereby precluding the fluff phase), each node starts a random timer upon receiving a transaction in stem phase. If the node does not receive any transaction message or block for that transaction before the timer expires, then the node diffuses the transaction normally.

Dandelion stem mode transactions are indicated by a new type of relay message type.

Stem transaction relay message type:

```rust
Type::StemTransaction;
```

After receiving a stem transaction, the node flips a biased coin to determine whether to propagate it in “stem mode”, or to switch to “fluff mode.” The bias is controlled by a parameter exposed to the configuration file, initially 90% chance of staying in stem mode (meaning the expected stem length would be 10 hops).

Nodes that receives stem transactions are called stem relays. This relay is chosen from among the outgoing (or whitelisted) connections, which prevents an adversary from easily inserting itself into the stem graph. Each node periodically randomly choose its stem relay every 10 minutes.

### Considerations

The main implementation challenges are: (1) identifying a satisfactory tradeoff between Dandelion’s privacy guarantees and its latency/overhead, and (2) ensuring that privacy cannot be degraded through abuse of existing mechanisms. In particular, the implementation should prevent an attacker from identifying stem nodes without interfering too much with the various existing mechanisms for efficient and DoS-resistant propagation.

* The privacy afforded by Dandelion depends on 3 parameters: the stem probability, the number of outbound peers that can serve as dandelion relay, and the time between re-randomizations of the stem relay. These parameters define a tradeoff between privacy and broadcast latency/processing overhead. Lowering the stem probability harms privacy but helps reduce latency by shortening the mean stem length; based on theory, simulations, and experiments, we have chosen a default of 90%. Reducing the time between each node’s re-randomization of its stem relay reduces the chance of an adversary learning the stem relay for each node, at the expense of increased overhead.
* When receiving a Dandelion stem transaction, we avoid placing that transaction in `tracking_adapter`. This way, transactions can also travel back “up” the stem in the fluff phase.
* Like ordinary transactions, Dandelion stem transactions are only relayed after being successfully accepted to mempool. This ensures that nodes will never be punished for relaying Dandelion stem transactions.
* If a stem orphan transaction is received, it is added to the `orphan` pool, and also marked as stem-mode. If the transaction is later accepted to mempool, then it is relayed as a stem transaction or regular transaction (either stem mode or fluff mode, depending on a coin flip).
* If a node receives a child transaction that depends on one or more currently-embargoed Dandelion transactions, then the transaction is also relayed in stem mode, and the embargo timer is set to the maximum of the embargo times of its parents. This helps ensure that parent transactions enter fluff mode before child transactions. Later on, this two transaction will be aggregated in one unique transaction removing the need for the timer.
* Transaction propagation latency should be minimally affected by opting-in to this privacy feature; in particular, a transaction should never be prevented from propagating at all because of Dandelion. The random timer guarantees that the embargo mechanism is temporary, and every transaction is relayed according to the ordinary diffusion mechanism after some maximum (random) delay on the order of 30-60 seconds.

## Dandelion in Grin

Dandelion also allows Grin transactions to be aggregated during the stem phase and then broadcasted to all the nodes on the network. This result in transaction aggregation and possibly cut-through (thus removing spent outputs) giving a significant privacy gain similar to a non-interactive coinjoin with cut-through. This section details this mechanism.

### Aggregation Mechanism

In order to aggregate transactions, Grin implements a modified version of the Dandelion protocol [4].

By default, when a node sends a transaction on the network it will be broadcasted with the Dandelion protocol as a stem transaction to its Dandelion relay. The Dandelion relay will then wait a period of time (the patience timer), in order to get more stem transactions to aggregate. At the end of the timer, the relay does a coin flip for each new stem transaction and determines if it will stem it (send to the next Dandelion relay) or fluff it (broadcast normally). Then the relay will take all the transactions to stem, aggregate them, and broadcast them to the next Dandelion relay. It will do the same for the transactions to fluff, except that it will broadcast the aggregated transactions “normally” (to a random subset of the peers).

This gives us a P2P protocol that can handle transaction merging.

A simulation of this scenario is available [here](simulation.md).

## References
* [1] (Sigmetrics 2018) [Dandelion++: Lightweight Cryptocurrency Networking with Formal Anonymity Guarantees](https://arxiv.org/abs/1805.11060)
* [2] (Sigmetrics 2017) [Dandelion: Redesigning the Bitcoin Network for Anonymity](https://arxiv.org/abs/1701.04439)
* [3] [Dandelion BIP](https://github.com/dandelion-org/bips/blob/master/bip-dandelion.mediawiki)

* [X] [Dandelion Grin Pull Request #1067](https://github.com/mimblewimble/grin/pull/1067)
