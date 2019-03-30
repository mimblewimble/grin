# Grin's Proof-of-Work

This document is meant to outline, at a level suitable for someone without prior knowledge,
the algorithms and processes currently involved in Grin's Proof-of-Work system. We'll start
with a general overview of cycles in a graph and the Cuckoo Cycle algorithm which forms the
basis of Grin's proof-of-work. We'll then move on to Grin-specific details, which will outline
the other systems that combine with Cuckoo Cycle to form the entirety of mining in Grin.

Please note that Grin is currently under active development, and any and all of this is subject to
(and will) change before a general release.

<<<<<<< HEAD
=======
이 문서는 사전 지식이없는 사람에게 적합한 수준에서 Grin의 Proof-of-Work 시스템과 관련된 알고리즘 및 프로세스를 개괄적으로 설명합니다. Grin의 작업 증명의 기초를 형성하는 Cuckoo Cycle 알고리즘과 그래프의주기에 대한 일반적인 개요로 시작하겠습니다. 그런 다음 Grin 고유의 세부 정보로 이동하여 Cuckoo Cycle과 결합하여 Grin에서 광업 전체를 형성하는 다른 시스템에 대해 설명합니다.

Grin은 현재 적극적으로 개발 중이며,이 중 일부 및 전부는 일반 출시 전에 변경 될 수 있습니다.

>>>>>>> ce3f5375948e91d021122bf637998b55c4248fda
## Graphs and Cuckoo Cycle

Grin's basic Proof-of-Work algorithm is called Cuckoo Cycle, which is specifically designed
to be resistant to Bitcoin style hardware arms-races. It is primarily a memory bound algorithm,
which, (at least in theory,) means that solution time is bound by memory bandwidth
rather than raw processor or GPU speed. As such, mining Cuckoo Cycle solutions should be viable on
most commodity hardware, and require far less energy than most other GPU, CPU or ASIC-bound
proof of work algorithms.

<<<<<<< HEAD
=======
Grin의 기본 Proof-of-Work 알고리즘은 Bitcoin 스타일 하드웨어 군비에 내성을 갖도록 특별히 설계된 Cuckoo Cycle이라고합니다. 이것은 주로 메모리 바운드 알고리즘입니다. (적어도 이론 상으로는) 솔루션 시간이 원시 프로세서 또는 GPU 속도가 아닌 메모리 대역폭에 의해 제한된다는 것을 의미합니다. 따라서 광산 뻐꾸기 사이클 솔루션은 대부분의 상용 하드웨어에서 실행 가능해야하며 다른 대부분의 GPU, CPU 또는 ASIC의 바인딩 된 작업 증명 알고리즘보다 훨씬 적은 에너지를 필요로합니다.

>>>>>>> ce3f5375948e91d021122bf637998b55c4248fda
The Cuckoo Cycle POW is the work of John Tromp, and the most up-to-date documentation and implementations
can be found in [his github repository](https://github.com/tromp/cuckoo). The
[white paper](https://github.com/tromp/cuckoo/blob/master/doc/cuckoo.pdf) is the best source of
further technical details.

There is also a [podcast with Mike from Monero Monitor](https://moneromonitor.com/episodes/2017-09-26-Episode-014.html)
in which John Tromp talks at length about Cuckoo Cycle; recommended listening for anyone wanting
more background on Cuckoo Cycle, including more technical detail, the history of the algorithm's development
and some of the motivations behind it.

### Cycles in a Graph

Cuckoo Cycle is an algorithm meant to detect cycles in a bipartite graph of N nodes
and M edges. In plainer terms, a bipartite graph is one in which edges (i.e. lines connecting nodes)
travel only between 2 separate groups of nodes. In the case of the Cuckoo hashtable in Cuckoo Cycle,
one side of the graph is an array numbered with odd indices (up to the size of the graph), and the other is numbered with even
indices. A node is simply a numbered 'space' on either side of the Cuckoo Table, and an Edge is a
line connecting two nodes on opposite sides. The simple graph below denotes just such a graph,
with 4 nodes on the 'even' side (top), 4 nodes on the odd side (bottom) and zero Edges
(i.e. no lines connecting any nodes.)

<<<<<<< HEAD
=======
Cuckoo Cycle은 N 개의 노드와 M 개의 가장자리로 구성된 이진 그래프의 사이클을 감지하기위한 알고리즘입니다. 간단히 말해서, 2 부분 그래프는 모서리 (즉, 노드를 연결하는 선)가 2 개의 개별 노드 그룹 사이에서만 이동하는 그래프입니다. Cuckoo Cycle에서 Cuckoo 해시 테이블의 경우, 그래프의 한면은 홀수 색인 (그래프 크기까지)이 붙은 배열이고 다른 배열은 짝수 인덱스로 번호가 매겨집니다. 노드는 단순히 Cuckoo Table의 한쪽에 번호가 매겨진 '공간'이고, Edge는 반대쪽에있는 두 노드를 연결하는 선입니다. 아래의 간단한 그래프는 '짝수'측면 (상단)에 4 개의 노드, 홀수 측면 (하단)에 4 개의 노드 및 가장자리가없는 에지 (즉, 모든 노드를 연결하는 선)가없는 그래프를 나타냅니다.

>>>>>>> ce3f5375948e91d021122bf637998b55c4248fda
![alt text](images/cuckoo_base_numbered_minimal.png)

*A graph of 8 Nodes with Zero Edges*

Let's throw a few Edges into the graph now, randomly:

![alt text](images/cuckoo_base_numbered_few_edges.png)

*8 Nodes with 4 Edges, no solution*

We now have a randomly-generated graph with 8 nodes (N) and 4 edges (M), or an NxM graph where
N=8 and M=4. Our basic Proof-of-Work is now concerned with finding 'cycles' of a certain length
within this random graph, or, put simply, a series of connected nodes starting and ending on the
same node. So, if we were looking for a cycle of length 4 (a path connecting 4 nodes, starting
and ending on the same node), one cannot be detected in this graph.

Adjusting the number of Edges M relative to the number of Nodes N changes the difficulty of the
cycle-finding problem, and the probability that a cycle exists in the current graph. For instance,
if our POW problem were concerned with finding a cycle of length 4 in the graph, the current difficulty of 4/8 (M/N)
would mean that all 4 edges would need to be randomly generated in a perfect cycle (from 0-5-4-1-0)
in order for there to be a solution.

<<<<<<< HEAD
=======

이제 8 개의 노드 (N)와 4 개의 에지 (M) 또는 N = 8과 M = 4 인 NxM 그래프가있는 무작위로 생성 된 그래프가 있습니다. 우리의 기본적인 Proof-of-Work는 이제이 무작위 그래프 내에서 특정 길이의 '주기'를 찾거나 단순히 같은 노드에서 시작하고 끝나는 일련의 연결된 노드를 찾는 것과 관련이 있습니다. 따라서 길이 4 (동일한 노드에서 시작하고 끝나는 4 개의 노드를 연결하는 경로)의 사이클을 찾는다면이 그래프에서 하나를 발견 할 수 없습니다.

노드 수 N을 기준으로 한 모서리 수를 조정하면 사이클 찾기 문제의 난이도와 현재 그래프에 사이클이 존재할 확률이 변경됩니다. 예를 들어, POW 문제가 그래프에서 길이 4의주기를 찾는 것과 관련된다면, 현재의 4/8 난이도 (M / N)는 모든 4 개의 엣지가 완벽한 사이클에서 무작위로 생성 될 필요가 있음을 의미합니다. 0-5-4-1-0)을 사용하십시오.

>>>>>>> ce3f5375948e91d021122bf637998b55c4248fda
Let's add a few more edges, again at random:

![alt text](images/cuckoo_base_numbered_more_edges.png)

*8 Nodes with 7 Edges*

Where we can find a cycle:

![alt text](images/cuckoo_base_numbered_more_edges_cycle.png)

*Cycle Found from 0-5-4-1-0*

If you increase the number of edges relative to the number
of nodes, you increase the probability that a solution exists. With a few more edges added to the graph above,
a cycle of length 4 has appeared from 0-5-4-1-0, and the graph has a solution.

Thus, modifying the ratio M/N changes the number of expected occurrences of a cycle for a graph with
randomly generated edges.

For a small graph such as the one above, determining whether a cycle of a certain length exists
is trivial. But as the graphs get larger, detecting such cycles becomes more difficult. For instance,
does this graph have a cycle of length 8, i.e. 8 connected nodes starting and ending on the same node?

<<<<<<< HEAD
=======
노드 수를 기준으로 가장자리 수를 늘리면 솔루션이있을 확률이 높아집니다. 위의 그래프에 몇 개의 가장자리가 추가되면 0-5-4-1-0에서 길이 4의 사이클이 나타나고 그래프에는 솔루션이 있습니다.

따라서, 비율 M / N을 변경하면 무작위로 생성 된 에지를 갖는 그래프에 대한 사이클의 예상 발생 횟수가 변경됩니다.

위와 같은 작은 그래프의 경우 특정 길이의주기가 존재하는지 여부를 판별하는 것은 쉽지 않습니다. 그러나 그래프가 커질수록 이러한주기를 감지하는 것이 더욱 어려워집니다. 예를 들어,이 그래프는 길이가 8 인 사이클, 즉 동일한 노드에서 시작하고 끝나는 8 개의 연결된 노드입니까?

>>>>>>> ce3f5375948e91d021122bf637998b55c4248fda
![alt text](images/cuckoo_base_numbered_many_edges.png)

*Meat-space Cycle Detection exercise*

The answer is left as an exercise to the reader, but the overall takeaways are:

* Detecting cycles in a graph becomes more difficult exercise as the size of a graph grows.
* The probability of a cycle of a given length in a graph increases as M/N becomes larger,
  i.e. you add more edges relative to the number of nodes in a graph.

<<<<<<< HEAD
=======
대답은 독자에게 연습으로 남겨 두지 만 전반적인 테이크 어웨이는 다음과 같습니다.

그래프의 크기가 커짐에 따라 그래프에서 사이클을 감지하는 것이 더 어려워집니다.

* M / N이 커짐에 따라 그래프에서 주어진 길이의주기가 발생할 확률이 증가하고,

  즉 그래프의 노드 수에 상대적으로 가장자리를 더 추가합니다.

>>>>>>> ce3f5375948e91d021122bf637998b55c4248fda
### Cuckoo Cycle

The Cuckoo Cycle algorithm is a specialized algorithm designed to solve exactly this problem, and it does
so by inserting values into a structure called a 'Cuckoo Hashtable' according to a hash which maps nodes
into possible locations in two separate arrays. This document won't go into detail on the base algorithm, as
it's outlined plainly enough in section 5 of the
[white paper](https://github.com/tromp/cuckoo/blob/master/doc/cuckoo.pdf). There are also several
variants on the algorithm that make various speed/memory tradeoffs, again beyond the scope of this document.
However, there are a few details following from the above that we need to keep in mind before going on to more
technical aspects of Grin's proof-of-work.

* The 'random' edges in the graph demonstrated above are not actually random but are generated by
  putting edge indices (0..N) through a seeded hash function, SIPHASH. Each edge index is put through the
  SIPHASH function twice to create two edge endpoints, with the first input value being 2 * edge_index,
  and the second 2 * edge_index+1. The seed for this function is based on a hash of a block header,
  outlined further below.
* The 'Proof' created by this algorithm is a set of nonces that generate a cycle of length 42,
  which can be trivially validated by other peers.
* Two main parameters, as explained above, are passed into the Cuckoo Cycle algorithm that affect the probability of a solution, and the
  time it takes to search the graph for a solution:
  * The M/N ratio outlined above, which controls the number of edges relative to the size of the graph.
    Cuckoo Cycle fixes M at N/2, which limits the number of cycles to a few at most.
  * The size of the graph itself

How these parameters interact in practice is looked at in more [detail below](#mining-loop-difficulty-control-and-timing).

Now, (hopefully) armed with a basic understanding of what the Cuckoo Cycle algorithm is intended to do, as well as the parameters that affect how difficult it is to find a solution, we move on to the other portions of Grin's POW system.

## Mining in Grin

The Cuckoo Cycle outlined above forms the basis of Grin's mining process, however Grin uses Cuckoo Cycle in tandem with several other systems to create a Proof-of-Work.

### Additional Difficulty Control

In order to provide additional difficulty control in a manner that meets the needs of a network with constantly evolving hashpower
availability, a further Hashcash-based difficulty check is applied to potential solution sets as follows:

If the Blake2b hash
of a potential set of solution nonces (currently an array of 42 u32s representing the cycle nonces,)
is less than an evolving difficulty target T, then the solution is considered valid. More precisely,
the proof difficulty is calculated as the maximum target hash (2^256) divided by the current hash,
rounded to give an integer. If this integer is larger than the evolving network difficulty, the POW
is considered valid and the block is submit to the chain for validation.

In other words, a potential proof, as well as containing a valid Cuckoo Cycle, also needs to hash to a value higher than the target difficulty. This difficulty is derived from:

### Evolving Network Difficulty

The difficulty target is intended to evolve according to the available network hashpower, with the goal of
keeping the average block solution time within range of a target (currently 60 seconds, though this is subject
to change).

The difficulty calculation is based on both Digishield and GravityWave family of difficulty computation,
coming to something very close to ZCash. The reference difficulty is an average of the difficulty over a window of
23 blocks (the current consensus value). The corresponding timespan is calculated by using the difference between
the median timestamps at the beginning and the end of the window. If the timespan is higher or lower than a certain
range, (adjusted with a dampening factor to allow for normal variation,) then the difficulty is raised or lowered
to a value aiming for the target block solve time.

### The Mining Loop

All of these systems are put together in the mining loop, which attempts to create
valid Proofs-of-Work to create the latest block in the chain. The following is an outline of what the main mining loop does during a single iteration:

* Get the latest chain state and build a block on top of it, which includes
  * A Block Header with new values particular to this mining attempt, which are:
    * The latest target difficulty as selected by the [evolving network difficulty](#evolving-network-difficulty) algorithm
    * A set of transactions available for validation selected from the transaction pool
    * A coinbase transaction (which we're hoping to give to ourselves)
    * The current timestamp
    * A randomly generated nonce to add further randomness to the header's hash
    * The merkle root of the UTXO set and fees (not yet implemented)
      * Then, a sub-loop runs for a set amount of time, currently configured at 2 seconds, where the following happens:
        * The new block header is hashed to create a hash value
        * The cuckoo graph generator is initialized, which accepts as parameters:
          * The hash of the potential block header, which is to be used as the key to a SIPHASH function
            that will generate pairs of locations for each element in a set of nonces 0..N in the graph.
          * The size of the graph (a consensus value).
          * An easiness value, (a consensus value) representing the M/N ratio described above denoting the probability
            of a solution appearing in the graph
        * The Cuckoo Cycle detection algorithm tries to find a solution (i.e. a cycle of length 42) within the generated
          graph.
        * If a cycle is found, a Blake2b hash of the proof is created and is compared to the current target
          difficulty, as outlined in [Additional Difficulty Control](#additional-difficulty-control) above.
        * If the Blake2b Hash difficulty is greater than or equal to the target difficulty, the block is sent to the
          transaction pool, propagated amongst peers for validation, and work begins on the next block.
        * If the Blake2b Hash difficulty is less than the target difficulty, the proof is thrown out and the timed loop continues.
        * If no solution is found, increment the nonce in the header by 1, and update the header's timestamp so the next iteration
          hashes a different value for seeding the next loop's graph generation step.
        * If the loop times out with no solution found, start over again from the top, collecting new transactions and creating
          a new block altogether.

### Mining Loop Difficulty Control and Timing

Controlling the overall difficulty of the mining loop requires finding a balance between the three values outlined above:

* Graph size (currently represented as a bit-shift value n representing a size of 2^n nodes, consensus value
  DEFAULT_SIZESHIFT). Smaller graphs can be exhaustively searched more quickly, but will also have fewer
  solutions for a given easiness value. A very small graph needs a higher easiness value to have the same
  chance to have a solution as a larger graph with a lower easiness value.
* The 'Easiness' consensus value, or the M/N ratio of the graph expressed as a percentage. The higher this value, the more likely
  it is a generated graph will contain a solution. In tandem with the above, the larger the graph, the more solutions
  it will contain for a given easiness value. The Cuckoo Cycle implementations fix this M to N/2, giving
  a ratio of 50%
* The evolving network difficulty hash.

These values need to be carefully tweaked in order for the mining algorithm to find the right balance between the
cuckoo graph size and the evolving difficulty. The POW needs to remain mostly Cuckoo Cycle based, but still allow for
reasonably short block times that allow new transactions to be quickly processed.

If the graph size is too low and the easiness too high, for instance, then many cuckoo cycle solutions can easily be
found for a given block, and the POW will start to favour those who can hash faster, precisely what Cuckoo Cycle is
trying to avoid. If the graph is too large and easiness too low, however, then it can potentially take any solver a
long time to find a solution in a single graph, well outside a window in which you'd like to stop to collect new
transactions.

These values are currently set to 2^12 for the graph size and 50% (as fixed by Cuckoo Cycle) for the easiness value,
however the size is only a temporary values for testing. The current miner implementation is very unoptimized,
and the graph size will need to be changed as faster and more optimized Cuckoo Cycle algorithms are put in place.

### Pooling Capability

Contrary to some existing concerns about Cuckoo Cycle's poolability, the POW implementation in Grin as described above
is perfectly suited to a mining pool. While it may be difficult to prove efforts to solve a single graph in isolation,
the combination of factors within Grin's proof-of-work combine to enforce a notion called 'progress-freeness', which
enables 'poolability' as well as a level of fairness among all miners.

#### Progress Freeness

Progress-freeness is central to the 'poolability' of a proof-of-work, and is simply based on the idea that a solution
to a POW problem can be found within a reasonable amount of time. For instance, if a blockchain
has a one minute POW time and miners have to spend one minute on average to find a solution, this still satisfies the POW
requirement but gives a strong advantage to big miners. In such a setup, small miners will generally lose at least one minute
every time while larger miners can move on as soon as they find a solution. So in order to keep mining relatively progress-free,
a POW that requires multiple solution attempts with each attempt taking a relatively small amount of time is desirable.

Following from this, Grin's progress-freeness is due to the fact that a solution to a Cuckoo with Grin's default parameters
can typically be found in under a second on most GPUs, and there is the additional requirement of the Blake2b difficulty check
on top of that. Members of a pool are thus able to prove they're working on a solution to a block by submitting valid Cuckoo solutions
(or a small bundle of them) that simply fall under the current network target difficulty.
