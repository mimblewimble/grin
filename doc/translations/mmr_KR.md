# Merkle Mountain Ranges

*다른 언어로 되어있는 문서를 읽으려면: [English](../mmr.md), [简体中文](mmr_ZH-CN.md).*

## MMR의 구조

Merkle Mountain Ranges [1]은 Merkle trees [2]의 대안입니다. 후자는 완벽하게 균형 잡힌 이진 트리를 사용하지만 전자는 완벽하게 균형잡힌 binary tree list 거나 오른쪽 상단에서 잘린 single binary tree로 볼 수 있습니다. Merkle Mountain Range (MMR)는 엄격하게 append 에서만 사용됩니다. 원소는 왼쪽에서 오른쪽으로 추가되고, 두 하위 원소가 있는 즉시 부모를 추가하여 그에 따라 범위를 채웁니다.

다음 그림은 각 노드를 삽입 순서대로 표시한 것입니다. 11 개의 삽입 된 리프와 총크기 19가 있는 range 를 표시합니다.

```
Height

3              14
             /    \
            /      \
           /        \
          /          \
2        6            13
       /   \        /    \
1     2     5      9     12     17
     / \   / \    / \   /  \   /  \
0   0   1 3   4  7   8 10  11 15  16 18
```

이 구조는 편평한 리스트로 표시할 수 있습니다. 여기서는 각 노드의 삽입 포지션에서 노드의 높이를 나타냅니다. ( 위의 그림과 아래의 평면 리스트를 비교하면 이해가 쉬움 - 역자 주 )

```
0  1  2  3  4  5  6  7  8  9 10 11 12 13 14 15 16 17 18
0  0  1  0  0  1  2  0  0  1  0  0  1  2  3  0  0  1  0
```

이 구조는 크기(19)에서 간단히 설명 할 수 있습니다. 빠른 binary operation을 사용하기 때문에 MMR 내에서 탐색하는 것도 매우 간단합니다. 주어진 노드의 위치`n`이라면 이 MMR의 높이, 부모의 위치, 형제 등을 계산할 수 있습니다.

## Hashing 과 Bagging

Merkle tree와 마찬가지로 MMR의 부모 노드는 자신의 두 하위 노드의 해시 값을 가지고있습니다. Grin은 전체적으로 Blake2b 해시 함수를 사용하고 충돌을 피하기 위해 해싱하기 전에 항상 노드의 위치를 ​​MMR에 추가합니다. 따라서 데이터 `D`를 저장하는 인덱스 ​`n` 에 리프노드 `l`이 있는 경우엔  (예를 들자면 출력의 경우 해당 데이터는 Pedersen commitment 입니다), 다음과 같이 표시됩니다.

```
Node(l) = Blake2b(n | D)
```

인덱스 `m`에서 어떤 부모 `p`라면 다음과 같습니다.

```
Node(p) = Blake2b(m | Node(left_child(p)) | Node(right_child(p)))
```

Merkle 트리와는 달리 MMR은 일반적으로 하나의 root를 구성하지 않으므로 계산할 방법이 필요합니다. 그렇지 않으면 해시 트리를 사용하는 목적이 무의미 합니다. 이 과정은 [1]에서 설명한 이유 때문에 "bagging the peaks" 라고 합니다.

먼저 MMR의 최고 높이를 확인합니다. (여기서 확인 할 수 있는 한 가지 방법을 정의 할 것입니다.) 일단 다른 작은 예제 MMR을 보여드리겠습니다. 인덱스는 1부터 시작하고 10 진수가 아닌 2 진수로 작성됩니다.

```
Height

2        111
       /     \
1     11     110       1010
     /  \    / \      /    \
0   1   10 100 101  1000  1001  1011
```

이 MMR에는 11 개의 노드가 있으며 그 피크는 111(7), 1010(10) 및 1011(11) 입니다. 먼저 이진법으로 표현 될 때 가장 왼쪽 첫번째 피크가 항상 가장 (위치가) 높고 항상 "모두 1"이 되는 것에 주목하세요. 그럼 이 피크는 `2^n - 1` 형태의 위치를 ​​가질 것이고 항상 MMR 내부에있는 가장 높은 위치 (그 위치는 전체 크기보다 작습니다)입니다. 사이즈 11의 MMR에 대해서도 반복적으로 처리합니다.

```
2^0 - 1 = 0, and 0 < 11
2^1 - 1 = 1, and 1 < 11
2^2 - 1 = 3, and 3 < 11
2^3 - 1 = 7, and 7 < 11
2^4 - 1 = 15, and 15 is not < 11
```

Finally, once all the positions of the peaks are known, "bagging" the peaks
consists of hashing them iteratively from the right, using the total size of
the MMR as prefix. For a MMR of size N with 3 peaks p1, p2 and p3 we get the
final top peak:

따라서 첫 번째 피크는 7입니다. 다음 피크를 찾으려면 오른쪽 형제에게 "점프" 해야 합니다. 해당 노드가 MMR에 없다면 왼쪽 하위 노드를 가져옵니다. 만약 그 하위노드도 MMR에 없으면, MMR에 있는 노드를 가져올 때까지 왼쪽에 있는 하위 노드를 계속 가져옵니다. 다음 피크를 찾으면 마지막 노드에 도달 할 때까지 프로세스를 반복합니다.

이 모든 오퍼레이션은 매우 간단합니다. 높이 `h`에 있는 노드의 오른쪽 형제로 점프하는 것은 `2^(h + 1)-1`만큼을 높이 `h`에 추가하는 것입니다. 왼쪽 형제를 가져 가면 `2^h`만큼 값을 뺍니다.

마지막으로 피크의 모든 위치를 알게되면 피크들을 "bagging"하는 것은 MMR의 전체 크기를 prefix로 사용해서 반복적으로 오른쪽에서부터 해싱하는 것으로 구성됩니다. 3 개의 피크 p1, p2 및 p3을 갖는 크기 N의 MMR에 대해, 다음과 같은 최종 최고 피크를 얻습니다. :


```
P = Blake2b(N | Blake2b(N | Node(p3) | Node(p2)) | Node(p1))
```

## Pruning

Grin에서는 해시되고 MMR에 저장되는 많은 데이터가 결국 제거 될 수 있습니다. 이런 일이 발생하면 해당 MMR안의 일부 리프 해시가 있는지 여부가 불필요하고 리프의 해시를 제거 될 수 있습니다. 충분한 리프가 제거되면 부모의 존재도 불필요하게 될 수 있습니다. 따라서 우리는 그 리프들의 삭제로 인해 MMR의 상당 부분을 pruning 할 수 있습니다.

MMR의 pruning은 간단한 반복 프로세스에 의존합니다. `X`는 우선 첫번째 제거할 리프로 초기화됩니다.

1. `X`를 Pruning 한다.
1. 만약 `x`가 형제 노드가 있다면 여기서 prunging을 중단한다.
1. 만약 `X`가 형제 노드가 없다면 `X`의 부모 노드는 `X`라고 배정된다.

결과를 시각화하기 위해 첫 번째 MMR 예시에서 시작하여 리프[0, 3, 4, 8, 16]을 제거하면 다음과 같은 pruning MMR이 발생합니다.


```
Height

3             14
            /    \
           /      \
          /        \
         /          \
2       6            13
       /            /   \
1     2            9     12     17
       \          /     /  \   /
0       1        7     10  11 15     18
```

[1] Peter Todd, [merkle-mountain-range](https://github.com/opentimestamps/opentimestamps-server/blob/master/doc/merkle-mountain-range.md)

[2] [Wikipedia, Merkle Tree](https://en.wikipedia.org/wiki/Merkle_tree)
