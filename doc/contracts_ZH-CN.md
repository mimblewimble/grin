# 合约

本文说明使用 Grin 设置智能合约。尽管 Grin 链上不支持脚本，依靠一些链上内置基本功能，即可实现这些智能合约，而且编辑方式也会越来越巧妙。

这些构建方式并非本文作者或 Grin 开发团队原创。主要都是众多密码学家和研究者的结晶。其中包括：Torben
Pryds Pedersen、Gregory Maxwell、Andrew Poelstra、John Tromp、Claus Peter
Schnorr。特此对未能列出名字的贡献者致歉，我们认可多数计算机科学发现都意义非凡。

## 内置功能

本节提及 Grin 区块链的一些重要功能。我们需要预习一些知识，才能构建和使用这些功能。

### Pedersen Commitments

所有输出包括`r*G + v*H`公式的 Pedersen commitment。`r`是盲因子，`v`是值，`G`和 `H` 是相同曲线组上两个不同的生成器点。

### 聚合签名（即 Schnorr 签名，多签）

我们假设有 SHA256 哈希函数和与上述相同的 G 曲线。最简单公式中，聚合签名构建需要以下条件：

* 待签名信息 `M`，本例中是交易费
* 私钥 `X`，对应公钥 `x*G`
* 随机数 `K`，仅用于构建签名

我们构建一个挑战 `e = SHA256(M | k*G | x*G)` 和标量 `s = k + e * x`。完整的聚合签名就为 `(s, k*G)`。

检查签名可使用公钥 `x*G` ，使用签名对后半部分 M 和 `k*G` 重新计算 `e`，验证签名对第一部分 `s` 满足：

```
s*G = k*G + e * x*G
```

简单示例，甲给信任乙，给对方发起一笔转账（稍后举例无需信任的情况）。使用上述私钥 `X` 计算输出盲因子之和减去输入盲因子之和，可直接为 Grin 构建聚合签名。使用 `r` 和公钥 `r*G` 产生的聚合签名生成最终的交易核，允许验证所有 Grin 交易没有非法造币（并且给交易费签名）。

仅使用标量和公钥即可构建签名，那也可使用简单的数理构建不同的合约。

### （绝对）限时锁定交易

类似于比特币 [nLockTime](https://en.bitcoin.it/wiki/Timelock#nLockTime)。

仅需简单修改即可对交易进行限时锁定：

* 待签名信息 `M` 锁定在高度 (lock_height) `h`，交易即可花费，并添加交易费
  * `M = fee | h`
* 锁定高度 `h` 写入交易核
* 锁定高度大于目前区块高度的交易核区块会被拒绝

### （相对）限时锁定交易

纳入可以确定相关锁定高度的（交易核）commitment，就可以延伸交易的绝对限时锁定概念。

只要参照交易内核首先加入到链上状态，锁定高度就与区块高度关联。

只要（通过参照交易内核 commitment）首先查看 Tx1 后 `h` 区块通过，可以限制 Tx2，只有纳入一个区块后方可有效。

* 待签名信息 `M` 需要包含以下信息 -
  * 与之前一样的 `fee`
  * lock_height `h` （与之前一样，只是解释为相对值）
  * 参照交易内核 commitment `C`
  * M = `fee | h | C`

要让 Tx2 接受，就需要包含 Merkle 证明，验证区块包含 Tx1 的 `C`。这就证明已满足相对 lock_height 要求。

## 衍生合约

### 无需信任交易

涉及一方的聚合 (Schnorr) 签名相对简单，但没有展现架构的全部灵活性。下面我们来展示如何用于多方输出。

如 1.2 节所示，聚合签名需要信任收款方。由于 Grin 的输出完全通过 Pedersen Commitment 隐藏，付款方无法证明钱准确无误发给一方，因此收款方可以说自己没有收到钱。为了解决这个问题，我们需要收款方与付款方交互进行交易，特别是对交易内核签名。

Alice 想要给 Bob 支付 Grin. Alice 开始交易构建流程：

1. Alice 选择输入值，建立找零输出。所有盲因子之和（找零输出减去输入）为 `rs`。
1. Alice 选择一个随机数 ks，发送她这部分交易 `ks*G` 和 `rs*G` 给 Bob。
1. Bob 选出自己的随机数 `kr` 和输出盲因子 `rr`，Bob 使用 `rr` 将自己的输出添加到交易。
1. Bob 算出信息 `M = fee | lock_height`，Schnorr 挑战 `e = SHA256(M | kr*G + ks*G | rr*G + rs*G)` 以及最后他这边的签名 `sr = kr + e * rr`。
1. Bob 将 `sr`, `kr*G` 和 `rr*G` 发给 Alice。 
1. Alice 像 Bob 一样算出 `e`，然后检查 `sr*G = kr*G + e*rr*G`。
1. Alice 将她的签名 `ss = ks + e * rs` 发给 Bob。
1. Bob 按第六步 Alice 验证 `sr*G` 一样来验证 `ss*G`，并生成最终签名 `s = (ss + sr, ks*G + kr*G)` 和包含 `s` 与公钥 `rr*G + rs*G` 的最终交易内核。

协议需要三步数据交换（Alice 发送交易文件给 Bob，Bob 再发送给 Alice，最后 Alice 再发送给 Bob），也就是上述所讲的交互。但交互也可以在特定时间内通过媒介来完成，包括两周时间的“慢速邮递”（pony express）。

本协议也可归纳为双方的任意数字 `i`。第一轮交互中，`ki*G` 和 `ri*G` 共享。第二轮中，双方都可以计算 `e = SHA256(M | sum(ki*G) | sum(ri*G))` 和自己的签名 `si`。最后确认方可以搜集全部分散签名 `si`，验证并生成 `s = (sum(si), sum(ki*G))`。

### 多方输出（多签）

本节说明建立只有多方同意才能花费的交易。此构建与之前的无需信任交易类似，但本例中需要聚合签名和 Pedersen Commitment。

这次，Alice 发起交易需要获得  Bob 和自己同意才能花费。Alice 正常发起交易，并以下列方式添加多方输出：

1. Bob 选出盲因子 `rb` 并发送 `rb*G` 给 Alice。
1. Alice 选出盲因子 `ra` 并建立秘诺 (commitment) `C = ra*G + rb*G + v*H`，并将秘诺发送给 Bob。
1. Bob 用 `C` 和 `rb` 建立 `v` 的范围证明 (range proof)，并发送给 Alice。
1. Alice 生成自己的范围证明，并将其与 Bob 的聚合，确认多方输出 `Oab`。
1. 之后与的操作“无需信任交易”一样。

我们注意到，双方都不知道新输出 `Oab` 的全部盲因子。要发起花费 Oab 的交易，就有人得知道 `ra + rb` 来生成交易核签名。Alice 和 Bob 需要合作才能要生成交易核。这又是利用与“无需信任交易”类似的协议完成。

### 多方限时锁定

本合约是其他多种合约的基础。本例中，Alice 同意锁定一些基金，用于与 Bob 进行财务往来，并向 Bob 证明自己资金充足。合约设定如下：

* Alice 用与 Bob 分享的输出发起两人签名两份私钥多方交易，但他不参与发起交易内核签名。
* Bob 用限时锁定（1440 区块之后约 24 小时）发起给 Alice 的退款交易。
* Alice 和 Bob 发起相应的交易内核完成两人签名交易，并向全网广播。

现在 Alice 和 Bob 可以用两人签名输出随意发起其他交易。如果 Bob 拒绝，Alice 只需要在锁定到期后向全网广播退款交易。

此合约一般可用于单向支付通道。

### 限定条件输出限时锁定

类似于比特币的 [CheckLockTimeVerify](https://en.bitcoin.it/wiki/Timelock#CheckLockTimeVerify)。

我们目前交易中有限定条件 lock_heights （超过 lock_height 后交易无效且不会被接收）

私钥可相加。Key<sub>3</sub> = Key<sub>1</sub> + Key<sub>2</sub>

Commitments 可相加。C<sub>3</sub> = C<sub>1</sub> + C<sub>2</sub>

关于_交易限定条件限时锁定_ ，我们可以利用这些特性，将两笔相关交易的两笔输出关联，来获得_限定条件输出限时锁定_。 

我们可以以两笔关联的输出 Out<sub>1</sub> 和 Out<sub>2</sub> 构建两笔交易 (Tx<sub>1</sub>, Tx<sub>2</sub>)，例如-

* 输出 Out<sub>1</sub> (commitment C<sub>1</sub>) 来自 Tx<sub>1</sub>，使用密钥 Key<sub>1</sub> 构建
* 输出 Out<sub>2</sub> (commitment C<sub>2</sub>) 来自 Tx<sub>2</sub>，使用 Key<sub>2</sub> 构建
* 交易 Tx<sub>2</sub> 有_限定条件_  lock_height

如果我们这样做（并按照需要管理密钥） -

* Out<sub>1</sub> + Out<sub>2</sub> _只能_ 使用密钥 Key<sub>3</sub> 匹配花费
* _只能_ 在 lock_height 后从 Tx<sub>2</sub> 花费

Tx<sub>1</sub> (包含 Out<sub>1</sub>) 可以立即广播到全网，接收并在链上确认。Tx<sub>2</sub> 只有在超过 lock_height 后才能全网广播和接收。

如果 Alice 只知道 K<sub>3</sub>，不知道 Key<sub>1</sub> 或 Key<sub>2</sub>，那么在超过 lock_height 之后，Alice 才能花费 输出 Out<sub>1</sub>。如果 Bob 知道 Key<sub>2</sub>，那么 Bob 可以立即花费输出 Out<sub>1</sub>。

我们对输出 Out<sub>1</sub> 有_限定条件_限时锁定（已在链上确认），可以l使用私钥 Key<sub>3</sub> (lock_height 后) 或私钥 Key<sub>2</sub> 立即花费。

### （相对）限定条件输出限时锁定

类似于比特币的 [CheckSequenceVerify](https://en.bitcoin.it/wiki/Timelock#CheckSequenceVerify).

将“限定条件输出限时锁定”与“（相对）限定条件输出限时锁定”混合，我们可以得出有相对限时锁定的确认输出（相对于相关的交易内核）。

可立即全网广播、接受并链上确认交易 Tx<sub>1</sub> (包含输出 Out<sub>1</sub>)。 相对于之前交易 Tx<sub>1</sub> 的参照交易内核，只有超过_相对_ lock_height，才能广播和接受交易 Tx<sub>2</sub>。

### 原子互换

原子互换可以在比特币、以太坊及其他可行的链上部署。这一功能依赖于限时锁定合约，外加检查两个公钥。比特币上这需要两份私钥两人签名，一份公钥是 Alice 的，一份是 Bob 必须公开的原像 (preimage) 哈希值。本设置中，我们要考虑公钥衍生 `x*G` 为哈希函数。而且 Bob 公开 `x`，Alice 可提供完整签名证明她知道 `x`（除了自己的私钥）。

Alice 有 Grin，Bob 有比特币。他们要互换。我们假设 Bob 在比特币链上创建一个输出，允许 Alice 知道原像 `x` 后花费，或 Bob 在限定时间 `Tb` 到期后花费。Alice 准备在 Bob 公开 `x` 后将她的 Grin 发送给 Bob。

首先 Alice 要把她的 Grin 发送到一个多方限时锁定合约中，设定退款时间 `Ta < Tb`。若要将两人签名两份私钥输出发送给 Bob 并执行互换，Alice 和 Bob 开始要按第 2.1 节所示发起正常无需信任交易。

1. Alice 挑选一个随机数 `ks` 和盲因子之和 `rs`，并发送 `ks*G` 和 `rs*G` 给 Bob。
1. Bob 挑选一个随机盲因子 `rr` 和随机数 `kr`。但这次 Bob 不再是仅仅发送 `sr = kr + e * rr` 和他的 `rr*G` 与 `kr*G`，而是发送 `sr' = kr + x + e * rr` 与 `x*G`。
1. Alice 可验证 `sr'*G = kr*G + x*G + rr*G`。她也可以检查 Bob 在其他链用 `x*G` 锁定钱。
1. 既然 Alice 也可以计算 `e = SHA256(M | ks*G + kr*G)`，Alice 按正常操作将 `ss = ks + e * xs` 发送回给 Bob。
1. 要完成签名，需要 Bob 计算 `sr = kr + e * rr`，最后的签名是 `(sr + ss, kr*G + ks*G)`。
1. 只要 Bob 一广播最后的交易获得了他的 Grin，Alice 就可计算 `sr' - sr` 获得 `x`。

#### 比特币设置事项

在完成原子互换之前，Bob 需要知道 Alice 的公钥。Bob 之后会在比特币链上创建一个输出，用类似 `alice_pubkey secret_pubkey 2 OP_CHECKMULTISIG` 的双私钥多签。输出会写入 `OP_IF`，这样在双方同意的时间内 Bob 可以拿回钱。而且所有甚至都可以写入 P2SH。此处 `secret_pubkey` 就是上节的 `x*G`。

要验证输出，Alice 需要接收 `x*G`，创建比特币脚本，算出哈希值，查看 P2SH 中的哈希匹配（上节第二步）。Alice 得到 `x`（第六步）后，即可建立花费双私钥多签输出的两个签名，获得两个私钥，最后获得比特币。

### 哈希限时锁定（闪电网络）

相对锁定时间待开发
