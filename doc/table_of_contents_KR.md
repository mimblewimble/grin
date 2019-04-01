# 문서 목록

## Grin 에 대한 설명들

- [intro](intro_KR.md) - Grin 에 대한 기술적인 소개
- [grin4bitcoiners](grin4bitcoiners.md) - Bitcoinner 의 관점에서 Grin 을 설명하기

## Grin 구현에 대해서 이해하기

- [grin4bitcoiners](grin4bitcoiners.md) - Grin 의 Blockchain이 어떻게 동기화 되는가에 대해서
- [blocks_and_headers](chain/blocks_and_headers.md) - Grin이 어떻게 block과 header 를 chain안에서 찾는지에 대해서
- [contract_ideas](contract_ideas.md) - 어떻게 smart contract 를 구현할 것인가에 대한 아이디어
- [dandelion/dandelion](dandelion/dandelion.md) - 트랜잭션 전파 와 [컷 스루 방식](http://www.ktword.co.kr/abbr_view.php?m_temp1=1823). Stemming과 fluffing.
- [dandelion/simulation](dandelion/simulation.md) - Dandelion 시뮬레이션 - lock height 스테밍과 플러핑 없이 트랜잭션 합치기
- [internal/pool](internal/pool.md) - 트랜잭션 풀에 대한 기술적인 설명에 대해서
- [merkle](merkle.md) - Grin의 Merkle tree 에 대한 기술적인 설명
- [merkle_proof graph](merkle_proof/merkle_proof.png) - Prunning 이 적용된 merkle proof의 예시
- [pruning](pruning.md) - Pruning 의 기술적인 설명
- [stratum](stratum.md) -Grin Stratum RPC protocol 의 기술적 설명
- [transaction UML](wallet/transaction/basic-transaction-wf.png) - 상호작용 트랜잭션의 UML (`lock_height` 없이 트랜잭션 합치기)

## 빌드하고 사용하기

- [api](api/api.md) - Grin 에 있는 다른 API 들과 어떻게 사용하는지에 대해서
- [build](build.md) - Grin 바이너리를 어떻게 빌드하고 작동시키는 지에 대해서
- [release](release_instruction.md) - Release 를 만드는것에 대한 안내
- [usage](usage.md) - Testnet3 에서 어떻게 Grin 을 사용하는지에 대한 설명
- [wallet](wallet/usage.md) - wallet 디자인에 대한 설명과 `grin wallet` 의 세부 명령어

## 외부자료 (위키)

- [FAQ](https://github.com/mimblewimble/docs/wiki/FAQ) - 자주 물어보는 질문들
- [Grin 빌드하기](https://github.com/mimblewimble/docs/wiki/Building)
- [Grin을 어떻게 사용하나요?](https://github.com/mimblewimble/docs/wiki/How-to-use-grin)
- [해킹과 기여하기](https://github.com/mimblewimble/docs/wiki/Hacking-and-contributing)
