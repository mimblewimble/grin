# 패스트 싱크

*다른 언어로 이 문서를 읽고 싶으시다면: [Español](fast-sync_ES.md).*

In Grin, we call "sync" the process of synchronizing a new node or a node that
hasn't been keeping up with the chain for a while, and bringing it up to the
latest known most-worked block. 
Grin 에서 새로운 노드가 동기화 되는 과정이나 체인을 얼마간 알지 못했을때 

Initial Block Download (or IBD) is often used by other blockchains, but this is problematic for Grin as it typically does not
download full blocks.
IBD ( 최초 블록 다운로드 )는 다른 블록체인에서도 종종 사용되어 집니다. 그러나 Grin에 있어서 


요약하자면, Grin 안에서의 fast-sync 는 다음과 같은 과정을 따릅니다.
1. 다른 노드들에게서 알려지는 가장 많이 일한 체인에 있는 모든 블록 헤더를 청크단위로 다운로드 합니다. 
2. Find a header sufficiently back from the chain head. This is called the node
   horizon as it's the furthest a node can reorganize its chain on a new fork if
   it were to occur without triggering another new full sync.
3. 체인헤드로 부터 
4. Download the full state as it was at the horizon, including the unspent
   output, range proof and kernel data, as well as all corresponding MMRs. This is
   just one large zip file.

5. 모든 스테이트를 입증하는합니다.. 
. Download full blocks since the horizon to get to the chain head.
1. 

In the rest of this section, we will elaborate on each of those steps.
이 섹션의 나머지 부분에 대해서는 각각 자세히 설명할 것입니다.