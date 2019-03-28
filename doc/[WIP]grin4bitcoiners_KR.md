# Bitcoiner를 위한 Grin/MimbleWimble 

## 프라이버시와 대체가능성(Fungibility) 

There are 3 main properties of Grin transactions that make them private:

1. There are no addresses.
2. There are no amounts.
3. 2 transactions, one spending the other, can be merged in a block to form only one, removing all intermediary information.

The 2 first properties mean that all transactions are indistinguishable from one another. Unless you directly participated in the transaction, all inputs and outputs look like random pieces of data (in lingo, they're all random curve points).

Moreover, there are no more transactions in a block. A Grin block looks just like one giant transaction and all original association between inputs and outputs is lost.

Grin 트랜잭션에는 3 가지 주요 속성이 있습니다.

1. 주소가 없습니다.
2. 금액은 없습니다.
3. 하나는 다른 트랜잭션을 사용하는 2 개의 트랜잭션을 하나의 블록으로 병합하여 모든 중간 정보를 제거 할 수 있습니다.

두 가지 첫 번째 속성은 모든 트랜잭션을 서로 구별 할 수 없음을 의미합니다. 거래에 직접 참여하지 않는 한 모든 입력과 출력은 임의의 데이터 조각처럼 보입니다 (용어로는 모두 임의의 곡선 점입니다).

또한 블록에 더 이상 트랜잭션이 없습니다. Grin 블록은 마치 하나의 거대한 거래처럼 보이고 입력과 출력 사이의 모든 원래 연결이 손실됩니다.

## 확장성(Scalability)

As explained in the previous section, thanks to the MimbleWimble transaction and block format we can merge transactions when an output is directly spent by the input of another. It's as if when Alice gives money to Bob, and then Bob gives it all to Carol, Bob was never involved and his transaction is actually never even seen on the blockchain.

Pushing that further, between blocks, most outputs end up being spent sooner or later by another input. So *all spent outputs can be safely removed*. And the whole blockchain can be stored, downloaded and fully verified in just a few gigabytes or less (assuming a number of transactions similar to bitcoin).

This means that the Grin blockchain scales with the number of users (unspent outputs), not the number of transactions. At the moment, there is one caveat to that: a small piece of data (called a *kernel*, about 100 bytes) needs to stay around for each transaction. But we're working on optimizing that as well.

이전 섹션에서 설명한 것처럼 MimbleWimble 트랜잭션 및 블록 형식 덕분에 출력이 다른 트랜잭션의 입력에 의해 직접 소비 될 때 트랜잭션을 병합 할 수 있습니다. 앨리스가 밥에게 돈을주고 밥이 캐럴에게 돈을 주면 밥은 결코 개입하지 않았고 실제로 거래는 블록 체인에서 실제로 보지 못했습니다.


더 많은 것을 블록 사이에서 밀어 넣으면 대부분의 출력이 다른 입력에 의해 조만간 소비됩니다. 따라서 * 모든 소비 지출을 안전하게 제거 할 수 있습니다 *. 그리고 전체 블록 체인을 저장하고, 다운로드하고, 몇 기가 바이트 (bitcoin과 유사한 트랜잭션의 수를 가정 할 때) 이하로 완벽하게 검증 할 수 있습니다.


즉, Grin 블록 체인은 트랜잭션 수가 아닌 사용자 수 (사용되지 않은 출력)에 따라 확장됩니다. 현재, 하나의주의 사항이 있습니다. 작은 데이터 조각 (약 100 바이트)은 각 트랜잭션마다 머무를 필요가 있습니다. 그러나 우리는이를 최적화하기 위해 노력하고 있습니다.

## 스크립팅(Scripting)

Maybe you've heard that MimbleWimble doesn't support scripts. And in some way, that's true. But thanks to cryptographic trickery, many contracts that in Bitcoin would require a script can be achieved with Grin using properties of Elliptic Curve Cryptography. So far, we know how to do:

* Multi-signature transactions.
* Atomic swaps.
* Time-locked transactions and outputs.
* Lightning Network

아마도 MimbleWimble은 스크립트를 지원하지 않는다는 말을 들었을 것입니다. 그리고 어떤면에서는 사실입니다. 그러나 암호 기법 덕분에 Bitcoin에서 스크립트를 필요로하는 많은 계약은 Elliptic Curve Cryptography의 속성을 사용하여 Grin으로 달성 할 수 있습니다. 지금까지 우리가하는 방법을 알고 있습니다 :

* 다중 서명 거래.
* 원자 교환.
* 시간 잠금 트랜잭션 및 출력.
* 번개 네트워크

## 블록 보상 주기와 블록 비율

Bitcoin's 10 minute block time has its initial 50 btc reward cut in half every 4 years until there are 21 million bitcoin in circulation. Grin's emission rate is linear, meaning it never drops. The block reward is currently set at 60 grin with a block goal of 60 seconds. This still works because 1) dilution trends toward zero and 2) a non-negligible amount of coins gets lost or destroyed every year.

비트 코인 (Bitcoin)의 10 분 블록 타임 (block block time)은 2 천 1 백만 비트 코르크가 유통 될 때까지 4 년마다 반으로 상응하는 초기 50 btc 보상을 제공합니다. Grin의 배출율은 선형 적이기 때문에 결코 떨어지지 않습니다. 블록 보상은 현재 60 초의 블로킹 목표로 60 회 웃음으로 설정됩니다. 이것은 여전히 ​​효과가 있습니다. 1) 희석화가 0에 가까워지고 2) 무시할 수 없을 정도로 적은 양의 동전이 매년 분실되거나 파괴됩니다.


## FAQ

### 잠시만요 뭐라구요? 주소가 없다구요?

Nope, no address. All outputs in Grin are unique and have no common data with any previous output. Instead of relying on a known address to send money, transactions have to be built interactively, with two (or more) wallets exchanging data with one another. This interaction **does not require both parties to be online at the same time**. Practically speaking, there are many ways for two programs to interact privately and securely. This interaction could even take place over email or Signal (or carrier pigeons).

아니, 주소가 없어. Grin의 모든 출력은 고유하며 이전 출력과 공통된 데이터가 없습니다. 알려진 주소를 사용하여 돈을 송금하는 대신 두 곳 이상의 지갑이 서로 데이터를 교환하면서 대화식으로 거래를 구축해야합니다. 이 상호 작용 **은 양 당사자가 동시에 온라인 상태 일 것을 요구하지 않습니다 **. 실질적으로 두 프로그램이 개인적으로 안전하게 상호 작용할 수있는 방법은 다양합니다. 이 상호 작용은 이메일 또는 신호 (또는 운송 업체 비둘기)를 통해 일어날 수도 있습니다.

### If transaction information gets removed, can't I just cheat and create money?

No, and this is where MimbleWimble and Grin shine. Confidential transactions are a form of [homomorphic encryption](https://en.wikipedia.org/wiki/Homomorphic_encryption). Without revealing any amount, Grin can verify that the sum of all transaction inputs equal the sum of transaction outputs, plus the fee. Going even further, comparing the sum of all money created by mining with the total sum of money that's being held, Grin nodes can check the correctness of the total money supply.

아니요, 여기는 MimbleWimble과 Grin이 빛나는 곳입니다. 기밀 거래는 [동형 (homomorphic) 암호화 (https://en.wikipedia.org/wiki/Homomo phic_encryption)의 한 형태입니다. 어떤 금액을 밝히지 않고 Grin은 모든 거래 투입의 합계가 거래 출력의 합계와 수수료를 합한 것과 일치하는지 확인할 수 있습니다. 더 나아가 광업으로 창출 된 모든 돈의 합계를 보유하고있는 총 금액과 비교하여, 그 라인 노드는 총 돈 공급의 정확성을 확인할 수 있습니다.

### If I listen to transaction relay, can't I just figure out who they belong to before being cut-through?

You can figure out which outputs are being spent by which transaction, but the trail of data stops here. All inputs and outputs look like random pieces of data, so you can't tell if the money was transferred, still belongs to the same person, which output is the actual transfer and which is the change, etc. Grin transactions are built with *no identifiable piece of information*.

In addition, Grin leverages [Dandelion relay](dandelion/dandelion.md), which provides additional anonymity as to which IP or client the transaction originated from, and allows for transactions to be aggregated.

어떤 거래가 어떤 출력을 소비하고 있는지 파악할 수 있지만 여기서 데이터의 흔적이 멈 춥니 다. 모든 입력과 출력은 임의의 데이터 조각처럼 보이므로 돈이 이전되었는지, 같은 사람에게 속해 있는지, 출력은 실제 전송인지, 변경인지 등은 알 수 없습니다. Grin 트랜잭션은 * 식별 할 수있는 정보는 없습니다 *.

또한, Grin은 [민들레 중계] (dandelion / dandelion.md)를 활용하여 트랜잭션이 발생한 IP 또는 클라이언트에 대한 추가 익명 성을 제공하고 트랜잭션을 집계 할 수 있습니다.

### 퀀텀 컴퓨타게돈(compute + armageddon) 에 대해서 궁금해요.

In every Grin output, we also include a bit of hashed data, which is quantum safe. If quantum computing was to become a reality, we can safely introduce additional verification that would protect existing coins from being hacked.

모든 Grin 결과에는 양자 안전 데이터가 포함되어 있습니다. 양자 컴퓨팅이 현실화되면 기존 동전을 해킹하지 못하게하는 추가 검증을 안전하게 도입 할 수 있습니다.

### 어떻게 이 모든일이 가능한거죠?

See our [technical introduction](intro.md) to get started.
