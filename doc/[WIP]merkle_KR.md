# 머클의 구조

MimbleWimble은 Pruning 데이터만 있는 시스템의 상태를 사용자가 증명하도록 설계되었습니다. 이러한 목표를 달성하기 위해 모든 트랜잭션 데이터는 pruning 된 경우라도 효율적인 업데이트와 serialization을 지원하는 Merkle 트리를 사용하여 블록 체인에 커밋됩니다.

또한 거의 모든 거래 데이터 (입력, 출력, Excess 및 Excess proof)는 어떤 방식으로 합산 될 수 있으므로 Merkle sum 트리를 기본 옵션으로 처리하고 여기에서 합계를 처리하는 것이 좋습니다.

A design goal of Grin is that all structures be as easy to implement and
as simple as possible. MimbleWimble introduces a lot of new cryptography
so it should be made as easy to understand as possible. Its validation rules
are simple to specify (no scripts) and Grin is written in a language with
very explicit semantics, so simplicity is also good to achieve well-understood
consensus rules.

Grin의 디자인 목표는 모든 구조를 구현하기 쉽고 가능한 한 간단하게 만드는 것입니다.
MimbleWimble은 많은 새로운 암호화 방식을 내 놓았고 이러한 방식을  가능한 한 쉽게 이해할 수 있도록 만들어야합니다.
새로운 암호화 방식의 유효성 규칙은 스크립트가 없이도 지정하기 쉽고 Grin은 매우 명확한 의미론을 가진 프로그래밍 언어로 작성되기 때문에 잘 이해되는 합의 규칙을 달성하는 것이 단순합니다.

## Merkle Trees

각 블록마다 4가지의 머클 트리가 커밋됩니다.

### Total Output Set

각 오브젝트는 uspent output 을 나타내는 commitment 또는 spent를 나타내는 NULL 마커 두 가지 중 하나입니다. Unspent 출력에 대한 sum-tree 입니다 (Spent 된 것은 합계에 아무런 영향을 미치지 않습니다). output 세트는 현재 블록이 적용된 *후에* 체인 의 상태를 반영해야합니다.

Root 합계는 제네시스 블록 이후 모든 Excess의 합계와 같아야합니다.

설계 요구 사항은 아래와 같습니다.

1. 효율적으로 추가 되어야 하고 및 unspent 에서 spent 로 업데이트가 되어야 합니다.
2. 특정 출력값이 Spent 임을 효율적으로 증명해야 합니다.
3. UTXO root간에 diffs를 효율적으로 저장합니다.
4. 수백만 개의 항목이 있거나 누락된 데이터가 있는 경우에도 트리에 효율적으로 저장되어야 합니다.
5. 노드가 NULL로 커밋되는 경우에는 unspent 하위 항목이 없고 그 데이터를 결과적으로 영구히 삭제할 수 있게 합니다.
6. 부분 아카이브 노드에서 Pruning된 트리의 serializtion 및 효율적인 병합을 지원합니다.

### Output의 증거

이 트리는 전체 출력 set을 반영하지만 commitment 대신 range proof를 가집니다. 이 트리는 절대 업데이트 되지 않고, 단지 추가되고, 어떤 것이든 더이상 더하지 않습니다. 출력을 소비 할 때 Tree를 삭제하는 것보다는 tree 에서 rangeproof를 삭제하는 것으로 충분합니다.

설계 요구 사항은 아래와 같습니다.

1. 부분 아카이브 노드에서 Pruning 된 트리의 serializtion 과 효율적인 병합을 지원해야 합니다.

### 입력과 출력

각 객체는 입력 (이전 트랜잭션 출력에 대한 명확한 레퍼런스) 또는 출력 (commitment, rangeproof) 중 하나입니다. 이 sum-tree는 출력에 대한 commitment이고 입력의 commitment에 대한 원본입니다.

입력 레퍼런스는 이전 commitment의 해시입니다. 모든 unspent 출력은 유니크 해야한다는 것이 컨센서스의 규착입니다.

Root 합계는 이 블록의 Excess 합계와 같아야 합니다. 이에 대해 다음 섹션을 참고하세요.

일반적으로 밸리데이터는 이 Merkle 트리의 100 % 또는 0 %를 확인 할 수 있으므로 모든 디자인과 호환됩니다.
설계 요구 사항은 다음과 같습니다 :

1. Proof of publication을 위해서 증명을 효율적으로 포함해야 합니다.

### Excesses

각 객체는 (초과, 서명) 형식입니다. 이러한 객체는 Excess를 합친 sum-tree 입니다.

일반적으로 밸리데이터는 항상 이 트리의 100 %를 확인 할 것이므로 Merkle 구조일 필요가 전혀 없습니다. 그러나 나중에 부분 아카이브 노드를 지원하기 위해 효율적인 Pruning을 지원하기를 원합니다.

설계 요구 사항 은 아래와 같습니다. :

1. 부분 아카이브 노드에서 pruning 된 tree의 serialzatoin 과 효율적인 병합을 지원해야 합니다.

## Proposed Merkle Structure

**The following design is proposed for all trees: a sum-MMR where every node
sums a count of its children _as well as_ the data it is supposed to sum.
The result is that every node commits to the count of all its children.**

**모든 tree에 대해 다음과 같은 설계가 제안됩니다. Sum-MMR은 더할 데이터 뿐만 아니라 자식의 수도 더합니다.
결과적으로 모든 노드가 모든 하위 노드의 수로 커밋됩니다.**

[MMRs, or Merkle Mountain Ranges](https://github.com/opentimestamps/opentimestamps-server/blob/master/doc/merkle-mountain-range.md)

출력값 세트를 위해서 6개의 디자인 원칙은 다음과 같습니다.

### 효율적인 insert/updates

즉시적이여야 합니다. (지금은 proof-of-inclusion입니다.). 이 원칙은 균형 잡힌 Merkle tree 디자인에 합당합니다.

### 효율적인 proof-of-spentness

Grin은 proof of spentness가 필요하지 않지만 SPV client 을 위해 앞으로 지원하는 것이 좋습니다.

자식의 수는 tree의 각 개체에 대한 인덱스를 의미합니다. 삽입은 트리의 맨 오른쪽에서만 발생하므로 변경되지 않습니다.

이렇게하면 동일한 출력이 나중에 트리에 추가 되더라도 영구적으로 proof-of-spentness를 허용하고 동일한 출력에 대해서도 오류 잘못된 증명에 대해서도 방지 할 수 있습니다. 이러한 속성은 삽입 순서가 지정되지 않은 tree에서는 하기 어렵습니다.

### 효율적인 diffs의 저장

모든 블록을 저장하면 충분합니다. 업데이트는 실행 취소만큼 수월하고, 블록은 항상 순서대로 처리되기 때문에 트리의 오른쪽에서 인접한 출력 세트를 제거하는 것과 만큼 reorg를 하는 동안 블록을 되감는 것이 간단합니다. 삭제를 지원하도록 설계된 트리의 반복 삭제보다 훨씬 빠릅니다.

### 데이터가 손실되는 상황에서도 효율적인 tree의 저장

랜덤한 결과가 소비되었을 때 root 해시를 업데이트하려면 전체 tree를 저장하거나 계산할 필요가 없습니다. 대신 depth 20에 해시 만 저장할 수 있습니다. 쉽세 말하자면 최대 100 만개가 저장됩니다. 그런 다음 각 업데이트는 이 depth보다 위의 해시를 다시 계산하면됩니다 (Bitcoin의 히스토리에는 2 ^ 29 미만의 출력이 있으므로 각 업데이트에 대해 크기가 2 ^ 9 = 512 인 트리를 계산해야 함). 모든 업데이트가 완료되면 root 해시를 다시 계산할 수 있습니다.

이 깊이는 설정 할 수 있고 출력 set가 증가하거나 사용 가능한 디스크 공간에 따라 변경 될 수 있습니다.

이런 과정은 어느 Merkle 트리에서 가능하지만 깊이를 어떻게 계산하느냐에 따라 PATRICIA tree 나 다른 prefix tree로 인해 복잡 할 수 있습니다.

### 사용된 코인 지우기

코인은 spent 에서 unspent로 이동하지 않으므로 spent 된 코인에 대한 데이터는 더 이상 업데이트나 검색를 위해 필요하지 않습니다.

### Efficient serialization of pruned trees

Since every node has a count of its children, validators can determine the
structure of the tree without needing all the hashes, and can determine which
nodes are siblings, and so on.

In the output set each node also commits to a sum of its unspent children, so
a validator knows if it is missing data on unspent coins by checking whether or
not this sum on a pruned node is zero.

모든 노드는 자식 수를 가지므로 밸리데이터는 모든 해시를 필요로하지 않고 tree 구조를 결정할 수 있으며 형제 노드를 결정할 수 있습니다.

출력 세트에서 각 노드는 unspent한 자식의 합계도 커밋하므로 밸리데이터는 정리되지 않은 노드에서이 합계가 0인지 여부를 확인하여 사용되지 않은 동전의 데이터가 누락되었는지 확인합니다.

## Algorithms

(To appear alongside an implementation.)

## Storage

The sum tree data structure allows the efficient storage of the output set and
output witnesses while allowing immediate retrieval of a root hash or root sum
(when applicable). However, the tree must contain every output commitment and
witness hash in the system. This data is too big to be permanently stored in
memory and too costly to be rebuilt from scratch at every restart, even if we
consider pruning (at this time, Bitcoin has over 50M UTXOs which would require
at least 3.2GB, assuming a couple hashes per UTXO). So we need an efficient way
to store this data structure on disk.

Another limitation of a hash tree is that, given a key (i.e. an output
commitment), it's impossible to find the leaf in the tree associated with that
key. We can't walk down the tree from the root in any meaningful way. So an
additional index over the whole key space is required. As an MMR is an append
only binary tree, we can find a key in the tree by its insertion position. So a
full index of keys inserted in the tree (i.e. an output commitment) to their
insertion positions is also required.


합계 트리 데이터 구조를 사용하면 출력 집합과 출력 증인을 효율적으로 저장하면서 루트 해시 또는 루트 합계 (해당되는 경우)를 즉시 검색 할 수 있습니다. 그러나 트리는 시스템에 모든 출력 약속 및 감시 해시를 포함해야합니다. 이 데이터는 너무 커서 메모리에 영구적으로 저장 될 수 없으며 재시작 할 때마다 처음부터 다시 작성하기에는 너무 비싸다. (이번에는 Bitcoin이 적어도 3.2GB를 필요로하는 50M UTXO를 가지고있다. UTXO 당). 따라서이 데이터 구조를 디스크에 저장하는 효율적인 방법이 필요합니다.

해시 트리의 또 다른 한계는 키 (즉, 출력 커미트먼트)가 주어지면, 그 키와 연관된 트리에서 잎을 발견하는 것이 불가능하다는 것이다. 우리는 의미있는 방식으로 뿌리에서 나무를 걸어 내려 갈 수 없습니다. 따라서 전체 키 공간에 대한 추가 색인이 필요합니다. MMR은 추가 전용 이진 트리이므로 삽입 위치를 기준으로 트리에서 키를 찾을 수 있습니다. 따라서 트리에 삽입 된 키의 전체 인덱스 (즉, 출력 커미트먼트)가 그들의 삽입 위치에 또한 요구된다.