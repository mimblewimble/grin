# Dandelion 시뮬레이션

이 문서는 노드의 네트워크가 Dandelion 프로토콜을 트랜잭션 통합(Transaction aggregation)과 함께 사용하는 것에 대해서 설명합니다. 이 시나리오에서 성공적인 (트랜잭션)통합을 시뮬레이션 할 것입니다.
이 문서는 (트랜잭션의) 모든 순간 순간에 대해서 간단히 시각화 하는것을 도와줄것입니다.

## T = 0 - Initial Situation

![t = 0](images/t0.png)

## T = 5

A는 B에게 grin를 보냅니다. A는 거래를 스템풀(stem pool)에 추가하고 이 트랜잭션에 대한 엠바고 타이머를 시작합니다.

![t = 5](images/t5.png)

## T = 10

A는 인내심이 바닥날때까지 기다립니다. ( 아마도 엠바고 타이머가 끝나는 때를 의미하는 듯 - 역자 주)

![t = 10](images/t10.png)

## T = 30

A는 인내심이 바닥나면 동전을 뒤집고 Stem transaction을 G에게 Dandelion을 중계(Relay)합니다. G는 Stem transaction을 받은뒤 Stem pool에 Transaction을 추가하고 이 Transaction의 엠바고 타이머를 시작합니다.

![t = 30](images/t30.png)

## T = 40

G는 E에게 Grin을 보냅니다ㅏ.
G는 이 Transaction을 Stem pool에 Transaction을 추가하고 이 Transaction의 엠바고 타이머를 시작합니다.

![t = 40](images/t40.png)

## T = 45

G는 인내심이 바닥나면 동전을 뒤집고 Stem transaction을 D에게 Dandelion을 중계(Relay)합니다.

![t = 45](images/t45.png)

## T = 50

B는 B1을 D에게 씁니다.
B는 B1을 Stem pool에 추가하고 이 Transaction의 엠바고 타이머를 시작합니다.

![t = 55](images/t55.png)

## T = 55

B는 인내심이 바닥나면 동전을 뒤집고 Stem transaction을 H에게 Dandelion을 중계(Relay)합니다.
D는 인내심이 바닥나면 동전을 뒤집고 통합된(aggregated) Stem transaction을 E에게 Dandelion을 중계(Relay)합니다.
E는 Stem transaction을 받은뒤 Stem pool에 Transaction을 추가하고 이 Transaction의 엠바고 타이머를 시작합니다.

![t = 55](images/t55.png)

## T = 60

H는 인내심이 바닥나면 동전을 뒤집고 Stem transaction을 E에게 Dandelion을 중계(Relay)합니다.
E는 Stem transaction을 받은뒤 Stem pool에 Transaction을 추가하고 이 Transaction의 엠바고 타이머를 시작합니다.

![t = 60](images/t60.png)

## T = 70 - Step 1

E는 인내심이 바닥나면 동전을 뒤집고 transaction을 모든 피어에게 전송하기로 합니다.(mempool안의 fluff 상태)

![t = 70_1](images/t70_1.png)

## T = 70 - Step 2

All the nodes add this transaction to their mempool and remove the related transactions from their stempool.
모든 노드는 이 transaction을 자신의 mempool에 넣고 자신의 stempool 에서 이 transaction과 관련된 transaction을 제거합니다.
![t = 70_2](images/t70_2.png)