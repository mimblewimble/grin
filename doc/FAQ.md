# FAQ

- Q: What is grin?  A: An implementation of [MimbleWimble](https://download.wpsoftware.net/bitcoin/wizardry/mimblewimble.txt)
- Q: Similar to Bitcoin?  A: Both are outputs-based, PoW. See also [Grin for Bitcoiners](grin4bitcoiners.md)
- Q: Mining? A: Testnet only. CPU, synchronous. GPU or asynchronous is not yet supported.
- Q: Block height? A: HTTP GET /v1/chain on a public peer node, for example http://testnet1.yeastplume.com:13413/v1/chain or (grintest.net)[https://grintest.net/]
- Q: Store of value? A: Not yet. Wait for Mainnet. Testnet1 can still disappear and reappear unexpectedly.
- Q: Block size limit? Target mean block time?  A: Target mean block time is 1 block per 60 seconds. The size is limited by transaction "weight", though there is also a hard cap on the order of tens of MB.
- Q: Does grin scale?  A: Yes, it might eventually do, thanks to transaction cut-through and possible level 2 solutions.
- Q: Fees? Monetary policy? A: https://github.com/mimblewimble/grin/wiki/fees-mining
- Q: Roadmap? A: Moving fast, changing things. Maybe look at [issues and milestones](https://github.com/mimblewimble/grin/milestones)
- Q: Proof of payment? A: Planned. Maybe in Testnet2
- Q: Microtransactions? A: On Testnet1, fees are 0.8% on a transaction of 1.0 coins.
- Q: Could grin ever support or make use of:
  ☑ Probably, or ☐ Probably not
  A: ☑ Contracts, ☑ [Pruning](pruning.md), ☐ Identity, like bitauth, ☑ SNARKs, [☑ Cross chain atomic swaps, ☑ multisig, ☑ time locks, ☑ lightning network](grin4bitcoiners.md#scripting), ☑ Payment channels, ☑ hidden nodes / onion routing, ☑ [Scripting - clean & native w/ tiny limits](https://lists.launchpad.net/mimblewimble/msg00029.html)
- Q: HW requirements for mining? A: Not much. Don't invest in equipment yet, there's not even a final beta released, and a lot can change before any official blockchain is launched.
- Q: Quantum safe?  A: No. Given sufficient warning, some QC resistance can be introduced through softforks.

# Troubleshooting

## Coins are 'confirmed but still locked'?
Like other cryptocurrencies, newly mined coins are time locked, so mined coins can't be spent immediately.

## "Peer request error" or other peer/network issues after restarting grin server
Possible workaround is rm -rf .grin/peers/*  then restart.

## grin server or waller crashes or hangs
Yes, this still happens quite often. You'll need to babysit grin.
Very welcome any solutions to give grin a "watchdog" solution that can restart
grin in case of trouble.

## Build error: Could not compile `tokio-retry`.
You need the latest rust. rustup, or [reinstall rust as described](build.md)

# Short term plans
## Transaction types
- (DONE) A temporary simple transaction exchange. Temporary - will be deprecated.
- (months) Maybe in testnet2 Full transaction; an exchange which involves a full roundtrip between sender and receiver using aggregate (Schnorr) signatures. Usable as proof of payment and for multisig.
