Introduction to MimbleWimble and Grin
=====================================

MimbleWimble is a blockchain format and protocol that provides
extremely good scalability, privacy and fungibility by relying on strong
cryptographic primitives. It addresses gaps existing in almost all current
blockchain implementations.

Grin is an open source software project that implements a MimbleWimble
blockchain and fills the gaps required for a full blockchain and
cryptocurrency deployment.

The main goal and characteristics of the Grin project are:

* Privacy by default. This enables complete fungibility without precluding
	the ability to selectively disclose information as needed.
* Scales with the number of users and not the number of transactions, with very
  large space savings compared to other blockchains.
* Strong and proven cryptography. MimbleWimble only relies on Elliptic Curve
  Cryptography which has been tried and tested for decades.
* Design simplicity that makes it easy to audit and maintain over time.
* Community driven, using an asic-resistant mining algorithm (Cuckoo Cycle)
  encouraging mining decentralization.

# Tongue Tying for Everyone

This document is targeted at readers with a good
understanding of blockchains and basic cryptography. With that in mind, we attempt
to explain the technical buildup of MimbleWimble and how it's applied in Grin. We hope
this document is understandable to most technically-minded readers. Our objective is
to encourage you to get interested in Grin and contribute in any way possible.

To achieve this objective, we will introduce the main concepts required for a good
understanding of Grin as a MimbleWimble implementation. We will start with a brief
description of some relevant properties of Elliptic Curve Cryptography (ECC) to lay the
foundation on which Grin is based and then describe all the key elements of a
MimbleWimble blockchain's transactions and blocks.

## Tiny Bits of Elliptic Curves

We start with a brief primer on Elliptic Curve Cryptography, reviewing just the
properties necessary to understand how MimbleWimble works and without
delving too much into the intricacies of ECC. For readers who would want to
dive deeper into those assumptions, there are other opportunities to
[learn more](http://andrea.corbellini.name/2015/05/17/elliptic-curve-cryptography-a-gentle-introduction/).

An Elliptic Curve for the purpose of cryptography is simply a large set of points that
we will call _H_. On those points,
the addition and multiplication operations have been defined, just like we know how
to do additions and multiplications on numbers or vectors. Given a number _k_ and
using the multiplication operation we can compute `k*H`, which is also a point on
_H_. Given another number _j_ we can also calculate `(k+j)*H` which is equivalent
to `k*H + j*H`. The addition and multiplication operations on an elliptic curve
maintain the commutative and associative properties of addition and multiplication:

    (k+j)*H = k*H + j*H

In ECC, if we pick a very large number _k_ as a private key, `k*H` is
considered the corresponding public key. Even if one knows the
value of the public key `k*H`, deducing _k_ is close to impossible (or said
differently, while multiplication is trivial, "division" by curve points is
extremely difficult).

The previous formula `(k+j)*H = k*H + j*H`, with _k_ and _j_ both private
keys, demonstrates that a public key obtained from the addition of two private
keys (`(k+j)*H`) is identical to the addition of the public keys for each of those
two private keys (`k*H + j*H`). In the Bitcoin blockchain, Hierarchical
Deterministic wallets heavily rely on this principle. MimbleWimble and the Grin
implementation do as well.

## Transacting with MimbleWimble

The structure of transactions demonstrates a crucial tenet of MimbleWimble:
strong privacy and confidentiality guarantees.

The validation of MimbleWimble transactions relies on two basic properties:

* **Verification of zero sums.** The sum of outputs minus inputs always equals zero,
proving that the transaction did not create new funds, _without revealing the actual amounts_.
* **Possession of private keys.** Like with most other cryptocurrencies, ownership of
transaction outputs is guaranteed by the possession of ECC private keys. However,
the proof that an entity owns those private keys is not achieved by directly signing
the transaction.

The next sections on balance, ownership, change and proofs details how those two
fundamental properties are achieved.

### Balance

Building upon the properties of ECC we described above, one can obscure the values
in a transaction.

If _v_ is the value of a transaction input or output and _H_ an elliptic curve, we can simply
embed `v*H` instead of _v_ in a transaction. This works because using the ECC
operations, we can still validate that the sum of the outputs of a transaction equals the
sum of inputs:

    v1 + v2 = v3  =>  v1*H + v2*H = v3*H

Verifying this property on every transaction allows the protocol to verify that a
transaction doesn't create money out of thin air, without knowing what the actual
values are. However, there are a finite number of usable values and one could try every single
one of them to guess the value of your transaction. In addition, knowing v1 (from
a previous transaction for example) and the resulting `v1*H` reveals all outputs with
value v1 across the blockchain. For these reasons, we introduce a second elliptic curve
_G_ (practically _G_ is just another generator point on the same curve group as _H_) and
a private key _r_ used as a *blinding factor*.

An input or output value in a transaction can then be expressed as:

    r*G + v*H

Where:

* _r_ is a private key used as a blinding factor, _G_ is an elliptic curve and
  their product `r*G` is the public key for _r_ on _G_.
* _v_ is the value of an input or output and _H_ is another elliptic curve.

Neither _v_ nor _r_ can be deduced, leveraging the fundamental properties of Elliptic
Curve Cryptography. `r*G + v*H` is called a _Pedersen Commitment_.

As a an example, let's assume we want to build a transaction with two inputs and one
output. We have (ignoring fees):

* vi1 and vi2 as input values.
* vo3 as output value.

Such that:

    vi1 + vi2 = vo3

Generating a private key as a blinding factor for each input value and replacing each value
with their respective Pedersen Commitments in the previous equation, we obtain:

    (ri1*G + vi1*H) + (ri2*G + vi2*H) = (ro3*G + vo3*H)

Which as a consequence requires that:

    ri1 + ri2 = ro3

This is the first pillar of MimbleWimble: the arithmetic required to validate a
transaction can be done without knowing any of the values.

As a final note, this idea is actually derived from Greg Maxwell's
[Confidential Transactions](https://www.elementsproject.org/elements/confidential-transactions/),
which is itself derived from an Adam Back proposal for homomorphic values applied
to Bitcoin.

### Ownership

In the previous section we introduced a private key as a blinding factor to obscure the
transaction's values. The second insight of MimbleWimble is that this private
key can be leveraged to prove ownership of the value.

Alice sends you 3 coins and to obscure that amount, you chose 113 as your
blinding factor (note that in practice, the blinding factor being a private key, it's an
extremely large number). Somewhere on the blockchain, the following output appears and
should only be spendable by you:

    X = 113*G + 3*H

_X_, the result of the addition, is visible by everyone. The value 3 is only known to you and Alice,
and 113 is only known to you.

To transfer those 3 coins again, the protocol requires 113 to be known somehow.
To demonstrate how this works, let's say you want to transfer those 3 same coins to Carol.
You need to build a simple transaction such that:

    Xi => Y

Where _Xi_ is an input that spends your _X_ output and Y is Carol's output. There is no way to build
such a transaction and balance it without knowing your private key of 113. Indeed, if Carol
is to balance this transaction, she needs to know both the value sent and your private key
so that:

    Y - Xi = (113*G + 3*H) - (113*G + 3*H) = 0*G + 0*H

By checking that everything has been zeroed out, we can again make sure that
no new money has been created.

Wait! Stop! Now you know the private key in Carol's output (which, in this case, must
be the same as yours to balance out) and so you could
steal the money back from Carol!

To solve this, we allow Carol to add another value of her choosing. She picks 28, and
what ends up on the blockchain is:

    Y - Xi = ((113+28)*G + 3*H) - (113*G + 3*H) = 28*G + 0*H

Now the transaction doesn't sum to zero anymore, we have an _excess value_ on _G_
(28), which is the result of the summation of all blinding factors. But because `28*G` is
a valid public key on the elliptic curve _G_, with private key 28,
for any x and y, only if `y = 0` is `x*G + y*H` a valid public key on _G_.

So all the protocol needs to verify is that (`Y - Xi`) is a valid public key on _G_ and that
the transaction author knows the private key (28 in our transaction with Carol). The
simplest way to do so is to require an ECDSA signature built with the excess value (28),
which then validates that:

* The author of the transaction knows the excess value (which is also the
  private key for the output)
* The sum of the transaction's outputs, minus the inputs, adds to a zero value
  (because only a valid public key, matching the private key, will check against
  the signature).

Hence, what is being signed does not even matter (it can just be an empty string "").
That signature, attached to every transaction, together with some additional data (like mining
fees), is called a _transaction kernel_.

### Some Finer Points

This section elaborates on the building of transactions by discussing how change is
introduced and the requirement for range proofs so all values are proven to be
non-negative. Neither of these are absolutely required to understand MimbleWimble and
Grin, so if you're in a hurry, fee free to jump straight to
[Putting It All Together](#transaction-conclusion).

#### Change

In the above example, you had to share your private key (the blinding factor) with
Carol. In general, even though private keys should never be reused, this isn't
generally very desirable. Practically, this isn't an issue because transactions
include a change output.

Let's say you only want to send 2 coins to Carol from the 3 you received from
Alice. You simply generate another private key (say 42) as a blinding factor to
protect your change output, and tell Carol you're sending her 2 coins and that
for her transaction to be balanced she should use 113-42 as sum of blinding
factors.

Then Carol adds her own excess value of 28 (for example) and we get as outputs:

    Your change:  42*G + 1*H
    Carol:        (113-42+28)*G + 2*H

The final sum that all validators end up doing looks like:

    (42*G + 1*H) + (99*G + 2*H) - (113*G + 3*H) = 28*G + 0*H

Carol generates a signature with `28*G` as public key, as described in the previous
section, to prove that the value is zero and that she was given the summation of blinding
factors for the input and change. The signature is included in the _transaction kernel_
which will be checked by all transaction validators.

#### Range Proofs

In all the above calculations, we rely on the transaction values to always be positive. The
introduction of negative amounts would be extremely problematic as one could
create new funds in every transaction.

For example, one could create a transaction with an input of 2 and outputs of 5
and -3 and still obtain a well-balanced transaction, following the definition in
the previous sections. This can't be easily detected because even if _x_ is
negative, the corresponding point `x.H` on the ECDSA curve looks like any other.

To solve this problem, MimbleWimble leverages another cryptographic concept (also
coming from Confidential Transactions) called
range proofs: a proof that a number falls within a given range, without revealing
the number. We won't elaborate on the range proof, but you just need to know
that for any `r.G + v.H` we can build a proof that will show that _v_ is greater than
zero and does not overflow.

It's also important to note that in order to create a valid range proof from the example above, both of the values 113 and 28 used in creating and signing for the excess value must be known. The reason for this, as well as a more detailed description of range proofs are further detailed in the [range proof primer](rangeproofs.md).

<a name="transaction-conclusion"></a>
### Putting It All Together

A MimbleWimble transaction includes the following:

* A set of inputs, that reference and spend a set of previous outputs.
* A set of new outputs that include:
  * A value and a blinding factor (which is just a new private key) multiplied on
  a curve and summed to be `r.G + v.H`.
  * A range proof that shows that v is non-negative.
* An explicit transaction fee, in clear.
* A signature, computed by taking the excess blinding value (the sum of all
outputs plus the fee, minus the inputs) and using it as a private key.

## Blocks and Chain State

We've explained above how MimbleWimble transactions can provide
strong anonymity guarantees while maintaining the properties required for a valid
blockchain, i.e., a transaction does not create money and proof of ownership
is established through private keys.

The MimbleWimble block format builds on this by introducing one additional
concept: _cut-through_. With this addition, a MimbleWimble chain gains:

* Extremely good scalability, as the great majority of transaction data can be
  eliminated over time, without compromising security.
* Further anonymity by mixing and removing transaction data.
* And the ability for new nodes to sync up with the rest of the network very
efficiently.

### Cut-through

Blocks let miners assemble multiple transactions into a single set that's added
to the chain. In the following block representations, containing 3 transactions,
we only show inputs and
outputs of transactions. Inputs reference outputs they spend. An output included
in a previous block is marked with a lower-case x.

    I1(x1) --- O1
            |- O2

    I2(x2) --- O3
    I3(O2) -|

    I4(O3) --- O4
            |- O5

We notice the two following properties:

* Within this block, some outputs are directly spent by included inputs (I3
spends O2 and I4 spends O3).
* The structure of each transaction does not actually matter. As all transactions
individually sum to zero, the sum of all transaction inputs and outputs must be zero.

Similarly to a transaction, all that needs to be checked in a block is that ownership
has been proven (which comes from _transaction kernels_) and that the whole block did
not add any money supply (other than what's allowed by the coinbase).
Therefore, matching inputs and outputs can be eliminated, as their contribution to the overall
sum cancels out. Which leads to the following, much more compact block:

    I1(x1) | O1
    I2(x2) | O4
           | O5

Note that all transaction structure has been eliminated and the order of inputs and
outputs does not matter anymore. However, the sum of all outputs in this block,
minus the inputs, is still guaranteed to be zero.

A block is simply built from:

* A block header.
* The list of inputs remaining after cut-through.
* The list of outputs remaining after cut-through.
* The transaction kernels containing, for each transaction:
  * The public key `r*G` obtained from the summation of all the commitments.
  * The signatures generated using the excess value.
  * The mining fee.

When structured this way, a MimbleWimble block offers extremely good privacy
guarantees:

* More transactions may have been done but do not appear.
* All outputs look the same: just very large numbers that are impossible to
differentiate from one another. If one wanted to exclude some outputs, they'd have
to exclude all.
* All transaction structure has been removed, making it impossible to tell which output
was matched with each input.

And yet, it all still validates!

### Cut-through All The Way

Going back to the previous example block, outputs x1 and x2, spent by I1 and
I2, must have appeared previously in the blockchain. So after the addition of
this block, those outputs as well as I1 and I2 can also be removed from the
overall chain, as they do not contribute to the overall sum.

Generalizing, we conclude that the chain state (excluding headers) at any point
in time can be summarized by just these pieces of information:

1. The total amount of coins created by mining in the chain.
2. The complete set of unspent outputs.
3. The transactions kernels for each transaction.

The first piece of information can be deduced just using the block
height (its distance from the genesis block). And both the unspent outputs and the
transaction kernels are extremely compact. This has 2 important consequences:

* The state a given node in a MimbleWimble blockchain needs to maintain is very
small (on the order of a few gigabytes for a bitcoin-sized blockchain, and
potentially optimizable to a few hundreds of megabytes).
* When a new node joins a network building up a MimbleWimble chain, the amount of
information that needs to be transferred is also very small.

In addition, the complete set of unspent outputs cannot be tampered with, even
only by adding or removing an output. Doing so would cause the summation of all
blinding factors in the transaction kernels to differ from the summation of blinding
factors in the outputs.

## Conclusion

In this document we covered the basic principles that underlie a MimbleWimble
blockchain. By using the addition properties of Elliptic Curve Cryptography, we're
able to build transactions that are completely opaque but can still be properly
validated. And by generalizing those properties to blocks, we can eliminate a large
amount of blockchain data, allowing for great scaling and fast sync of new peers.
