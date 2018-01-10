# Coinbase Maturity

Coinbase (block reward + fees) outputs are "locked" and require 1,000 confirmations (blocks added to the current chain) before they mature sufficiently to be spendable. This is to avoid subsequent txs from being at risk of being reversed if a chain reorganization occurs.
Bitcoin does something very similar, requiring 100 confirmations (Bitcoin blocks are every 10 minutes, Grin blocks every 60 seconds).
Grin enforces coinbase maturity in both the transaction pool and the block validation pipeline. A transaction containing an input spending a coinbase output cannot be added to the transaction pool until it has sufficiently matured (based on current chain height and the height of the block producing the coinbase output).
Similarly a block is not valid if it contains an input spending a coinbase output before it has sufficiently matured (based on the height of the new block and the height of the block producing the coinbase output).

The maturity rule only applies to coinbase outputs, regular transaction outputs have an effective lock height of zero.

An output consists of -
  * features (currently coinbase vs. non-coinbase)
  * commitment `rG+vH`
  * switch commitment hash `blake2(rJ)`
  * rangeproof

An input consists of -
  * commitment (reference to output being spent)

[tbd - describe what is required to spend an output]


Grin does not permit duplicate commitments to exist in the UTXO set at the same time.
But once an output is spent it is removed from the UTXO set and a duplicate commitment can be added back into the UTXO set.
This is not necessarily recommended but Grin must handle this situation in a way that does not break consensus across the network.

Several things complicate this situation -

1. It is possible for two blocks to have identical rewards, particularly for the case of empty blocks, but also possible for non-empty blocks with transaction fees.
1. It is possible for a non-coinbase output to have the same value as a coinbase output.
1. It is possible (but not recommended) for a miner to reuse private keys.

Grin does not allow duplicate commitments to exist in the UTXO set simultaneously.
But the UTXO set is specific to the state of a particular chain fork. It _is_ possible for duplicate _identical_ commitments to exist simultaneously on different concurrent forks.
And these duplicate commitments may have different "lock heights" at which they mature and become spendable on the different forks.

* Output O<sub>1</sub> from block B<sub>1</sub> spendable at height h<sub>1</sub> (on fork f<sub>1</sub>)
* Output O<sub>1</sub>' from block B<sub>2</sub> spendable at height h<sub>2</sub> (on fork f<sub>2</sub>)

The complication here is that input I<sub>1</sub> will spend either O<sub>1</sub> or O<sub>1</sub>' depending on which fork the block containing I<sub>1</sub> exists on. And crucially I<sub>1</sub> may be valid at a particular block height on one fork but not the other.

Said another way - a commitment may refer to multiple outputs, all of which may have different lock heights. And we _must_ ensure we correctly identify which output is actually being spent and that the coinbase maturity rules are correctly enforced based on the current chain state.
