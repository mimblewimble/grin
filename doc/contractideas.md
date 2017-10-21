## Atomic Swaps

"On another chain, Igno sends coins to me that I can only redeem by revealing a hash preimage (which he knows, I don't). On the MW chain we do this exchange so that Igno can take my coins by revealing the preimage. When he takes his coins, he reveals it, enabling me to take my coins.

Note that this requires both chains to support hash preimages: all Bitcoin script derivatives, Ethereum, and now Mimblewimble support this." - Andrew Poelstra
- https://lists.launchpad.net/mimblewimble/msg00022.html

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

"So the setting is that Igno holds some coins on the MW altchain, by knowing the blinding factor rI0 in an output rI0*G+a*H of a coins, while Andrew holds some coins on an MW' sidechain, by knowing the blinding factor rA0' in an output rA0'*G+a'*H of a' coins, and they'll like to swap these. Note that we require MW and MW' to use the same curve and generators G,H. We adopt a notation for quantities that start with a lower case letter for its role, followed by an uppercase letter for who
picked or computed it, a serial number, and an optional ' to distinguish the two chains.

For simplicity we ignore change outputs and fees. As Igno has pointed out, fees must also be listed and signed in the kernels, to prevent relays and miners from hijacking fees (a relay could take half the fee for itself by adding an output and kernel, while a miner could avoid the coinbase locktime on fees).

The plan is to
1) prepare transfers from original outputs into 2-of-2 outputs for holding
2) prepare locktimed refunds from the 2-of-2 outputs in case the swap fails
3) prepare the swapping transactions from the 2-of-2 outputs
4) verifiably link Igno's signatures for the transactions in 3)
5) let Igno obtain his swapping coins on MW'
6) let Andrew obtain his swapping coins on MW

Let's see how each step works in detail.

1) prepare transfers from original outputs into 2-of-2 outputs for holding

                           input       blinding/output   kernel       nonce/challenge         signature 
        MW Igno          rI0*G+a*H          rI1         rI1-rI0            kI1           sI1=kI1+e1*(rI1-rI0)
        MW Andrew          rA1               rA1          kA1                              sA1=kA1+e1*rA1
        MW tx1         (rI1+rA1)*G+a*H   rI1+rA1-rI0                 e1=H(kI1*G+kA1*G)        s1=sI1+sA1

       MW'Igno             rI1'              rI1'         kI1'                            sI1'=kI1'+e1'*rI1'
       MW'Andrew       rA0'*G+a'*H rA1'   rA1'-rA0'       kA1'                            sA1'=kA1'+e1'*(rA1'-rA0')
       MW'tx1'       (rI1'+rA1')*G+a*H   rI1'+rA1'-rA0'             e1'=H(kI1'*G+kA1'*G)     s1'=sI1'+sA1'

We assume that all commits to blinding factors and nonces are shared between them.
Both parties must also construct range proofs for the 2-of-2 outputs,
details of which we ignore.

The constituent signatures sI1,sA1,sI1', and sA1' are not yet shared,
since locking up funds is only safe if refunds are assured for a failing swap.

2) prepare locktimed refunds from the 2-of-2 outputs in case the swap fails

                       input           output       kernel       nonce/challenge        signature
       MW Igno       rI2*G+a*H        rI2-rI1       kI2                            sI2=kI2+e2*(rI2-rI1)
       MW Andrew       -rA1                         kA2                               sA2=kA2+e2*-rA1
       MW tx2       (rI1+rA1)*G+a*H  rI2-rI1-rA1              e2=H(L||kI2*G+kA2*G)     s2=sI2+sA2

The constituent signatures sI2 and sA2 are shared and verified by both parties.
Transaction tx2' on MW' is prepared similarly, but with a somewhat earlier locktime L'.

Now that it's safe to share the constituent signatures sI1,sA1,sI1', and sA1',
they are summed into signatures s1 for tx1 and s1' for tx1'.

This step could be slightly simplified by picking rI2==rI1, and
omitting kI2 and sI2, taking s2=sA2. (Andrew remarked on this "it's
really easy to create footguns in MW reusing keys. Though I think in
this case it's actually safe, you're just directly reversing the first
transaction.")

When both tx1 and tx1' are confirmed, we can proceed with step 3).
If any remaining steps fail to complete for any reason,
then either party can issue their refund transaction.

3) prepare the swapping transactions from the 2-of-2 outputs

                    input                 output       kernel       nonce/challenge      signature
       MW Igno     -rI1                                 kI3                             sI3=kI3+e3*-rI1
       MW Andrew   rA3*G+a*H              rA3-rA1       kA3                             sA3=kA3+e3*(rA3-rA1)
       MW tx3      (rI1+rA1)*G+a*H       -rI1+rA3-rA1               e3=H(kI3*G+kA3*G)   s3=sI3+sA3

                     input                 output          kernel     nonce/challenge       signature
       MW'Igno   rI3'*G+a'*H              rI3'-rI1'         kI3'                             sI3'=kI3'+e3'*(rI3'-rI1')
       MW'Andrew     -rA1'                                  kA3'                             sA3'=kA3'+e3'*-rA1'
       MW'tx3'   (rI1'+rA1')*G+a'*H      rI3'-rI1'-rA1'                e3'=H(kI3'*G+kA3'*G)  s3'=sI3'+sA3'

At this point the atomic swap is reduced to the exchange of signature
s3' for s3. We need the revelation of s3' by Igno to reveal s3 to Andrew, which is
achieved by having Andres know the difference between sI3 and sI3'.

4) verifiably link Igno's signatures for the transactions in 3)

Igno reveals sconv = sI3-sI3' = kI3+e3*-rI1 - (kI3'+e3'*(rI3'-rI1')) and Andrew verifies that sconv*G = kI3*G-e3*rI1*G - kI3'*G+e3'*(rI3'-rI1')

5) let Igno obtain his swapping coins on MW'

Appearance of tx3' on MW' reveals s3'

6) let Andrew obtain his swapping coins on MW

Andrew computes s3 = sI3+sA3 = sconv+sI3'+sA3=sconv+s3'-sA3'+sA3 and issues tx3" - John Tromp
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
