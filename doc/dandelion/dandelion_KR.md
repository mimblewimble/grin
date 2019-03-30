# Grin에서 사용하는 Dandelion : 프라이버시 보호 트랜잭션의 통합과 전파

*역자 주 : Dandelion 은 민들레라는 뜻으로 앞으로 설명될 각종 용어에서 민들레의 홑씨와 관련된 용어들이 있으니 참고바람.*

이 문서에서는 Grin에서 구현된 Dandelion 구현체에 대해서 설명하고 그리고 Dandelion이 P2P 프로토콜내에서 트랜잭션 통합을 다루기 위해 어떻게 수정되었는지 설명합니다.

## 소개

Dandelion은 발신 IP에 도청 연결 트랜잭션(eavesdroppers linking transactions)의 위험을 줄이는 새로운 트랜잭션 전파 메커니즘입니다. 또한 전체 네트워크에 퍼지기 전에 추가적인 프라이버시 보호 기능을 제공합니다. 프라이버시 보호 기능은 Grin 트랜잭션을 입력-출력 쌍을 제거하는 방식으로 제공됩니다.

Dandelion은 G. Fanti 에 의해 소개되었습니다([1] 참고). 그리고 ACM Sigmetrics 2017에서 발표되었습니다. 2017 년 6 월 BIP [2]는 2018 년 후반에 발표될 Dandelion ++ [3]이라고 불리는 더 실용적이고 견고한 Dandelion의 다른 버전을 소개하려고 제안되었습니다. 이 문서는 Grin을 위한 BIP의 개정판이라고 생각하시면 됩니다.

우선은 오리지널 Dandelion 전파에 대해서 정의한 다음, 트랜잭션 에그레게이션 (transaction aggregation)과 함께 어떻게 Grin의 프로토콜에 적용되었는지 알아보겠습니다.

## Original Dandelion

### 매커니즘에 대해서

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

2. *Stem Mempool.* Stem 페이즈 동안, 각 Stem node(앨리스)는 transaction pool에 stem transaction만을 포함한 transaction을 저장합니다. 이것이 stem pool 입니다. stem pool의 내용은 각 노드마다 다르므로 공유 할 수 없습니다. 다음 경우에 stem pool에서 stem transaction이 제거됩니다.
   
    1.  앨리스는 fluff 모드인 transaction을 받았다고 "정상적으로" 알립니다.
    2. Alice는 이 트랜잭션이 포함 된 블록을 받았고 이러한 것은 이 트랜잭션이 전파되고 블록에 포함되었음을 의미합니다.

3. *Robust propagation.* 프라이버시 강화가 transaction을 전파 하지 않게 해서는 안됩니다. stem node가 transaction을 중계(relay)하지 못해서 fluff 단계에 가지 못할 상황일때 (악의적이거나 우발적인) 실패를 방지하기 위해 각 노드는 stem 페이즈에서 transaction을 수신 할 때 임의의 타이머를 시작합니다. 타이머가 만료되기 전에 노드가 해당 transaction에 대한 transaction 메시지 또는 블록을 수신하지 않으면 노드는 transaction을 정상적으로 전파합니다.

Dandelion stem mode transaction은 새로운 타입의 릴레이 메시지 타입으로 표시됩니다.

Stem transaction relay 메시지 타입은 아래와 같습니다.

```rust
Type::StemTransaction;
```
Stem transaction을 수신한 후 node는 바이어스(biased) 된 코인을 뒤집어서 "stem mode"로 전달할지 또는 "fluff mode"로 전환할지 결정합니다. 바이어스는 설정파일에 있는 파라미터에 의해 제어되는데, 초기에는 90 % 확률로 stem mode로 유지하게 합니다.(예상되는 stem 길이가 10번 건너뜀(hops)이 될 것임을 의미함)

Stem transacion을 수신하는 노드를 stem relay라고합니다. 이 릴레이는 밖으로 향하거나 connection 이나 허가된 연결 중에서 선택 되어지는데 침입자들이 stem graph에 쉽게 잡입하는 것을 방지합니다. 각 노드는 매 10 분마다 주기적으로 무작위로 stem relay를 선택합니다.

### 고려해야 하는 것들

주요 구현의 도전 과제는 (1) Dandelion의 프라이버시를 보장하는 것 과 latency/overhead 사이의 만족스러운 트레이드 오프를 밝히는 것, (2) 기존 매커니즘의 남용을 통해 프라이버시가 질적으로 저하 될 수 없다는 것을 보장하는 것입니다.
특히 구현에서는 효율적이고 DoS 내성 전파를 위한 다양한 기존 메커니즘을 너무 많이 방해하지 않고 공격자가 stem node를 식별하는 것을 방지해야합니다.

* Dandelion이 제공하는 프라이버시는 다음과 같은 3가지 파라미터에 달려있습니다. Stem 확률(stem probability), Dandelion 중계(relay)역할을 할 수 있는 아웃바운드(outbound) 피어 수 그리고 stem 중계(relay)의 재-무작위 추출 간의 시간입니다. 이러한 파라미터는 프라이버시와 전파 latency/processing overhead 간의 트레이드 오프를 정의합니다.
Stem 확률(stem probability)을 낮추면 프라이버시가 손상되지만 평균 stem 길이를 짧게 해서 latency를 줄이는 데 도움이 됩니다.(그래서) 이론,시뮬레이션 및 실험을 기준으로 stem 확률은 90%를 기본값으로 선택 되었습니다. 각 노드의 stem 중계(relay)의 재-무작위 추출 사이의 시간을 줄이면 공격자가 각 노드의 stem 중계(relay)를 알 기회가 줄어들고 overhead가 증가합니다.
* Dandelion stem transaction을 받을 때, 그 transaction을 `tracking_adapter`에 두는 것을 피합니다. 이렇게 하면 Transaction이 fluff 페이즈에서 Stem을 "위로" 이동할 수도 있습니다.
* 일반 거래와 마찬가지로, Dandelion stem transaction은 mempool에 성공적으로 승인 된 후에 만 ​​중계(relay)됩니다. 이렇게 하면  Dandelion stem transaction을 중계(relay) 할 때 노드가 절대로 불이익을 받지 않습니다.
* Stem orphan transaction이 접수되면 `고아 (orphan)`pool에 추가되고 stem mode로 표시됩니다. transaction이 나중에 mempool에 승인되면 stem transaction 또는 일반 transaction (stem mode 또는 fluff mode, 동전 반전에 따라 다름)으로 중계(relay)됩니다.
* 노드가 하나나 또는 그 이상의 현재 엠바고 상태인 Dandelion transaction에 의존하는 하위 거래(child transaction)를 받으면 transaction도 stem mode로 중계(relay)되고 엠바고 타이머는 상위 트랜잭션 (parent)의 엠바고 기간의 최대치로 설정됩니다. 이렇게 하면 상위 트랜잭션이 하위 트랜잭션 전에 fluff 모드로 들어갈 수 있습니다. 나중에 이 두 transaction은 유니크한 하나의 transaction으로 통합되어 타이머가 필요하지 않게됩니다.
* 트랜잭션 전파 latency는 프라이버시 기능을 넣어도 최소한으로 영향을 받아야 합니다. 특히 Dandelion 때문에 거래가 전혀 방해받지 않아야 합니다. 무작위 타이머는 엠바고 메커니즘이 일시적이며 일반 확산 메커니즘에 따라 모든 트랜잭션이 최대 (임의로) 30-60 초 정도의 딜레이 후에 중계(relay)되도록 보장합니다.

## Grin 에서의 Dandelion

Dandelion은 또한 Grin Transaction을 Stem phase에서 통합(Aggregated) 한 다음 네트워크의 모든 노드에 전파 할 수 있습니다. 이로 인해 트랜잭션 통합(transaction aggregation)과 컷 스루(cut-through, 소비 된 출력값 삭제)가 가능해져 컷 스루(cut-through) 방식의 non-interactive coinjoin 과 유사한 매우 중요한 프라이버시 이득을 얻을 수 있습니다. 

### 통합 매커니즘 (Aggregation Mechanism)

트랜잭션을 통합(aggregate)하기 위해 Grin은 Dandelion 프로토콜의 수정 된 버전을 구현합니다 [4].

기본적으로 노드가 네트워크에서 transaction을 전송하면 Dandelion 프로토콜을 사용하여 Dandelion 중계기(relay)에 stem transaction으로 전파됩니다. Dandelion 중계기(relay)는 더 많은 stem transaction를 통합하기 위해 일정 기간 (patience 타이머) 대기합니다. 타이머가 끝날때 중계기(relay)는 새로운 stem transaction마다 코인 플립을 해서 stem을 하거나 (다음 Dandelion relay로 보내거나) fluff(정상적으로 전파하거나) 할지를 결정합니다. 그런 다음 relay는 모든 transaction을 stem으로 가져 와서 통합하여 다음 Dandelion 릴레이에 전파합니다. Transaction이 "정상적으로" 피어의 무작위 하위 집합으로 통합된 Transaction을 전파한다는 점을 제외하고는 fluff(단계)로 가는 transaction에 대해 동일한 작업을 수행합니다.
이 매커니즘은 transaction 병합을 다룰수 있는 P2P protocol을 제공합니다.

이 시나리오에 대한 시뮬레이션은 [여기](simulation_KR.md)에서 확인 할 수 있습니다.

## 레퍼런스

* [1] (Sigmetrics 2017) [Dandelion: Redesigning the Bitcoin Network for Anonymity](https://arxiv.org/abs/1701.04439)
* [2] [Dandelion BIP](https://github.com/dandelion-org/bips/blob/master/bip-dandelion.mediawiki)
* [3] (Sigmetrics 2018) [Dandelion++: Lightweight Cryptocurrency Networking with Formal Anonymity Guarantees](https://arxiv.org/abs/1805.11060)
* [4] [Dandelion Grin Pull Request #1067](https://github.com/mimblewimble/grin/pull/1067)
