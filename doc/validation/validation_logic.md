# Standalone Validations Logic

## Transaction

* Validate the "transaction body"
  * Validate the total weight, including reward is lower than the consensus max weight
  * Validate the sorting of inputs, outputs and kernels by their hashes
  * Validate the outputs have all been fully cut-through (no inputs matching an included output)
  * Batch verify all output range proofs (but only those that aren't in the validation cache yet)
  * Verify all kernel signatures against the excess and the message (fee+lock_time?)
* Verify no output or kernel include invalid features (coinbase)
* Verify the big "sum": all inputs plus reward+fee, all output commitments, all kernels plus the kernel excess

## Block

* Validate the "transaction body" as with transactions
* Verify no kernels have a future lock height
* Check that the reward plus fees "sums" correctly with the coinbase outputs commitments and kernels excess
* Verify the big "sum": all inputs plus reward+fee, all output commitments, all kernels plus the kernel excess from the header

# Chain validations

Headers and blocks have a quick rejection check first when they've already gone through validation and have either been accepted or definitely rejected (non orphans, no local error).

## Header

In all header difficulty calculations, the difficulty proven by the proof-of-work is subject to adjustments due to either Cuckatoo sizes or Cuckaroo scaling factor.

* Check the version against what we're expecting at the moment given a hard fork schedule.
* Check the header timestamp isn't too far off in the future (12 * block time)
* Check we either have a primary or secondary proof of work solution
* Check the solution is a valid Cuck(ar|at)oo cycle of the required length (42)
* Check the previous block header exists
* Check the heights are coherent (previous + 1)
* Check the header timestamp is strictly greater than the previous
* Check the header PoW total difficulty is greater than the previous one
* Check the header PoW satisfies the network difficulty claimed in the header
* Check the calculated network difficulty equals the header claimed network difficulty
* Check the header secondary scaling factor matches the network calculated one
* Validate the previous header MMR root is correct against the local MMR.

## Block

* Run full header validation
* Check we have the previous full block (orphan otherwise)
* Run standalone block validation
* Validate the MMRs roots and sizes against our header
* Block header goes through full header validation
