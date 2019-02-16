# 문서 목록

## Grin 에 대한 설명들

- [intro](intro.md) - Grin 에 대한 기술적인 소개
- [grin4bitcoiners](grin4bitcoiners.md) - Bitcoinner 의 관점에서 Grin 을 설명하기

## Grin 구현에 대해서 이해하기

- [grin4bitcoiners](grin4bitcoiners.md) - Grin 의 Blockchain이 어떻게 동기화 되는가에 대해서
- [blocks_and_headers](chain/blocks_and_headers.md) - Grin이 어떻게 block과 header 를 chain안에서 찾는지에 대해서
- [contract_ideas](contract_ideas.md) - 어떻게 smart contract 를 구현할 것인가에 대한 아이디어
- [dandelion/dandelion](dandelion/dandelion.md) - 트랜잭션 전파 와 [컷 스루 방식](http://www.ktword.co.kr/abbr_view.php?m_temp1=1823). 어간추출과 부풀리기.
- [dandelion/simulation](dandelion/simulation.md) - Dandelion 시뮬레이션 - lock height 스테밍과 플러핑 없이 트랜잭션 합치기
- [internal/pool](internal/pool.md) - 트랜잭션 풀에 대한 기술적인 설명에 대해서
- [merkle](merkle.md) - Grin의 Merkle tree 에 대한 기술적인 설명
- [merkle_proof graph](merkle_proof/merkle_proof.png) - Prunning 이 적용된 merkle proof의 예시
- [pruning](pruning.md) - Pruning 의 기술적인 설명
- [stratum](stratum.md) -Grin Stratum RPC protocol 의 기술적 설명
- [transaction UML](wallet/transaction/basic-transaction-wf.png) - UML of an interactive transaction (aggregating transaction without `lock_height`)

## Build and use

- [api](api/api.md) - Explaining the different APIs in Grin and how to use them
- [build](build.md) - Explaining how to build and run the Grin binaries
- [release](release_instruction.md) - Instructions of making a release
- [usage](usage.md) - Explaining how to use grin in Testnet3
- [wallet](wallet/usage.md) - Explains the wallet design and `grin wallet` sub-commands

## External (wiki)

- [FAQ](https://github.com/mimblewimble/docs/wiki/FAQ) - Frequently Asked Questions
- [Building grin](https://github.com/mimblewimble/docs/wiki/Building)
- [How to use grin](https://github.com/mimblewimble/docs/wiki/How-to-use-grin)
- [Hacking and contributing](https://github.com/mimblewimble/docs/wiki/Hacking-and-contributing)
