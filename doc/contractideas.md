## Atomic Swaps

"On another chain, Igno sends coins to me that I can only redeem by revealing a hash preimage (which he knows, I don't). On the MW chain we do this exchange so that Igno can take my coins by revealing the preimage. When he takes his coins, he reveals it, enabling me to take my coins.

Note that this requires both chains to support hash preimages: all Bitcoin script derivatives, Ethereum, and now Mimblewimble support this." - Andrew Poelstra
- https://lists.launchpad.net/mimblewimble/msg00022.html
========================================================

"At the Stanford BPASE Conference [https://cyber.stanford.edu/blockchainconf] I gave a talk where I briefly mentioned that it was possible to do atomic swaps with no preimages at all."

"Ok, so algebraically how do atomic swaps work in a scriptless way? Suppose I'm trying to send Igno 1 MW1 coin on one chain in exchange for MW2 coin on another. Then:

1. As before we send our coins to 2-of-2 outputs on each chain. Each of us refuses to move our coin until the other has given us a locktimed "refund" transaction; we set our locktimes so I can retrieve my coin before he can retrieve his. 

(So far this is the same as the classic Bitcoin atomic swap by Tier Nolan [3]; the difference in locktimes is because during part of the protocol Igno can take his coins but I can't yet take mine, so I want to be sure he can't do this and simultaneously back out. This way ff he takes the coins, I can take mine, but if he backs out then I've long since backed out, and these are his only possibilities.)

  2. Igno and I construct transactions that move the locked coins to their final destinations. We agree on the kernels and signature nonces, and in particular on signature challenges e and e'.

3. Igno sends me a "conversion" keys sconv which satisfies

         sconv * G = R - R' + eP - e'P'

4. I sign the MW1 transaction giving Igno his coin and send him the signature.

5. Now Igno signs the MW1 transaction, giving himself his coin. To do this he adds his signature

         s = k + xe

where `k` is his secret nonce and `x` his secret key are values I don't know but which have been forced on him (their public counterparts are committed in the hash `e`).

6. I then compute s' = s + sconv, which is Igno's half of the MW2 transaction, and am able to take my coins.


Observe that I can verify sconv is legitimate in step (3), and that this verification equation is sufficient to force my computed s' to verify iff s does. Observe further that once the two signatures are public, anybody can compute "sconv" as s' - s, which gives us two properties:

 1. It assures that sharing sconv does not harm the security of anyone's keys, since it's publicly computable anyway by anybody who has access to the final signatures.

 2. This scheme is deniable, since it depends on Igno giving me sconv before I knew s', which neither of us can prove. In other words either of us could fabricate the above transcript for any pair of signatures.


My thinking is that this atomic linking of multiple transactions is a fairly general primitive that can be used to link lightning channels etc, and that we might not need hash preimages for this after all." - Andrew Poelstra

- https://lists.launchpad.net/mimblewimble/msg00036.html
- https://lists.launchpad.net/mimblewimble/msg00047.html

## Secure Transaction Exchange

## LNs

"I talked with Thaddeus Dryja just now and showed him down to do locktime and hash preimages, and he said this should be sufficient to create HTLC's (hash-timelocked lightning channels), so I guess this gives us full lightning
support in principle." - Andrew Poelstra
- https://lists.launchpad.net/mimblewimble/msg00022.html

- https://lists.launchpad.net/mimblewimble/msg00029.html
- https://lists.launchpad.net/mimblewimble/msg00086.html
- https://lists.launchpad.net/mimblewimble/msg00090.html

## Lock-time

"Suppose that I want to send a Bitcoin to Igno conditioned on him revealing a hash preimage. He sends me the hash e. We do the following.

1. I send the coins to a multisignature output controlled by the 2-of-2 of both of us, though I don't complete my half of signing the resulting excess value.

2. Igno produces a transaction that sends the coins back to me, locktimed to some time in the future; we complete this transaction. (Well, Igno does his part and I can do mine later.)

2a. I broadcast the first transaction, so there are coins on the chain that can be spent only with both our our consents.

3. I produce a transaction which sends these coins to Igno. With the excess I sign the hash e, leave the locktime blank, and do my part to sign.

At this point Igno can either (a) complete the transaction, doing his part of the signature and revealing the preimage to the network, including me; or (b) do nothing, in which case I'll take the coin back after the lock time." - Andrew Polestra 
- https://lists.launchpad.net/mimblewimble/msg00022.html

- https://lists.launchpad.net/mimblewimble/msg00025.html
- https://lists.launchpad.net/mimblewimble/msg00034.html
- https://lists.launchpad.net/mimblewimble/msg00048.html
- https://lists.launchpad.net/mimblewimble/msg00050.html
- https://lists.launchpad.net/mimblewimble/msg00102.html

## ZKCP (Zero-Knowledge Contigent Payments)

"Recall ZKCP, as written up here
https://bitcoincore.org/en/2016/02/26/zero-knowledge-contingent-payments-announcement/

In this case, Igno produces a zero-knowledge proof that the hash preimage will decrypt the solution to some problem I care about. He gives me the encrypted solution and the hash and the proof, then we do the above exchange to trade the preimage for money."
- https://lists.launchpad.net/mimblewimble/msg00022.html

- https://lists.launchpad.net/mimblewimble/msg00037.html

## Scripting etc.
- https://lists.launchpad.net/mimblewimble/msg00025.html
- https://lists.launchpad.net/mimblewimble/msg00029.html