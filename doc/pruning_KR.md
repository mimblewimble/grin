# 블록체인 데이터 프루닝(가지치기)에 대해

Mimblewimble의 주된 매력 중 하나는 이론적인 공간효율성 입니다. 실제로 신뢰 할수 있거나 또는 사전에 입증된 전체 블록체인 스테이트는 아주 작을수도 있는 UTXO(unspent transaction outputs)만 나타냅니다.

Grin의 블록체인에는 다음 유형의 데이터가 포함됩니다 (Mimblewimble 프로토콜에 대한 사전 지식이 있다고 가정합니다).

1. 아래를 포함하는 트랜잭션 출력값
   1. Pedersen commitment (33 bytes).
   2. range proof (현재는 5KB 이상)
2. 출력값의 레퍼런스인 트랜잭션 입력값 (32 bytes)
3. 각각의 트랜잭션에 포함된 트랜잭션 "증명들"
    1. 트랜잭션의 excess commitment 합계(33 bytes)
    2. 초과값과 함께 생성된 서명 (평균 71 bytes)
4. 머클트리와 작업증명을 포함한 블록헤더 (약 250 bytes)

백만개의 블록에 천만 개의 트랜잭션 (2 개의 입력이 있고 평균 2.5 개의 출력값이 있다고 가정할때) 과 10만개의 UTXO(원문에서는 unspent outputs라고 표기 - 역자 주)를 가정 할 때 전체 체인 (Pruing 없음, 컷 쓰루 없음)과 함께 대략적인 체인의 크기를 얻습니다.

* 128GB 크기의 트랜잭션 데이터 (inputs and outputs).
* 1 GB 크기의 트랜잭션 proof data.
* 250MB 크기의 block headers.
* 약 130GB 크기의 전체 체인 사이즈.
* 1.8GB크기의 컷-스루(cut-through) 이후의 전체 체인 사이즈(헤더 데이터는 포함함)
* 520MB 크기의 UTXO 사이즈.
* Total chain size, without range proofs of 4GB.
* 4GB크기의 range proof가 없는 경우 전체 체인 사이즈
* 3.3MB 크기의 range proof가 없는 경우 UTXO 사이즈

모든 데이터에서 체인이 완전히 검증되면 UTXO commitment의 세트 만 노드 작동에 필수적으로 필요합니다.

데이터가 정리(prune) 될 수있는 아래와 같은 몇 가지 상황이 있을 수 있습니다.

* 입증된 풀 노드는 여유공간에 확인된 데이터들을 삭제 할 수 있습니다.
* 풀 노드는 빈 공간의 유효성을 확인
* 부분 검증 노드 (SPV와 유사함)는 모든 데이터를 수신하거나 유지하는 데 관심이 없을 수 있습니다.
* 새 노드가 네트워크에 참여하면 결과적으로 풀 노드가 될지라도 더 빨리 사용할 수있도록 하기 위해 부분 검증 노드로 일시적으로 작동 할 수 있습니다.

## 완전히 정리된 스테이트(Fully Pruned State)의 입증에 대해서

(데이터)Pruning은 가능한 한 많은 양의 데이터를 제거하면서 Mimblewimble 스타일의 검증을 보장하는 것이 필요합니다.
이는 pruning 노드 상태를 정상적으로 유지하는 데 필요할 뿐만 아니라 최소한의 양의 데이터만 새 노드로 전송할 첫번째 고속 동기화에서도 필요합니다.

체인 스테이트의 완전한 입증을 위해 아래와 같은 사항들이 필요합니다.

* 모든 Kernel의 서명들은 kernel의 공개키에 의해서 증명됩니다.
* The sum of all UTXO commitments, minus the supply is a valid public key (can
  be used to sign the empty string).
* 모든 커널의 pubkeys 합계는 모든 UTXO commitment에서 공급을 뺀 값과 같습니다.
* UTXO PMMR의 루트 해시, Range proof의 PMMR 및 Kernel의 MMR은 유효한 작업증명 체인의 블록헤더와 일치힙니다.
* 모든 Range proof가 유효해야 합니다.

또한 전체 체인의 스테이트에 대해 확인 할 필요는 없지만 새 블록을 받아들이고 유효성 입증을 하려면 아래와 같은 추가 데이터가 필요합니다.

* 출력 기능에서 모든 UTXO에 필요한 전체 출력 데이터를 만듭니다

(그러기 위해선)최소한 다음과 같은 데이터가 필요합니다.

* 블록헤더의 체인
* 체인에 포함된 순서로 되어있는 모든 Kernel들. 이 Kernel들은 Kernel MMR 의 재구성을 가능하게 합니다.
* 모든 UTXO(원문에서는 unspent output 으로 표기 - 역자 주)
* 정리된 데이터(Pruned data)의 해시를 알기위한 UTXO MMR과 Range proof MMR.

입증된 노드에 의해서 랜덤하게 선택된 Range proof의 하위 set만 증명함으로써 추가 pruning이 가능 할 수 있습니다.