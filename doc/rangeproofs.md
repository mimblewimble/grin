# Range Proofs - A Primer

This document is intended to give the reader a high level conceptual overview of what range proofs are and how they're constructed. It's assumed that the reader has read and is familiar with more basic MimbleWimble concepts, in particular Pedersen commitments and MimbleWimble outputs as outlined in the [Introduction to MimbleWimble](intro.md). 

While understanding range proofs is not 100% necessary in order to understand MimbleWimble, a few points about them should be noted, even if just from a 'black box' perspective:

* Range Proofs are used to prove, with zero-knowledge (i.e. without revealing the amount or the blinding factor), that the value committed to in a given commitment falls within a certain range. The inclusion of a range proof in a MimbleWimble commitment demonstrates that the amount committed to is positive, and therefore doesn't create any new money.

* The blinding factor used to create an output on the block chain must be known in order to create a valid range proof. The existence of a valid range proof therefore also proves that the signer has knowledge of the blinding factor and therefore all of the private keys used in the creation of a new output. 

The following outlines a simple example, which builds up the concept of a range proof and demonstrates why the blinding factor in an output must be known beforehand in order to create one.

Note that this document is derived from Greg Maxwell's [Confidential Transactions Paper](https://www.elementsproject.org/elements/confidential-transactions/investigation.html), in which the concept of a range proof originated. This document simply further illustrates the concepts found in that paper. [Further reading](#further-reading) is provided at the end of the document.

## Signing an Output

Say I have this output:

```
C = (113G + 20H)
```

And I want to prove that the amount I'm commiting to (20) is positive, and in within the range for my coin. 

Firstly, rember a commitment actually appears in the UTXO set as a large, mostly unintelligible number, for example:

```
C = 2342384723497239482384234.... 
```

This number is made up of two public keys, one that only exists as a point on curve G, and one that only exists on curve H. (More specifically, the key 113G is only usable for signature operations when using generator G as a parameter to the ECDSA algorithm, and the key 20H can only create a signature when using the generator H as a parameter). In reality, the private key 113 is a very large 256 bit integer while 20 is a relatively tiny value that represents a usable currency amount, so the large number that appears on the blockchain is made up of something that looks more like:

```
C = (32849234923..74932897423987G + 20H)
```

Our commitments are of the form (bG + vH), where b is a blinding factor and v is the value. It's important to note that bG is a valid public key on G, and vH is a valid public key on H, but the large number created by adding these two values together is more or less meaningless in the context of either G or H. The sum of these two values, for any possible values of b or v, will not create a public key that's valid using either generator G or H.

## Commitments to Zero

However, it is indeed possible for the total commitment value to represent a valid public key on G or H: if either b is 0 or v is 0. For example:

```
C = (113G + 0H) = 113G -> Is a point on G
```

Since we're using vH to represent our currency values, it follows that if the output C can be used as a public key to verify a signature against the blinding factor generator G, the value v cannot be anything other than zero. Therefore if I can provide a signature for an output that is a point on G and can be verified using the commitment as a public key, I've demonstrated that I knew the blinding factor (which is the private key that created the signature), and that the amount committed to must be zero.

## Commitments to Any Number

Of course, that's not very useful in a blockchain context, as amounts will never be zero. Let's continue and consider the case where the value is 1, i.e:

```
C = (113G + 1H) -> Not a point on G or H
```

Since C is not a point on G or H, on its own this output can't be signed for. However, I can easily prove it's a commitment to 1 without revealing the blinding factor by simply subtracting 1*H from it, and observing that the result is a valid key on G. 

Consider:

```
C = 113G + 1H

C' = 113G + 1H - 1H = 113G
```

As long as a signature is provided that can be verified using C' as the public key, C' is a point on G and therefore, without the verifier necessarily knowing the value 113G beforehand, it's been demonstrated that that C is a commitment to 1.

This works for any pair of numbers, not just for 1. For any value I can pick for v, I can construct a similar proof that demonstrates C is a commitment to that value without revealing the blinding factor.

## Ring Signatures

Of course, the problem here is that I'm also revealing the amount. What we need now is a way to prove that the value v in the vH value of a commitment lies within a certain range (that is, greater than zero but less than the maximum currency value), without being able to determine either the blinding factor or the amount.

To demonstrate how we do this, let's assume that the only valid values for v that we could have in our currency are zero and one. Therefore, to show that the value we've committed to is positive it's sufficient to demonstrate that either v is zero, or that v is one. And we need to do this without actually revealing which value it is.

In order to do this, we need another type of signature called a [ring signature](https://en.wikipedia.org/wiki/Ring_signature). For our purposes, a ring signature is a signature scheme where there are multiple public keys, and the signature proves that the signer knew the corresponding private key for at least one (but not necessarily more) of the public keys. 

So, using a ring signature, it's possible to create what's called an OR proof that demonstrates C is either a commitment to zero OR a commitment to one as follows:

As before, take an output commitment C and compute C':

```
C' = C - 1H
```

Following from the example:

```
C' = 113G + 1H - 1H
```

If a ring signature is provided over {C, C'}, it proves that the signer knows the private key for either C or C', but not both. If C was a commitment to zero, the signer was able to use C to create the ring signature. If C was a commitment to one, the signer was able to use C' to create the ring signature. If C is neither a commitment to one or zero, the signer would not have been able to create the ring signature at all. However, no matter how the ring signature was created, the private key that was used to create it was not revealed. If a ring signature exists at all, C must have been either a commitment to zero, OR a commitment to one. 

As before, this works for all values of v I can pick... I can choose v=256 and construct a similar ring signature that proves v was zero OR that v was 256. In this case, the verifier just needs to subtract 256H from C and verify the ring signature used to produce the resulting C'.

## Confidential Proofs

Now that we have this construct, it's possible to use another less direct method of proving that the value v lies in a certain range using the established additive properties of commitments. To demonstrate, consider the original commitment in our example:

```
C = 113G + 20H
```

Now let's break up this commitment into parts while being careful to break the blinding factor into something that sums to the original 113 value:

```
C1 + C2 + C3 + C4 + C5 = C

or

(24G + 1H) + (25G + 2H) + (26G + 4H) + (27G + 8H) + (11G + 16H) = C
```

Obviously, this summation is not necessarily true because 1+2+4+8+16 is not equal to 20. But note that the values for v have been deliberately chosen to be powers of two. Therefore, I can use them to create the binary representation of any number up to 2<sup>5</sup> so long as I assume that some of the values actually chosen for v won't be what's shown above, but 0 instead. And a method of proving whether a commitment is a commitment to 0 OR a particular value has just been demonstrated above.

So all I need to do is provide a ring signature over each commitment value C1..C5, which demonstrates that:

```
C1 is 1 OR 0   C2 is 2 OR 0   C3 is 4 OR 0   C4 is 8 OR 0   C5 is 16 OR 0
```

Therefore, so long as my committed value can be represented in less than 2<sup>5</sup> bits, I've proven that its value must lie somewhere between 0 and 2<sup>5</sup> without revealing anything further about its value. 

## Conclusion

This is the essence of a Range Proof, used to demonstrate that commitments are positive values and that the signer has knowledge of all of the private keys used in the commitment. Note that I absolutely needed to know the private key used as a blinding factor in the output, as I needed to choose blinding values for C1 though C5 that add up to the original private key. However, I can provide the values C1..C5 to a verifier, and a verifier can use them to verify the range proof and add them up to ensure the values in the range proof equal the original commitment. However, the verifier can't break down C1..C5 into their constituent bG+vH components, and thus has no way of determining what the original private key 113 was. 

## Further Details

Note that for efficiency reasons, the range proofs used in Grin actually build up numbers in base 4 instead of base 2. However, binary is easier for the sake of the example, as binary is more familiar to most people than base 4 arithmetic.

## FAQ

Q: If I have an output `C=bG+vH` on the blockchain, and there is only a finite number of usable amounts for vH, why can't I reveal the amount by just subtracting each possible vH value from C until I get a value that can be used to create a signature on H? 

A: Pedersen Commitments are information-theoretically private. For any value of v I choose in `bG+vH`, I can choose a value of b that will make the entire commitment sum to C. Even given infinite computing power, it remains impossible to determine what the intended value of v in a given commitment is without knowing the blinding factor.

# Further Reading

[Confidential Transactions](https://www.elementsproject.org/elements/confidential-transactions/investigation.html) - The original paper with further technical details on internal  representation

[Confidential Assets](https://blockstream.com/bitcoin17-final41.pdf) - A more formalised version of the above with further improvements
