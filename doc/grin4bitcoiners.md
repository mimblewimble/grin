# Grin/Mimblewimble for Bitcoiners

*Read this in other languages: [Korean](translations/grin4bitcoiners_KR.md), [Chinese](translations/grin4bitcoiners_ZH-CN.md)*

This page is a short comparison for readers familiar with Bitcoin. For a deeper treatment of what Grin does and does not hide, see the
[Grin Privacy Primer](https://github.com/mimblewimble/docs/wiki/Grin-Privacy-Primer).

## Privacy and Fungibility

Grin transactions are private in ways that Bitcoin’s transparent UTXO model is not. Three properties matter most:

1. **No on-chain addresses.** Outputs are one-shot curve points. There is no reusable on-chain “payment address” (no Bitcoin-style address or scriptPubKey reuse) that links payments over time. Wallets may still use an *off-chain* contact handle for interaction—for example a slatepack address, Tor onion, or account public key—but that identifier never appears on the chain as a field tying outputs together.
2. **No amounts.** Every transaction uses confidential transactions (CT). Amounts sit inside [Pedersen commitments](intro.md#balance) with [range proofs](intro.md#range-proofs) (Grin uses [Bulletproofs](https://eprint.iacr.org/2017/1066.pdf)), so outsiders cannot read transfer values.
3. **Cut-through and aggregation.** When an output is spent, Mimblewimble can remove the spent input/output pair from the long-term chain state. Inside a block, many transactions are merged into one aggregate set of inputs, outputs, and kernels, so a confirmed block does not look like a list of labeled payments.

Because of (1) and (2), unspent outputs and kernels all look like random-looking data unless you participated in building them. Nodes can still verify that no money was created out of thin air by checking that commitments balance (homomorphic structure) and that range proofs and signatures are valid.

### What cut-through does *not* erase

It is easy to overstate the third point. **Cut-through improves scalability and reduces long-term history, but it does not make input↔output linking impossible.**

- While a transaction is **relayed** (before or while it is mined), observers can still see which commitments are spent and which new ones appear. That is a real information channel.
- **Taint / hop analysis** can follow known “marked” outputs across spends if an adversary can introduce or learn those outputs (for example by paying you, or by watching the network closely). Aggregation and Dandelion make this harder, but they are not perfect anonymity. Interactive constructions such as **payjoins** (both parties contribute inputs so a simple “one payer → one payee + change” pattern is less obvious) can further blur linkage; that work lives in wallet and contract protocols rather than consensus rules.
- After cut-through, the **kernel** of a transaction remains as a permanent ~100-byte footprint. The chain still records that “some” balanced state change occurred, even when the intermediate UTXOs are gone.

So: Grin hides amounts and *on-chain* addresses strongly; it **weakens but does not eliminate** graph-style analysis of which outputs feed which later spends. That matches the Privacy Primer’s view that addresses and amounts are checked off as strong wins, while **input/output linking** remains an area of residual leakage and ongoing research.

## Scalability

Because spent outputs can be removed from the active UTXO set, a Grin node’s long-term storage grows primarily with **unspent outputs** (users holding coins), not with every historical payment. Full verification still needs kernels and headers; kernels are the main “per-transaction” residue that remains after cut-through.

In practice the chain stays far smaller than a transparent UTXO chain with the same activity, but it is not free—range proofs and kernels have a cost, and sync still requires downloading state (for example via PIBD or a txhashset snapshot).

## Scripting

Mimblewimble does not carry Bitcoin-style scripts on the chain. Many contracts that Bitcoin implements with Script can still be built using elliptic-curve techniques and interactive protocols, for example:

* Multi-signature transactions
* Atomic swaps
* Time-locked transactions and outputs
* Payment-channel style constructions (e.g. Lightning-like designs)
* Payjoin-style collaborative transactions (wallet/contract protocol work)

Those constructions live more in wallet protocols than in on-chain script programs.

## Emission Rate

Bitcoin’s block subsidy halves over time toward a fixed supply. Grin’s base emission is **linear** (a constant block reward), so supply grows without a hard cap. Dilution trends toward zero as the monetary base grows, and coins are also lost over time. The block target is on the order of one minute (see current consensus parameters for the exact reward).

## FAQ

### Wait, what!? No address?

Correct—there is **no reusable on-chain address** on outputs. Each output is a unique commitment; nothing like a Bitcoin address is written into the UTXO and reused across payments.

That is different from how wallets *find each other*. To build a transaction interactively, one party still needs a way to contact the other—often a public key, slatepack address, Tor hidden service, or similar. Those are **interaction endpoints**, not on-chain address fields. Both parties do not need to be online at the same instant; the handshake can happen over any private channel (including offline media).

### If transaction information gets removed, can I just cheat and create money?

No. Confidential transactions are designed so nodes can check that the sum of inputs equals the sum of outputs plus the fee **without** learning the amounts. Across the whole chain, nodes also check consistency of total supply against coinbase issuance. Cut-through removes spent UTXOs; it does not remove the need for those balance checks.

### If I listen to transaction relay, can’t I just figure out who they belong to before cut-through?

You can observe **which** outputs are spent and **which** new outputs appear in a given transaction or stem. You generally **cannot** read amounts, and there is no on-chain reusable address to label “who paid whom” the way a Bitcoin explorer does from scriptPubKeys.

What you *can* still do, with extra information, is **link** spends: for example by paying someone a known output and watching how related commitments move, or by combining relay timing with other metadata. Dandelion stem relay reduces the reliability of “this IP originated this tx,” and stem aggregation can blur some couplings, but they do not make the network a black box.

For a careful split of “what Grin hides” vs “what still leaks,” read the [Privacy Primer](https://github.com/mimblewimble/docs/wiki/Grin-Privacy-Primer).

### What about quantum computers?

Commitments and signatures used today are not quantum-safe in the usual sense. Outputs also carry hash material that can support future migration plans if quantum threats become practical. Even if a quantum attacker could strip the cryptographic hiding of historical commitments, amounts are not known in advance—so there is no reliable way to pick “high-value” transactions to attack first. Treat long-range quantum risk like other cryptocurrencies: monitor standards and upgrades; do not assume current crypto is forever.

### How does all this magic work?

See our [technical introduction](intro.md) and the [Privacy Primer](https://github.com/mimblewimble/docs/wiki/Grin-Privacy-Primer).
