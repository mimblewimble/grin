# 블록체인의 동기화

최신 노드 상태를 따라 가기 위해 네트워크에 참여할 때 새 노드가 사용하는 여러 가지 방법을 설명합니다.
먼저, 독자에게 다음과 같은 Grin 또는 MimbleWimble의 특성을 먼저 전제 하고 설명하겠습니다.

* All block headers include the root hash of all unspent outputs in the chain at the time of that block.
* Inputs or outputs cannot be tampered with or forged without invalidating the whole block state.

We're purposefully only focusing on major node types and high level algorithms that may impact the security model. 
Detailed heuristics that can provide some additional improvements (like header first), while useful, will not be mentioned in this
section.

## Full 히스토리 동기화

### 설명

이 모델은 대부분의 메이저 퍼블릭 블록체인 에서 "풀 노드"가 사용하는 모델입니다. 새로운 노드는 제네시스 블록에 대한 사전 정보를 가지고 있습니다. 노드는 네트워크의 다른 피어와 연결되어 피어에게 알려진 최신 블록에 도달 할 때까지 블록을 요청하기 시작합니다.

보안 모델은 비트 코인과 비슷합니다. 전체 체인, 총 작업, 각 블록의 유효성, 전체 내용 등을 검증 할 수 있습니다. 또한 MimbleWimble 및 전체 UTXO 세트 실행들을 통해 훨씬 더 무결성 검증이 잘 수행될 수 있습니다.

We do not try to do any space or bandwidth optimization in this mode (for example,
once validated the range proofs could possibly be deleted). The point here is to
provide history archival and allow later checks and verifications to be made.

이 모드에서는 저장공간 최적화 또는 대역폭 최적화를 시도하지 않습니다 (예를 들자면 유효성 검증 후 Range proof 가 삭제 될 수 있습니다). 여기서 중요한 것은 기록 아카이브를 제공하고 나중에 확인 및 증명을 하게 하는 것입니다.

### 무엇이 잘못 될 수 있나요?

다른 블록체인과 동일하게 아래와 같은 문제가 생길 수 있습니다.

* 연결된 모든 노드가 부정직하다면 (sybil 공격 또는 그와 비슷한 상태를 말합니다.), 전체 체인 상태에 대해 거짓말을 할 수 있습니다.
* 엄청난 마이닝 파워를 가진 누군가가 전체 블록체인 기록을 다시 쓸 수 있습니다.
* Etc.

## 부분 블록체인 히스토리 동기화

이 모델에서는 보안에 대해서 가능한 한 적게 ​​타협하면서 매우 빠른 동기화를 위힌 최적화를 하려고 합니다. 사실 보안 모델은 다운로드 할 데이터의 양이 훨씬 적음에도 불구하고 풀 노드와 거의 동일합니다.

새로 네트워크에 참여하는 노드는 블록헤드에서 블록 수만큼 떨어진 값인 `Z`로 미리 구성됩니다. ( 원문에서는 horizon `Z` 로 표현되었습니다. 블록헤드 - 블록 = `Z`라고 할 수 있습니다. - 역자 주 ) 예를 들어 `Z = 5000` 이고 헤드가 높이 `H = 23000` 에 있으면, 가장 높은 블록은 가장 긴 체인에서 높이가 `h = 18000`인 블록입니다.

또한 새로운 노드에는 제네시스 블록에 대한 사전 정보가 있습니다. 노드는 다른 피어들과 연결하고 가장 긴 체인의 헤드에 대해 알게 됩니다. 가장 높은 블록의 블록 헤더를 요청하며 다른 피어의 동의가 필요하게 됩니다. 컨센서스가 `h = H - Z`에 이르지 않으면 노드는 `Z`값을 점차 증가시켜 컨센서스가 이루어질 때까지`h`를 뒤로 이동시킵니다. 그런 다음 가장 긴 블록에서의 전체 UTXO 정보를 얻습니다. 이 정보를 통해 다음을 증명할 수 있습니다.

* 모든 블록헤더안에 있는 해당 체인의 전체 난이도
* 예상되는 코인 공급량과 같은 모든 UTXO 실행값의 합.
* 블록헤더에 있는 루트 해시와 매치되는 모든 UTXO의 루트해시

블록의 유효성 검사가 완료되면 피어는 블록 콘텐츠를 `Z`값에서 헤드까지 다운로드하고 유효성을 검사 할 수 있습니다.

이 알고리즘은 `Z`의 매우 낮은 값 (또는 `Z = 1`인 극단적인 경우에도)에서도 작동합니다. 그러나 어느 블록체인에서도 발생할 수있는 정상적인 포크 때문에 낮은 값이 문제가 될 수 있습니다. 이러한 문제를 방지하고 로컬 검증된 을 늘리려면 최소한 며칠 분량의 `Z`값에서 최대 몇 주간의 `Z`값을 사용하는 것을 권장합니다.

### 무엇이 잘못 될 수 있나요?

While this sync mode is simple to describe, it may seem non-obvious how it still
can be secure. We describe here some possible attacks, how they're defeated and
other possible failure scenarios.

이 동기화 모드는 간단하게 설명 할 수 있지만 어떻게 보안이 유지되는 할 것인가에 대해선 불분명해 보일 수 있습니다. 
여기서는 몇몇 가능 할 수 있는 공격 유형과 공격 방식 및 기타 가능한 실패 시나리오에 대해 설명합니다.

#### An attacker tries to forge the state at horizon

This range of attacks attempt to have a node believe it is properly synchronized
with the network when it's actually is in a forged state. Multiple strategies can
be attempted:

* Completely fake but valid horizon state (including header and proof of work).
  Assuming at least one honest peer, neither the UTXO set root hash nor the block
  hash will match other peers' horizon states.
* Valid block header but faked UTXO set. The UTXO set root hash from the header
  will not match what the node calculates from the received UTXO set itself.
* Completely valid block with fake total difficulty, which could lead the node down
  a fake fork. The block hash changes if the total difficulty is changed, no honest
  peer will produce a valid head for that hash.

#### A fork occurs that's older than the local UTXO history

Our node downloaded the full UTXO set at horizon height. If a fork occurs on a block
at an older horizon H+delta, the UTXO set can't be validated. In this situation the
node has no choice but to put itself back in sync mode with a new horizon of
`Z'=Z+delta`.

Note that an alternate fork at Z+delta that has less work than our current head can
safely be ignored, only a winning fork of total work greater than our head would.
To do this resolution, every block header includes the total chain difficulty up to
that block.

#### The chain is permanently forked

If a hard fork occurs, the network may become split, forcing new nodes to always
push their horizon back to when the hard fork occurred. While this is not a problem
for short-term hard forks, it may become an issue for long-term or permanent forks
To prevent this situation, peers should always be checked for hard fork related
capabilities (a bitmask of features a peer exposes) on connection.

### Several nodes continuously give fake horizon blocks

If a peer can't reach consensus on the header at h, it gradually moves back. In the
degenerate case, rogue peers could force all new peers to always become full nodes
(move back until genesis) by systematically preventing consensus and feeding fake
headers.

While this is a valid issue, several mitigation strategies exist:

* Peers must still provide valid block headers at horizon `Z`. This includes the
  proof of work.
* A group of block headers around the horizon could be asked to increase the cost
  of the attack.
* Differing block headers providing a proof of work significantly lower could be
  rejected.
* The user or node operator may be asked to confirm a block hash.
* In last resort, if none of the above strategies are effective, checkpoints could
  be used.
