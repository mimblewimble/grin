
Mode of Interactions
====================

There's a variety of ways wallet software can be integrated with, from hardware
to automated bots to the more classic desktop wallets. No single implementation
can hope to accommodate all possible interactions, especially if it wants to
remain user friendly (who or whatever the user may be). With that in mind, Grin
needs to provide a healthy base for a more complete wallet ecosystem to
develop.

We propose to achieve this by implementing, as part of the "standard" wallet:

* A good set of APIs that are flexible enough for most cases.
* One or two default main mode of interaction.

While not being exhaustive, the different ways we can imagine wallet software
working with Grin are the following:

1. A receive-only online wallet server. This should have some well-known network
   address that can be reached by a client. There should be a spending key kept
   offline.
1. A fully offline interaction. The sender uses her wallet to dump a file that's
   sent to the receiver in any practical way. The receiver builds upon that file,
   sending it back to the sender. The sender finalizes the transaction and sends it
   to a Grin node.
1. Fully online interaction through a non-trusted 3rd party. In this mode
   receiver and sender both connect to a web server that facilitates the
   interaction. Exchanges can be all be encrypted.
1. Hardware wallet. Similar to offline but the hardware wallet interacts with
   a computer to produce required public keys and signatures.
1. Web wallet. A 3rd party runs the required software behind the scenes and
   handles some of the key generation. This could be done in a custodial,
   non-custodial and multisig fashion.
1. Fully programmatic. Similar to the online server, but both for receiving and
   sending, most likely by an automated bot of some sorts.

As part of the Grin project, we will only consider the first 2 modes of
interaction. We hope that other projects and businesses will tackle other modes
and perhaps even create new ones we haven't considered.

Design Considerations
=====================

Lower-level APIs
----------------

Rust can easily be [reused by other languages](https://doc.rust-lang.org/1.2.0/book/rust-inside-other-languages.html)
like Ruby, Python or node.js, using standard FFI libraries. By providing APIs
to build and manipulate commitments, related bulletproofs and aggregate
signatures we can kill many birds with one stone:

* Make the job of wallet implementers easier. The underlying cryptographic
  concepts can be quite complex.
* Make wallet implementations more secure. As we provide a higher level API,
  there is less risk in misusing lower-level constructs.
* Provide some standardization in the way aggregations are done. There are
  sometimes multiple ways to build a commitment or aggregate signatures or proofs
  in a multiparty output.
* Provide more eyeballs and more security to the standard library. We need to
  have the wallet APIs thoroughly reviewed regardless.

Receive-only Online Wallet
--------------------------

To be receive only we need an aggregation between a "hot" receiving key and an
offline spending key. To receive, only the receiving key should be required, to
spend both keys are needed.

This can work by forming a multi-party output (multisig) where only the public
part of the spending key is known to the receiving server. Practically a master
public key that can be derived similarly to Hierarchical Deterministic wallets
would provide the best security and privacy.

TODO figure out what's needed for the bulletproof. Maybe pre-compute multiple
of them for ranges of receiving amounts (i.e. 1-10 grins, 10-100 grins, etc).

Offline Wallet
--------------

This is likely the simplest to implement, with each interaction dumping its
intermediate values to a file and building off each other.
