**Draft**

This is a first attempt at a table of content for a more exhaustive technical
documentation of Grin (we'd call it a white paper if we had to do an ICO).
This should get progressively filled up, until we're ready to advertize it
more widely.

* What is Grin?
* [Introduction to Mimblewimble](intro.md)
* Cryptographic Primitives
  * Pedersen Commitments
  * Aggregate (Schnorr) Signatures
    * Bulletproofs
* Block and Transaction Format
  * Transaction
    * Input, output
    * Kernel
  * Block
    * Header
    * Body
    * Compact Block
* Chain State and Merkle Mountain Range
  * Motivation
  * [Merkle Mountain Range](mmr.md)
  * [State and Storage](state.md)
  * [Fast Sync](fast-sync.md)
  * Merkle Proofs
* Proof of Work
  * Cuckoo Cycle
  * Difficulty Algorithm
* Wire protocol
  * Seeding and Sync
  * Propagation
  * Low-level Messages
* Dandelion & Aggregation
* Building Transactions
* Important Parameters
  * Fees and Transaction Weight
  * Reward and Block Weight
* [Smart Contracts](contracts.md)
