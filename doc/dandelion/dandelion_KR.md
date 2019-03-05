# Grin에서 사용하는 Dandelion : 프라이버시 보호 트랜잭션의 집합과 전파

*역자 주 : Dandelion 은 민들레라는 뜻으로 앞으로 설명될 각종 용어에서 민들레의 홑씨와 관련된 용어들이 있으니 참고바람.*

이 문서에서는 Grin에서 구현된 Dandelion 구현체에 대해서 설명하고 그리고 Dandelion이 P2P 프로토콜내에서 트랜잭션 집합을 다루기 위해 어떻게 수정되었는지 설명합니다

## 소개

Dandelion is a new transaction broadcasting mechanism that reduces the risk of eavesdroppers linking transactions to the source IP. Moreover, it allows Grin transactions to be aggregated (removing input-output pairs) before being broadcasted to the entire network giving an additional privacy perk.

Dandelion은 발신 IP에 도청 연결 트랜잭션(eavesdroppers linking transactions)의 위험을 줄이는 새로운 트랜잭션 브로드캐스팅 메커니즘입니다. 또한 전체 네트워크에 퍼지기 전에 추가적인 프라이버시 보호 기능을 제공합니다. 프라이버시 보호 기능은 Grin 트랜잭션을 입력-출력 쌍을 제거하는 방식으로 제공됩니다.

Dandelion은 G. Fanti 에 의해 소개되었습니다([1] 참고). 그리고 ACM Sigmetrics 2017에서 발표되었습니다. 2017 년 6 월 BIP [2]는 2018 년 후반에 발표될 Dandelion ++ [3]이라고 불리는 더 실용적이고 견고한 Dandelion의 다른 버전을 소개하려고 제안되었습니다. 이 문서는 Grin을 위한 BIP의 개정판이라고 생각하시면 됩니다.

우선은 오리지널 Dandelion 전파에 대해서 정의한 다음, 트랜잭션 에그레게이션 (transaction aggregation)과 함께 어떻게 Grin의 프로토콜에 적용되었는지 알아보겠습니다.

## Original Dandelion

### 매커니즘에 대해서

Dandelion transaction propagation proceeds in two phases: first the “stem” phase, and then “fluff” phase. During the stem phase, each node relays the transaction to a *single* peer. After a random number of hops along the stem, the transaction enters the fluff phase, which behaves just like ordinary flooding/diffusion. Even when an attacker can identify the location of the fluff phase, it is much more difficult to identify the source of the stem.

Dandelion 트랜잭션 전파는 두 가지 단계로 진행됩니다. 첫 번째는 "stem" (줄기)페이즈, "fluff"(솜털/보풀, 민들레 홀씨를 연상하면 될 듯 - 역자 주)페이즈 입니다. Stem 페이즈에서 각 노드는 트랜잭션을 *단일* 피어로 릴레이 합니다. Stem를 따라 임의의 수로 (노드를) 몇번 건너뛴 후 (hops) 후 트랜잭션은 일반적인 플러딩 / 확산과 같이 동작하는 fluff 페이즈에 들어갑니다. 공격자가 fluff 단계의 위치를 ​​식별 할 수 있더라도 Stem 페이즈의 발신처를 식별하는 것이 훨씬 더 어렵습니다.

그림 설명:

```
                                                   ┌-> F ...
                                           ┌-> D --┤
                                           |       └-> G ...
  A --[stem]--> B --[stem]--> C --[fluff]--┤
                                           |       ┌-> H ...
                                           └-> E --┤
                                                   └-> I ...
```

### Specifications

Dandelion 프로토콜은 아래와 같은 세가지 매커니즘을 기반으로 합니다.

1. *Stem/fluff 전파.*
    Dandelion 트랜잭션은 "Stem(줄기) 모드"에서 시작됩니다. 각 노드는 거래를 무작위로 선택된 하나의 노드에게 중계합니다. 고정 된 확률로 트랜잭션은 "fluff(솜털)" 모드로 전환되고 이후에는 일반적인 플러딩 / 확산에 따라 릴레이됩니다.

2. *Stem Mempool.* During the stem phase, each stem node (Alice) stores the transaction in a transaction pool containing only stem transactions: the stempool. The content of the stempool is specific to each node and is non shareable. A stem transaction is removed from the stempool if:
   =
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

* [1] (Sigmetrics 2017) [Dandelion: Redesigning the Bitcoin Network for Anonymity](https://arxiv.org/abs/1701.04439)
* [2] [Dandelion BIP](https://github.com/dandelion-org/bips/blob/master/bip-dandelion.mediawiki)
* [3] (Sigmetrics 2018) [Dandelion++: Lightweight Cryptocurrency Networking with Formal Anonymity Guarantees](https://arxiv.org/abs/1805.11060)
* [4] [Dandelion Grin Pull Request #1067](https://github.com/mimblewimble/grin/pull/1067)
