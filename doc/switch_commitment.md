# Introduction to Switch Commitments

*Read this in other languages: [简体中文](translations/switch_commitment_ZH-CN.md).*

## General introduction

In cryptography a _Commitment_ (or _commitment scheme_) refers to a concept which can be imagined
like a box with a lock. You can put something into the box (for example a piece of a paper with a
secret number written on it), lock it and give it to another person (or the public).

The other person doesn't know yet what's the secret number in the box, but if you decide to publish
your secret number later in time and want to prove that this really is the secret which you came
up with in the first place (and not a different one) you can prove this simply by giving the
key of the box to the other person.

They can unlock the box, compare the secret within the box with the secret you just published
and can be sure that you didn't change your secret since you locked it. You "**committed**"
to the secret number beforehand, meaning you cannot change it between the time of
commitment and the time of revealing.


## Examples

### Hash Commitment

A simple commitment scheme can be realized with a cryptographic hash function. For example: Alice and Bob
want to play _"Guess my number"_ and Alice comes up with with her really secret number `29` which
Bob has to guess in the game, then before the game starts, Alice calculates:

    hash( 29 + r )

and publishes the result to Bob. The `r` is a randomly chosen  _Blinding Factor_ which is
needed because otherwise Bob could just try hashing all the possible numbers for the game and
compare the hashes.

When the game is finished, Alice simply needs to publish her secret number `29` and the
blinding factor `r` and Bob can calculate the hash himself and easily verify that Alice
did not change the secret number during the game.


### Pedersen Commitment

Other, more advanced commitment schemes can have additional properties. For example Mimblewimble
and Confidential Transactions (CT) make heavy use of
_[Pedersen Commitments](https://link.springer.com/content/pdf/10.1007/3-540-46766-1_9.pdf)_,
which are _homomorphic_ commitments. Homomorphic in this context means that (speaking in the
"box" metaphor from above) you can take two of these locked boxes (_box1_ and _box2_) and
somehow "_add_" them together, so that you
get a single box as result (which still is locked), and if you open this single box later
(like in the examples before) the secret it contains, is the sum of the secrets
from _box1_ and _box2_.

While this "box" metaphor no longer seems to be reasonable in the real-world this
is perfectly possible using the properties of operations on elliptic curves.

Look into [Introduction to Mimblewimble](intro.md) for further details on Pedersen Commitments
and how they are used in Grin.


## Properties of commitment schemes:

In general for any commitment scheme we can identify two important properties
which can be weaker or stronger, depending on the type of commitment scheme:

- **Hidingness (or Confidentiality):** How good is the commitment scheme protecting the secret
  commitment. Or speaking in terms of our example from above: what would an attacker need to
  open the box (and learn the secret number) without having the key to unlock it?

- **Bindingness:** Is it possible at all (or how hard would it be) for an attacker to somehow
  find a different secret, which would produce the same commitment, so that the attacker could
  later open the commitment to a different secret, thus breaking the _binding_ of the
  commitment.

### Security of these properties:

For these two properties different security levels can be identified.

The two most important combinations of these are

- **perfectly binding** and **computationally hiding** commitment schemes and
- **computationally binding** and **perfectly hiding** commitment schemes

"_Computationally_" binding or hiding means that the property (bindingness/hidingness)
is secured by the fact that the underlying mathematical problem is too hard to be solved
with existing computing power in reasonable time (i.e. not breakable today as computational
resources are bound in the real world).

"_Perfectly_" binding or hiding means that even with infinite computing power
it would be impossible to break the property (bindingness/hidingness).



### Mutual exclusivity:

It is important to realize that it's **impossible** that any commitment scheme can be
_perfectly binding_ **and** _perfectly hiding_ at the same time. This can be easily shown
with a thought experiment: Imagine an attacker having infinite computing power, he could
simply generate a commitment for all possible values (and blinding factors) until finding a
pair that outputs the same commitment. If we further assume the commitment scheme is
_perfectly binding_ (meaning there cannot be two different values leading to the same
commitment) this uniquely would identify the value within the commitment, thus
breaking the hidingness.

The same is true the other way around. If a commitment scheme is _perfectly hiding_
there must exist several input values resulting in the same commitment (otherwise an
attacker with infinite computing power could just try all possible values as
described above). This concludes that the commitment scheme cannot be _perfectly
binding_.

#### Always a compromise

The key take-away point is this: it's **always a compromise**, you can never have both
properties (_hidingness_ and _bindingness_) with _perfect_ security. If one is _perfectly_
secure then the other can be at most _computationally_ secure
(and the other way around).


### Considerations for cryptocurrencies

Which roles do these properties play in the design of cryptocurrencies?

**Hidingness**:
In privacy oriented cryptocurrencies like Grin, commitment schemes are used to secure
the contents of transactions. The sender commits to an amount of coins he sends, but for
the general public the concrete amount should remain private (protected by the _hidingness_ property of the commitment scheme).

**Bindingness**:
At the same time no transaction creator should ever be able to change his commitment
to a different transaction amount later in time. If this would be possible, an attacker
could spend more coins than previously committed to in an UTXO (unspent transaction
output) and therefore inflate coins out of thin air. Even worse, as the amounts are
hidden, this could go undetected.

So there is a valid interest in having both of these properties always secured and
never be violated.

Even with the intent being that both of these properties will hold for the lifetime
of a cryptocurrency, still a choice has to be made about which commitment scheme to use.


#### A hard choice?

Which one of these two properties needs to be _perfectly_ safe
and for which one it would be sufficient to be _computationally_ safe?
Or in other words: in case of a disaster, if the commitment scheme unexpectedly
gets broken, which one of the two properties should be valued higher?
Economical soundness (no hidden inflation possible) or ensured privacy (privacy will
be preserved)?

This seems like a hard to choice to make.


If we look closer into this we realize that the commitment scheme only needs to be
_perfectly_ binding at the point in time when the scheme actually gets broken. Until
then it will be safe even if it's only _computationally_ binding.

At the same time a privacy-oriented cryptocurrency needs to ensure the _hidingness_
property **forever**. Unlike the _binding_ property, which only is important at the
time when a transaction is created and will not affect past transactions, the _hidingness_
property must be ensured at all times. Otherwise, in the unfortunate case should the
commitment scheme be broken, an attacker could go back in the chain and unblind
past transactions, thus break the privacy property retroactively.


## Properties of Pedersen Commitments

Pedersen Commitments are **computationally binding** and **perfectly hiding** as for a given
commitment to the value `v`: `v*H + r*G` there may exist a pair of different values `r1`
and `v1` such that the sum will be the same. Even if you have infinite computing power
and could try all possible values, you would not be able to tell which one is the original one
(thus _perfectly hiding_).


## Introducing Switch Commitments

So what can be done if the bindingness of the Pedersen Commitment unexpectedly gets broken?

In general a cryptocurrency confronted with a broken commitment scheme could choose to
change the scheme in use, but the problem with this approach would be that it requires to
create new transaction outputs using the new scheme to make funds secure again. This would
require every coin holder to move his coins into new transaction outputs.
If coins are not moved into new outputs, they will not profit from the
security of the new commitment scheme. Also, this has to happen **before** the scheme gets
actually broken in the wild, otherwise the existing UTXOs no longer can be assumed
to contain correct values.

In this situation [_Switch Commitments_](https://eprint.iacr.org/2017/237.pdf) offer a neat
solution. These type of commitments allow changing the properties of the commitments just
by changing the revealing / validating procedure without changing the way commitments
are created. (You "_switch_" to a new validation scheme which is backwards
compatible with commitments created long before the actual "_switch_").


### How does this work in detail

First let's introduce a new commitment scheme: The **ElGamal commitment** scheme is a commitment
scheme similiar to Pedersen Commitments and it's _perfectly binding_ (but only _computationally
hiding_ as we can never have both).
It looks very similar to a Pedersen Commitment, with the addition of a new
element, calculated by multiplying the blinding factor `r` with another generator point `J`:

    v*H + r*G ,  r*J

So if we store the additional field `r*J` and ignore it for now, we can treat it like
Pedersen Commitments, until we decide to also validate the full ElGamal
commitment at some time in future. This is exactly what was implemented in an
[earlier version of Grin](https://github.com/mimblewimble/grin/blob/5a47a1710112153fb38e4406251c9874c366f1c0/core/src/core/transaction.rs#L812),
before mainnet was launched. In detail: the hashed value of `r*J`
(_switch\_commit\_hash_) was added to the transaction output, but this came with
the burden of increasing the size of each output by 32 bytes.

Fortunately, later on the Mimblewimble mailinglist Tim Ruffing came up with a really
[beautiful idea](https://lists.launchpad.net/mimblewimble/msg00479.html)
(initially suggested by Pieter Wuille), which offers the same advantages but doesn't
need this extra storage of an additional element per transaction output:

The idea is the following:

A normal Pedersen commitment looks like this:

    v*H + r*G

(`v` is value of the input/output, `r` is a truly random blinding factor, and `H` and `G` are
two generator points on the elliptic curve).

If we adapt this by having `r` not being random itself, but using another random number `r'`
and create the Pedersen Commitment:

    v*H + r*G

such that:

    r = r' + hash( v*H + r'*G  ,  r'*J )

(using the additional third generation point `J` on the curve) then `r` still is perfectly
valid as a blinding factor, as it's still randomly distributed, but now we see
that the part within the brackets of the hash function (`v*H + r'*G  ,  r'*J`) is an
**ElGamal commitment**.

This neat idea lead to the removal of the switch commitment hash from the outputs in this
(and following) [pull requests](https://github.com/mimblewimble/grin/issues/998) as now it
could be easily included into the Pedersen Commitments.


This is how it is currently implemented in Grin. Pedersen commitments are
used for the Confidential Transaction but instead of choosing the blinding factor `r`
only by random, it is calculated by adding the hash of an ElGamal commitment to a random `r'`
(see here in [main_impl.h#L267](https://github.com/mimblewimble/secp256k1-zkp/blob/73617d0fcc4f51896cce4f9a1a6977a6958297f8/src/modules/commitment/main_impl.h#L267)).


In general switch commitments were first described in the paper
["Switch Commitments: A Safety Switch for Confidential Transactions"](https://eprint.iacr.org/2017/237.pdf)).
The **"switch"** in the name comes from the fact that you can virtually flip a "switch" in
the future and simply by changing the validation procedure you can change the strength of
the bindingness and hidingness property of your commitments and this even works in a
backwards compatible way with commitments created today.



## Conclusion

Grin uses Pedersen Commitments - like other privacy cryptocurrencies do as well - with
the only difference that the random blinding factor `r` is created using the ElGamal
commitment scheme.

This might not seem like a big change on a first look, but it provides an
important safety measure:

Pedersen Commitments are already _perfectly hiding_ so whatever happens, privacy will
never be at risk without requiring any action from users. But in case of a disaster if the
bindingness of the commitment scheme gets broken, then switch commitments can be enabled
(via a soft fork) requiring that all new transactions prove that their commitment is not
breaking the bindingness by validating the full ElGamal commitment.

But in this case users would still have a choice:

- they can decide to continue to create new transactions, even if this might compromise
  their privacy (only on their **last** UTXOs) as the ElGamal commitment scheme is
  only computationally hiding, but at least they would still have access to their coins

- or users can decide to just leave the money alone, walk away and make no more transactions
  (but preserve their privacy, as their old transactions only validated the Pedersen commitment
  which is perfectly hiding)


There are many cases where a privacy leak is much more dangerous to one's life than
some cryptocurrency might be worth. But this is a decision that should be left up to
the individual user and switch commitments enable this type of choice.

It should be made clear that this is a safety measure meant to be enabled in case of a
disaster. If advances in computing would put the hardness of the discrete log problem
in question, a lot of other cryptographic systems, including other cryptocurrencies,
will be in urgent need of updating their primitives to a future-proof system. The switch
commitments just provide an additional layer of security if the bindingness of Pedersen
commitments ever breaks unexpectedly.
