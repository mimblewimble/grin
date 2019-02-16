# 상태와 스토리지

## Grin의 상태

### 구조

Grin chain의 모든 상태는 다음 데이터와 같이 이루어져 있습니다.

1. unspent output(UTXO) 세트
2. 각 출력값에 대한 range proof
3. 모든 트랜잭션 커널(kernel)들
4. A MMR for each of the above (with the exception that the output MMR includes
   hashes for *all* outputs, not only the unspent ones).
4. 상기에 언급했던 각각의 것들에 대한 MMR (예외적으로 출력값 MMR은 사용되지 않은 것 뿐만 아니라 *모든* 출력값의 해쉬를 포함합니다.)
In addition, all headers in the chain are required to anchor the above state
with a valid proof of work (the state corresponds to the most worked chain).
더해서 chain 안의 모든 헤더들은 유효한 PoW 와 상기 상태에 대한 anchor가 요구됩니다. ( 상태는 가장 많이 일한 체인와 일치합니다.)
We note that once each range proof is validated and the sum of all kernels
commitment is computed, range proofs and kernels are not strictly necessary for
a node to function anymore.
한번 각각의 range proof 가 인증되고 모든 kernel의 실행 합계가 계산되었다면 node 의 작동에 굳이 꼭 필요하진 않습니다.

### 인증하기

With a full Grin state, we can validate the following:
완전한 Grin의 상태를 사용해서 우리는 다음과 같은 것들을 인증 할 수 있습니다.

1. The kernel signature is valid against its commitment (public key). This
   proves the kernel is valid.
   Kernel 의 signature 가 Kernel의 실행에 대해 유효하다면 ( 공개키). 이것은 Kernel이 유요하다는것을 증명합니다.
2. The sum of all kernel commitments equals the sum of all UTXO commitments
   minus the total supply. This proves that kernels and output commitments are all valid and no coins have unexpectedly been created.
    모든 커밋 실행의 합이 모든 UTXO 실행의 합에서 총 공급량을 뺸 값이 같다면 이것은 Kernal과 출력값의 실행들이 유효하고 코인이 새로이 만들어지지 않았다는 것을 증명합니다.  
3. All UTXOs, range proofs and kernels hashes are present in their respective
   MMR and those MMRs hash to a valid root.
   모든 UTXO, range prook 와 Kernel 해쉬들은 각각의 MMR이 있고 그 MMR 들은 유효한 root 를 해쉬합니다.
4. A known block header with the most work at a given point in time includes
   the roots of the 3 MMRs.
   특정 시점에 가장 많이 일했다고 알려진 Block header 에는 3개의 MMR에 대한 root 가 포함됩니다.
   This validates the MMRs and proves that the whole state has been produced by the most worked chain.
   이것은 MMR과 인증하고 전체 상태가 가장 많이 일한 chain ( 가장 긴 체인)에서 만들어졌다는 것을 증명합니다.

### MMR 과 Pruning

The data used to produce the hashes for leaf nodes in each MMR (in addition to
their position is the following:

* The output MMR hashes the feature field and the commitments of all outputs
  since genesis.
* The range proof MMR hashes the whole range proof data.
* The kernel MMR hashes all fields of the kernel: feature, fee, lock height,
  excess commitment and excess signature.

Note that all outputs, range proofs and kernels are added in their respective
MMRs in the order they occur in each block (recall that block data is required
to be sorted).

As outputs get spent, both their commitment and range proof data can be
removed. In addition, the corresponding output and range proof MMRs can be
pruned.

## 상태 스토리지

Data storage for outputs, range proofs and kernels in Grin is simple: 
Grin 에 있는 출력값에 대한 데이터 스토리지, Range proof 와 kernel은 간단합니다.
a plain append-only file that's memory-mapped for data access. As outputs get spent,a remove log maintains which positions can be removed. 

Those positions nicely match MMR node positions as they're all inserted in the same order.

When the remove log gets large, corresponding files can be occasionally compacted by rewriting them without the removed pieces (also append-only) and the remove log can be emptied.

As for MMRs, we need to add a little more complexity.
MMR 에 대해서는 약간의 복잡함을 더할 필요가 있습니다.
