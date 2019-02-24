# 상태와 스토리지

## Grin의 상태

### 구조

Grin chain의 모든 상태는 다음 데이터와 같이 이루어져 있습니다.

1. unspent output(UTXO) 세트
2. 각 출력값에 대한 range proof
3. 모든 트랜잭션 커널(kernel)들
4. 상기에 언급했던 각각의  MMR (예외적으로 출력값 MMR은 사용되지 않은 것 뿐만 아니라 *모든* 출력값의 해쉬를 포함합니다.) 더해서, 유효한 Proof of work 와 함께 chain 안의 모든 헤더들은 상기 상태에 대해 고정되어야 합니다. ( 상태는 가장 많이 일한 체인와 일치합니다.)
한번 각각의 range proof 가 인증되고 모든 kernel의 실행 합계가 계산되었다면 node 의 작동에 더이상 엄격하게 필요하진 않습니다.

### 인증하기

완전한 Grin의 상태를 사용해서 우리는 다음과 같은 것들을 인증 할 수 있습니다.

1. Kernel 의 signature 가 Kernel의 실행에 대해 유효하다면 ( 공개키). 이것은 Kernel이 유요하다는것을 증명합니다.
2. 모든 커밋 실행의 합이 모든 UTXO 실행의 합에서 총 공급량을 뺸 값이 같다면 이것은 Kernal과 출력값의 실행들이 유효하고 코인이 새로이 만들어지지 않았다는 것을 증명합니다.
3. 모든 UTXO, range prook 와 Kernel 해쉬들은 각각의 MMR이 있고 그 MMR 들은 유효한 root 를 해쉬합니다.
4. 특정 시점에 가장 많이 일했다고 알려진 Block header 에는 3개의 MMR에 대한 root 가 포함됩니다. 이것은 전체 상태가 가장 많이 일한 chain ( 가장 긴 체인)에서 MMR과 증명들이 만들어졌다는 것을 입증합니다.

### MMR 과 Pruning

The data used to produce the hashes for leaf nodes in each MMR (in addition to
their position is the following:
데이터는 리프 노드안의 각 MMR의 Hash를 만들어 냈었지만 
* The output MMR hashes the feature field and the commitments of all outputs
  since genesis.
* 출력값의 MMR 해
* The range proof MMR hashes the whole range proof data.
* The kernel MMR hashes all fields of the kernel: feature, fee, lock height,
  excess commitment and excess signature.

Note that all outputs, range proofs and kernels are added in their respective
MMRs in the order they occur in each block (recall that block data is required
to be sorted).
모든 출력값, Range proof, 그리고 kernel은 각각의 MMR에 더해지고 

As outputs get spent, both their commitment and range proof data can be
removed. In addition, the corresponding output and range proof MMRs can be
pruned.

## 상태 스토리지

Grin 에 있는 출력값에 대한 데이터 스토리지, Range proof 와 kernel은 간단합니다.
그 형태는 데이터 엑세스를 위한 메모리 매핑 된 append only 파일입니다.
출력값이 소비되는것에 따라서 제거 로그는 지울수 있는 위치를 유지힙니다.
이런 포지션은 MMR과 노드 포지션이 같은 순서로 입력되었으므로 잘 일치합니다.
제거 로그가 커지면 (Append only 파일도 )때때로 해당 파일을 지워진 부분 없이 다시 작성해서 압축하고 제거 로그를 비울 수 있습니다.

MMR 에 대해서는 약간의 복잡함을 더할 필요가 있습니다.
