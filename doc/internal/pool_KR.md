# Transaction Pool

This document describes some of the basic functionality and requirements of grin's transaction pool.

## Overview of Required Capabilities

The primary purpose of the memory pool is to maintain a list of mineable transactions to be supplied to the miner service while building new blocks. The design will center around ensuring correct behavior here, especially around tricky conditions like head switching.

For standard (non-mining) nodes, the primary purpose of the memory pool is to serve as a moderator for transaction broadcasts by requiring connectivity to the blockchain. Secondary uses include monitoring incoming transactions, for example for giving early notice of an unconfirmed transaction to the user's wallet.

Given the focus of grin (and mimblewimble) on reduced resource consumption, the memory pool should be an optional but recommended component for non-mining nodes.

메모리 풀의 주된 목적은 새로운 블록을 빌드하는 동안 마이너 서비스에 제공 할 수있는 트랜잭션의 목록을 유지하는 것입니다. 이 디자인은 특히 헤드 전환과 같은 까다로운 조건을 중심으로 올바른 동작을 보장하는 데 중점을 둘 것입니다.

표준 (비 마이닝) 노드의 경우 메모리 풀의 주요 목적은 블록 체인에 대한 연결성을 요구하여 트랜잭션 브로드 캐스트의 중재자 역할을하는 것입니다. 2 차 용도로는 예를 들어 사용자의 지갑에 미확인 트랜잭션을 일찍 알리는 것과 같이 들어오는 트랜잭션 모니터링이 있습니다.

자원 소비 감소에 대한 미소의 초점을 고려할 때 메모리 풀은 선택 사항이지만 비 광산 노드에 권장되는 구성 요소 여야합니다.

## Design Overview

The primary structure of the transaction pool is a pair of Directed Acyclic Graphs. Since each transaction is rooted directly by its inputs in a non-cyclic way, this structure naturally encompasses the directionality of the chains of unconfirmed transactions. Defining this structure has a few other nice properties: descendent invalidation (when a conflicting transaction is accepted for a given input) is nearly free, and the mineability of a given transaction is clearly depicted in its location in the hierarchy.

Another, non-obvious reason for the choice of a DAG is that the acyclic nature of transactions is a necessary property but must be explicitly verified in a way that is not true of other UTXO-based cryptocurrencies. Consider the following loop of single-input single-output transactions in BTC:

트랜잭션 풀의 기본 구조는 한 세트의 Directed Acyclic Graph입니다. 각 거래는 비 주기적 방식으로 입력에 의해 직접적으로 루팅되기 때문에이 구조는 자연적으로 미확인 거래 체인의 방향성을 포함합니다. 이 구조체를 정의하는 데는 몇 가지 좋은 속성이 있습니다. 상속 무효화 (충돌하는 트랜잭션이 주어진 입력에 대해 허용되는 경우)는 거의 무료이며 주어진 트랜잭션의 유효성이 계층 구조의 해당 위치에 명확하게 표시됩니다.

DAG의 선택에 대한 명백하지 않은 또 다른 이유는 트랜잭션의 비주기적인 특성이 필수 속성이지만 다른 UTXO 기반 크립토 통화에 맞지 않는 방식으로 명시 적으로 검증되어야한다는 것입니다. BTC에서 다음과 같은 단일 입력 단일 출력 트랜잭션 루프를 고려하십시오.

A->B->C->A

Because each input in Bitcoin specifically references the hash and output index of the output in a preceding transaction, for a loop to exist, a transaction must reference (and know the hash of) a transaction that does not yet exist (C, in the trivial example.) Furthermore, the hash and output index pair (called an "outpoint" in Bitcoin) is covered by the transaction hash of A, such that any change to either causes the hash of A to change. Therefore, attempting to build such a loop by amending A with the proper outpoint in C after C has been built causes A's hash to change, invalidating B, and so forth.

In grin, an input references an output by the output's own hash. Thus, the backreference does not include the situation the output was generated in, which allows (from a purely mechanical point of view) the creation of a loop without the ability to generate a specific hash from a tightly constrained preimage.

The pair of graphs represents the connected graph and the orphans graph. (While it is possible to represent both groups of transactions in a single graph, it makes determination of orphan status of a given transaction non-trivial, requiring either the maintenance of a flag or traversal upwards of potentially many inputs.)

A transaction reference in the pool has parents, one for each input. The parents fall into one of four states:

Bitcoin의 각 입력은 앞의 트랜잭션에서 출력의 해시 및 출력 인덱스를 특별히 참조하기 때문에 루프가 존재하려면 트랜잭션이 아직 존재하지 않는 트랜잭션을 참조하고 해시를 알고 있어야합니다 (C, 사소한 경우 또한 해시 및 출력 인덱스 쌍 (Bitcoin에서 "아웃 포인트"라고 함)은 A의 트랜잭션 해시로 처리되므로 A의 해시가 변경되어 A의 해시가 변경됩니다. 따라서 C가 빌드 된 후 A에서 올바른 아웃 포인트로 A를 수정하여 루프를 작성하려고하면 A의 해시가 변경되고 B가 무효화됩니다.

웃음, 입력은 출력 자신의 해쉬에 의해 출력을 참조합니다. 따라서 역 참조는 출력이 생성 된 상황을 포함하지 않으므로 엄격한 제한된 사전 이미지에서 특정 해시를 생성 할 수있는 기능이없는 루프를 만들 수 있습니다 (순수 기계적 관점).

한 쌍의 그래프는 연결된 그래프와 고아 그래프를 나타냅니다. (두 그래프를 하나의 그래프로 표현하는 것은 가능하지만 주어진 트랜잭션의 고아 상태를 결정하는 것은 중요하다. 플래그를 유지하거나 잠재적으로 많은 입력을 순회해야한다.)

풀의 트랜잭션 참조에는 각 입력에 대해 하나씩 부모가 있습니다. 부모는 다음 네 가지 상태 중 하나에 속합니다.

* Unknown
* Blockchain transaction
* Pool transaction
* Orphan transaction

A mineable transaction is defined as a transaction which has met all of its locktime requirements and which all parents are either blockchain transactions are mineable pool transactions. One such requirement is the maturity requirement for spending newly generated coins. This will also include the explicit per-transaction locktime, if adopted.

광산 가능한 트랜잭션은 모든 잠금 시간 요구 사항을 충족하고 모든 부모가 블록 체인 트랜잭션 중 하나 인 트랜잭션으로 정의됩니다. 그러한 요구 사항 중 하나는 새로 생성 된 동전을 소비하는 성숙 요건입니다. 채택 된 경우 명시 적 트랜잭션 별 잠금 시간도 포함됩니다.

## Transaction Selection

In terms of needs, preference should be given to older transactions; beyond this, it seems beneficial to target transactions that reduce the maximum depth of the transaction graph, as this reduces the computational complexity of traversing the graph and making changes to it. Since fees are largely static, there is no need for fee preference.

Kahn's algorithm with the parameters above to break ties could provide a efficient mechanism for producing a correctly ordered transaction list while providing hooks for limited customization.

필요에 따라 구형 거래가 우선되어야한다. 이 외에도 트랜잭션 그래프의 최대 깊이를 줄이는 트랜잭션을 대상으로 지정하는 것이 좋습니다. 이렇게하면 그래프를 가로 지르고 변경하는 계산상의 복잡성이 줄어들 기 때문에 트랜잭션 그래프의 최대 깊이를 줄이는 트랜잭션을 대상으로 지정하는 것이 좋습니다. 수수료는 대부분 정체 적이기 때문에 수수료 선호가 필요 없습니다.

위의 매개 변수가있는 Kahn의 알고리즘은 제한된 사용자 정의를 위해 후크를 제공하면서 올바르게 정렬 된 트랜잭션 목록을 생성하는 효율적인 메커니즘을 제공 할 수 있습니다.

## Summary of Common Operations

### Adding a Transaction

The most basic task of the transaction pool is to add an incoming transaction to the graph.

The first step is the validation of the transaction itself. This involves the enforcement of all consensus rules surrounding the construction of the transaction itself, and the verification of all relevant signatures and proofs.

The next step is enforcement of node-level transaction acceptability policy. These are generally weaker restrictions governing relay and inclusion that may be adjusted without the need of hard- or soft-forking mechanisms. Additionally, this will include toggles and customizations made by operators or fork maintainers. Bitcoin's "standardness" language is adopted here.

Note that there are some elements of node-level policy which are not enforced here, for example the maximum size of the pool in memory.

Next, the state of the transaction and where it would be located in the graph is determined. Each of the transactions' inputs are resolved between the current blockchain UTXO set and the additional set of outputs generated by pool transactions.

트랜잭션 풀의 가장 기본적인 작업은 들어오는 트랜잭션을 그래프에 추가하는 것입니다.

첫 번째 단계는 트랜잭션 자체의 유효성 검사입니다. 여기에는 거래 자체의 구성과 모든 관련 서명 및 증명의 검증을 둘러싼 모든 합의 규칙의 집행이 포함됩니다.

다음 단계는 노드 레벨 트랜잭션 허용 정책의 시행입니다. 이들은 일반적으로 하드 또는 소프트 포크 (soft-forking) 메커니즘없이 조정될 수있는 릴레이 및 포함을 관리하는 약한 제한 사항입니다. 또한 운영자 또는 포크 유지 관리자가 수행 한 토글 및 사용자 지정이 포함됩니다. 여기 Bitcoin의 "표준"언어가 채택되었습니다.

여기서 시행되지 않는 노드 레벨 정책의 일부 요소 (예 : 메모리 풀의 최대 크기)가 있음에 유의하십시오.

그런 다음 트랜잭션의 상태와 그래프에있는 트랜잭션의 위치가 결정됩니다. 각 트랜잭션의 입력은 현재 블록 체인 UTXO 세트와 풀 트랜잭션에 의해 생성 된 추가 출력 세트 사이에서 해결됩니다.

## Adversarial Conditions

Under adversarial situations, the primary concerns to the transaction pool are denial-of-service attacks. The greatest concern should be maintaining the ability of the node to provide services to miners, by supplying ready made transactions to the mining service for inclusion in blocks. Resource consumption should be constrained as well. As we've seen on other chains, miners often have little incentive to include transactions if doing so impacts their ability to collect their primary reward.

적대적인 상황에서 트랜잭션 풀의 주된 관심사는 서비스 거부 (denial-of-service) 공격입니다. 가장 큰 관심사는 블록에 포함시키기 위해 준비된 거래를 광업 서비스에 제공함으로써 노드가 광부에게 서비스를 제공 할 수있는 능력을 유지하는 것입니다. 리소스 소비도 제한되어야합니다. 다른 사슬에서 보았 듯이, 광부들은 거래가 주요 보상금을 모을 수있는 능력에 영향을 미친다면 거래를 포함 할 인센티브가 거의 없습니다.
