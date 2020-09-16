# Grin 代码结构

*阅读其它语言版本: [English](../code_structure.md).*

Grin 使用 [Rust]（https://www.rust-lang.org/）编写，这是一个内存安全的编译语言。诸如 Cuckoo 挖掘算法之类的性能关键部分都是作为插件构建的，因此可以轻松地在各种硬件的算法实现之间进行交换。Grin 带有 CPU 和实验性 GPU 支持。

## 项目根目录中的文件

List of files tracked in `git` and some files you'll create when you use grin.
`git` 中跟踪的文件列表以及使用 grin 时将创建的一些文件。

- [CODE_OF_CONDUCT](../CODE_OF_CONDUCT.md) - 如果您想参与到其中，该做些什么。取自 rust，并稍作修改。
- [CONTRIBUTING](../CONTRIBUTING.md) - 如何帮助并参与其中成为 grin 的一部分。
- [Cargo.toml](../Cargo.toml) 和 Cargo.lock（本地创建，*不*在 git 中）- 定义如何编译和构建项目代码。
- [LICENSE](../LICENSE) - Apache 2.0 license
- [README](../README.md) - 您应该阅读的第一个文档，同时它列出了包含更多详细信息的进阶阅读。
- [rustfmt.toml](../rustfmt.toml) - rustfmt 的配置文件。在提交*新*代码之前需要。

## 文件夹结构

在检查了 grin，构建和使用之后，这些是您的文件夹将会有以下内容：

- `api`\
 可通过 REST 访问的 ApiEndpoints 代码。
- `chain`\
 区块链实现，接受一个块（请参阅 pipe.rs）并将其添加到链中，或拒绝它。
- `config`\
 用于处理配置的代码。
- `core`\
 所有核心类型：哈希，块，输入，输出，以及如何对其进行序列化。核心挖掘算法等。
- `doc`\
 所有文档。
- `servers`\
 grin 服务的许多组成部分（adapters, lib, miner, seed, server, sync, types），包括挖矿服务器。
- `keychain`\
 Code for working safely with keys and doing blinding.
- `p2p`\
 所有点对点连接和与协议相关的逻辑（握手，块传播等）。
- `pool`\
 交易池实现的代码。
- `server`\
 在启动服务器之前，您[要创建的文件夹](build_ZH-CN.md)：cd 到项目根目录；mkdir server；cd server；grin server start（或 run），它将创建一个子文件夹 .grin
  - `.grin`
    - `chain` - 具有区块链块和相关信息的数据库
    - `peers` - 一个数据库，其中包含您连接的 Grin peers 节点的列表
    - `txhashset` - 包含内核，范围证明和输出的文件夹，每个文件夹都有一个 pmmr_dat.bin 文件
- `src`\
  构建 grin 可执行文件的代码。
- `store`\
  数据存储 - Grin 在 LMDB（键值嵌入式数据存储）周围使用了接近零成本的 Rust 包装器。
- `target`\
  在编译和构建过程完成之后，grin 的二进制文件所在的位置。
  万一遇到麻烦，请参阅[troubleshooting](https://github.com/mimblewimble/docs/wiki/Troubleshooting)
- `util`\
  底层 rust 工具。
- `wallet`\
  简单的命令行钱包实现。将会创建：
  - `wallet_data` - 储存您“输出”的数据库，一旦被确认并到期，就可以通过 [`grin wallet send`](wallet/usage.md) 命令来花费掉。（本地创建，*不*包含在 git 中)
  - `wallet.seed` - 您的钱包种子。（本地创建，*不*包含在 git 中)

## grin 依赖

- [secp256k1](https://github.com/mimblewimble/rust-secp256k1-zkp)
  libsecp256k1 的集成和 rust 绑定，还有一些更动等待更新。在 util/Cargo.toml 中被导入。
