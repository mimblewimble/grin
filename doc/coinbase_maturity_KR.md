# The Coinbase Maturity Rule (aka Output Lock Heights)
# Coinbase 만기 규칙 (A.K.A 출력 )
Coinbase outputs (block rewards & fees) are "locked" and require 1,440 confirmations (i.e 24 hours worth of blocks added to the chain) before they mature sufficiently to be spendable. This is to reduce the risk of later txs being reversed if a chain reorganization occurs.

Coinbase 산출물 (블록 보상 및 수수료)은 "잠겨"있고 쓸모있을만큼 충분히 성숙하기 전에 1,440 회의 확인 (즉, 체인에 추가 된 24 시간 분량의 확인)이 필요합니다. 이것은 체인 재구성이 발생할 경우 나중에 txs가 역전 될 위험을 줄이기위한 것입니다.

Bitcoin does something very similar, requiring 100 confirmations (Bitcoin blocks are every 10 minutes, Grin blocks are every 60 seconds) before mining rewards can be spent.


Bitcoin은 광업 보상을 보내기 전에 100 회의 확인 (Bitcoin 블록은 매 10 분, Grin 블록은 매 60 초)이 필요한 매우 유사한 작업을 수행합니다.


Grin enforces coinbase maturity in both the transaction pool and the block validation pipeline. A transaction containing an input spending a coinbase output cannot be added to the transaction pool until it has sufficiently matured (based on current chain height and the height of the block producing the coinbase output).
Similarly a block is invalid if it contains an input spending a coinbase output before it has sufficiently matured, based on the height of the block containing the input and the height of the block that originally produced the coinbase output.

Grin은 트랜잭션 풀과 블록 유효성 검사 파이프 라인 모두에서 코인베이스 성숙도를 적용합니다. 코인베이스 출력을 사용하는 입력을 포함하는 트랜잭션은 충분히 성숙 될 때까지 (현재 체인 높이와 코인베이스 출력을 생성하는 블록의 높이를 기반으로) 트랜잭션 풀에 추가 될 수 없습니다. 유사하게 블록은 입력을 포함하는 블록의 높이와 원래 코인베이스 출력을 생성 한 블록의 높이를 기반으로 충분히 성숙되기 전에 코인베이스 출력을 소비하는 입력을 포함하면 유효하지 않습니다.


The maturity rule *only* applies to coinbase outputs, regular transaction outputs have an effective lock height of zero.

만기 규칙 * 만 * 코인베이스 출력에 적용됩니다. 일반 트랜잭션 출력은 유효한 잠금 높이가 0입니다.


An output consists of -

* features (currently coinbase vs. non-coinbase)
* commitment `rG+vH`
* rangeproof

To spend a regular transaction output two conditions must be met. We need to show the output has not been previously spent and we need to prove ownership of the output.

기적 인 트랜잭션 출력을 사용하려면 두 가지 조건이 충족되어야합니다. 출력물이 이전에 소비되지 않았 음을 보여 주어야하며 출력물의 소유권을 증명해야합니다.

A Grin transaction consists of the following -

* A set of inputs, each referencing a previous output being spent.
* A set of new outputs that include -
  * A value `v` and a blinding factor (private key) `r` multiplied on a curve and summed to be `rG+vH`
  * A range proof that shows that v is non-negative.
* An explicit transaction fee in the clear.
* A signature, computed by taking the excess blinding value (the sum of all outputs plus the fee, minus the inputs) and using it as the private key.

We can show the output is unspent by looking for the commitment in the current Output set. The Output set is authoritative; if the output exists in the Output set we know it has not yet been spent. If an output does not exist in the Output set we know it has either never existed, or that it previously existed and has been spent (we will not necessarily know which).

현재 출력 세트에서 커밋을 찾음으로써 출력이 미사용 상태임을 나타낼 수 있습니다. 출력 세트는 신뢰할 수 있습니다. 출력이 출력 집합에 존재하는 경우 아직 사용되지 않았 음을 알 수 있습니다.
출력이 출력 세트에 존재하지 않는다면 우리는 그것이 결코 존재하지 않았거나 이전에 존재했고 소비되었음을 알 수 있습니다 (우리는 반드시 어떤 것을 알지 못할 것입니다).

To prove ownership we can verify the transaction signature. We can *only* have signed the transaction if the transaction sums to zero *and* we know both `v` and `r`.


소유권을 증명하기 위해 거래 서명을 확인할 수 있습니다. 거래가 0이되고 v와 r을 모두 알고있는 경우에만 거래에 서명 할 수 있습니다.

Knowing `v` and `r` we can uniquely identify the output (via its commitment) *and* we can prove ownership of the output by validating the signature on the original coinbase transaction.

v와 r을 알면 우리는 출력을 (약속을 통해) 식별 할 수 있으며 원래의 코인베이스 트랜잭션에서 서명의 유효성을 검사하여 출력의 소유권을 증명할 수 있습니다.


Grin does not permit duplicate commitments to exist in the Output set at the same time.
But once an output is spent it is removed from the Output set and a duplicate commitment can be added back into the Output set.
This is not necessarily recommended but Grin must handle this situation in a way that does not break consensus across the network.

Grin은 동시에 Output 집합에 중복 된 커밋이 존재하는 것을 허용하지 않습니다. 그러나 일단 출력이 소비되면 출력 집합에서 제거되고 중복 확약이 출력 집합에 다시 추가 될 수 있습니다.
이것은 반드시 권장되는 것은 아니지만 Grin은 네트워크를 통해 합의를 깨지 않는 방식으로 이러한 상황을 처리해야합니다.

Several things complicate this situation -

1. It is possible for two blocks to have identical rewards, particularly for the case of empty blocks, but also possible for non-empty blocks with transaction fees.
1. It is possible for a non-coinbase output to have the same value as a coinbase output.
1. It is possible (but not recommended) for a miner to reuse private keys.

Grin does not allow duplicate commitments to exist in the Output set simultaneously.
But the Output set is specific to the state of a particular chain fork. It *is* possible for duplicate *identical* commitments to exist simultaneously on different concurrent forks.
And these duplicate commitments may have different "lock heights" at which they mature and become spendable on the different forks.

Grin은 출력 세트에 중복 된 커밋이 동시에 존재하는 것을 허용하지 않습니다. 그러나 출력 세트는 특정 체인 포크의 상태에 따라 다릅니다.
상이한 동시 포크에 중복 된 동일한 커미트먼트가 동시에 존재할 수 있습니다. 그리고 이러한 중복 된 약속은 성숙하고 다른 포크에서 쓸모있게되는 다른 "잠금 장치 높이"를 가질 수 있습니다.

* Output O<sub>1</sub> from block B<sub>1</sub> spendable at height h<sub>1</sub> (on fork f<sub>1</sub>)
* Output O<sub>1</sub>' from block B<sub>2</sub> spendable at height h<sub>2</sub> (on fork f<sub>2</sub>)

The complication here is that input I<sub>1</sub> will spend either O<sub>1</sub> or O<sub>1</sub>' depending on which fork the block containing I<sub>1</sub> exists on. And crucially I<sub>1</sub> may be valid at a particular block height on one fork but not the other.

여기서 복잡한 점은 입력 I1이 I1을 포함하는 블록이 존재하는 포크에 따라 O1 또는 O1 '을 사용한다는 것입니다. 그리고 결정적으로 I1은 하나의 포크에서는 특정 블록 높이에서 유효하지만 다른 포크에서는 유효하지 않을 수 있습니다.

Said another way - a commitment may refer to multiple outputs, all of which may have different lock heights. And we *must* ensure we correctly identify which output is actually being spent and that the coinbase maturity rules are correctly enforced based on the current chain state.
다른 말로하면, 커밋은 여러 개의 출력을 의미 할 수 있으며, 모든 출력은 서로 다른 잠금 높이를 가질 수 있습니다.
그리고 우리는 어떤 결과가 실제로 소비되고 있는지, 코인베이스 성숙도 규칙이 현재의 사슬 상태를 기반으로 정확하게 시행되고 있는지 정확하게 확인해야합니다.


A coinbase output, locked with the coinbase maturity rule at a specific lock height, *cannot* be uniquely identified, and *cannot* be safely spent by their commitment alone. To spend a coinbase output we need to know one additional piece of information -
특정 잠금 높이에서 coinbase 성숙도 규칙으로 고정 된 코인베이스 출력은 고유하게 식별 될 수 없으며 자신의 의지만으로는 안전하게 사용할 수 없습니다.
코인베이스 출력을 사용하려면 추가 정보 하나를 알아야합니다.


* The block the coinbase output originated from

Given this, we can verify the height of the block and derive the "lock height" of the output (+ 1,000 blocks).
이 경우 블록의 높이를 검증하고 출력물의 "잠금 높이"(+ 1000 블록)를 도출 할 수 있습니다.

## Full Archival Node

Given a full archival node it is a simple task to identify which block the output originated from.
A full archival node stores the following -
전체 아카이브 노드가 주어지면 출력이 어느 블록에서 시작되었는지 식별하는 것은 간단한 작업입니다. 전체 아카이브 노드는 다음을 저장합니다.


* full block data of all blocks in the chain
* full output data for all outputs in these blocks

We can simply look back though all the blocks on the chain and find the block containing the output we care about.
체인의 모든 블록을 뒤돌아 볼 수 있으며 우리가 신경 쓰는 출력이 들어있는 블록을 찾을 수 있습니다.

The problem is when we need to account nodes that may not have full block data (pruned nodes, non-archival nodes).
[what kind of nodes?]
문제는 전체 블록 데이터 (정리 된 노드, 비 보관 노드)가없는 노드를 고려해야 할 때입니다. [어떤 종류의 노드입니까?]

How do we verify coinbase maturity if we do not have full block data?

## Non-Archival Node

[terminology? what are these nodes called?]

A node may not have full block data.
A pruned node may only store the following (refer to pruning doc) -
노드에는 전체 블록 데이터가 없을 수 있습니다. 정리 된 노드는 다음을 저장할 수 있습니다 (제거 기록 문서 참조).


* Block headers chain.
* All transaction kernels.
* All unspent outputs.
* The output MMR and the range proof MMR

Given this minimal set of data how do we know which block an output originated from?
이 최소한의 데이터 집합을 감안할 때 출력이 어느 블록에서 시작되었는지 어떻게 알 수 있습니까?

And given we now know multiple outputs (multiple forks, potentially different lock heights) can all have the *same* commitment, what additional information do we need to provide in the input to uniquely identify the output being spent?
여러 출력 (여러 포크, 잠재적으로 서로 다른 자물쇠 높이)이 모두 같은 약속을 지킬 수 있는지, 투입물에서 사용 된 출력을 고유하게 식별하기 위해 입력에 추가 정보가 필요합니까?

And to take it a step further - can we do all this without relying on having access to full output data? Can we use just the output MMR?
그리고 한 단계 더 나아가기 위해 전체 출력 데이터에 액세스하지 않고도이 모든 작업을 수행 할 수 있습니까? 출력 MMR 만 사용할 수 있습니까?


### Proposed Approach

We maintain an index mapping commitment to position in the output MMR.
우리는 출력 MMR에서의 위치에 대한 색인 매핑 약속을 유지합니다.

If no entry in the index exists or no entry in the output MMR exists for a given commitment then we now the output is not spendable (either it was spent previously or it never existed).
인덱스에 항목이 없거나 출력 MMR의 항목이 주어진 약속에 대해 존재하지 않으면 출력이 낭비되지 않습니다 (이전에 소비되었거나 존재하지 않았 음).


If we find an entry in the output MMR then we know a spendable output exists in the Output set *but* we do not know if this is the correct one. We do not if it is a coinbase output or not and we do not know the height of the block it originated from.
출력 MMR에서 항목을 찾으면 소비 가능한 출력이 출력 집합에 있음을 알 수 있지만 이것이 올바른지 여부는 알 수 없습니다. 우리가 코인베이스 출력인지 아닌지와 그 블록이 시작된 블록의 높이를 알지 못한다면 우리는 그렇지 않습니다.


If the hash stored in the output MMR covers both the commitment and the output features and we require an input to provide both the commitment and the feature then we can do a further validation step -
출력 MMR에 저장된 해시가 커밋 및 출력 기능을 모두 포함하고 커밋과 기능을 모두 제공하기 위해 입력이 필요한 경우 추가 검증 단계를 수행 할 수 있습니다.


* output exists in the output MMR (based on commitment), and
* the hash in the MMR matches the output data included in the input
출력이 MMR에 존재하고 (약정 기준), MMR의 해시는 입력에 포함 된 출력 데이터와 일치합니다.


With this additional step we know if the output was a coinbase output or a regular transaction output based on the provided features.
The hash will not match unless the features in the input match the original output features.
이 추가 단계를 통해 출력이 코인베이스 출력인지 또는 제공된 기능을 기반으로하는 일반적인 트랜잭션 출력인지 여부를 알 수 있습니다. 입력의 기능이 원래 출력 기능과 일치하지 않으면 해시가 일치하지 않습니다.


For a regular non-coinbase output we are finished. We know the output is currently spendable and we do not need to check the lock height.
일반적인 non-coinbase 출력을 위해 우리는 마쳤습니다. 생산량은 현재 소비가 가능하므로 잠금 높이를 확인할 필요가 없습니다.


For a coinbase output we can proceed to verify the lock height and maturity. For this we need to identify the block where the output originated.
We cannot determine the block itself, but we can require the input to specify the block (hash) and we can then prove this is actually correct based on the merkle roots in the block header (without needing full block data).

[tbd - overview of merkle proofs and how we will use these to prove inclusion based on merkle root in the block header]

To summarize -

Output MMR stores output hashes based on `commitment|features` (the commitment itself is not sufficient).

We do not need to include the range proof in the generation of the output hash.

To spend an output we continue to need -

* `r` and `v` to build the commitment and to prove ownership

An input must provide -

* the commitment (to lookup the output in the MMR)
* the output features (hash in output MMR dependent on features|commitment)
* a merkle proof showing inclusion of the output in the originating block
* the block hash of originating blocks
  * [tbd - maintain index based on merkle proof?]

From the commitment and the features we can determine if the correct output is currently unspent.
From the block and the output features we can determine the lock height (if any).
