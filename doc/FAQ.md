# FAQ

- Q: What is Grin?  A: An implementation of [MimbleWimble](https://download.wpsoftware.net/bitcoin/wizardry/mimblewimble.txt)
- Q: Similar to Bitcoin?  A: Both are outputs-based, PoW. Read [Grin for Bitcoiners](grin4bitcoiners.md)
- Q: mining? A: Testnet only. CPU, synchronous. GPU or asynchronous is not yet supported.
- Q: block height? A: HTTP GET /v1/chain on a public peer node, for example http://testnet1.yeastplume.com:13413/v1/chain
- Q: Store of value? A: please don't. We delete testnet coins randomly. And the wallet likes to crash. Developers wecome!
- Q: grin wallet / grin server hangs? A: Yes. Be your own watchdog. Watchdog code for capturing debug log and restarting the hung process - pull reqs welcome.
- Q: Block size limit? Target mean block time?
- Q: Fees? Monetary policy? A: https://github.com/mimblewimble/grin/wiki/fees-mining
- Q: Roadmap? A: Moving fast, changing things. Maybe look at [issues and milestones](https://github.com/mimblewimble/grin/milestones)
- Q: Proof of payment? A: Planned. Maybe in testnet2
- Q: Microtransactions? A: On testnet1, fees are 0.8% on a transaction of 1.0 coins.
- Q: Could Grin ever support or make use of:
  ☑ Probably, or ☐ Probably not
  A: ☑ Contracts, ☑ [Pruning](pruning.md), ☐ Identity, like bitauth, ☑ SNARKs, ☑ [Cross chain atomic swaps, multisig, time locks, lightning network](grin4bitcoiners.md#scripting), ☑ Payment channels, ☑ hidden nodes / onion routing
- Q: HW requirements for mining? A: Not much. Don't invest in equipment yet, there's not even a final beta released, and a lot can change before any official blockchain is launched.
- Q: Quantum safe?  A: Should be. In every Grin output, we also include a bit of hashed data, which is quantum safe. If quantum computing was to become a reality, we can safely introduce additional verification that would protect existing coins from being hacked. [Read more](https://github.com/mimblewimble/grin/blob/master/doc/grin4bitcoiners.md)

# Troubleshooting

## Coins are 'confirmed but still locked'?
Like other cryptocurrencies, newly mined coins are time locked, so mined coins can't be spent immediately.

## "Peer request error" or other peer/network issues after restarting grin server
Possible workaround is rm -rf .grin/peers/*  then restart.

## Build error: Could not compile `tokio-retry`.
You need the latest rust. rustup, or [reinstall rust as described](build.md)

# Short term plans
## Transaction types
- (DONE) A temporary simple transaction exchange. Temporary - will be deprecated.
- (months) Maybe in testnet2 Full transaction; an exchange which involves a full roundtrip between sender and receiver using aggregate (Schnorr) signatures. Usable as proof of payment and for multisig.

