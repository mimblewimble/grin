# Grin's Security Process

Grin has a [code of conduct](CODE_OF_CONDUCT.md) and the handling of vulnerability disclosure is no exception. We are committed to conduct our security process in a professional and civil manner. Public shaming, under-reporting or misrepresentation of vulnerabilities will not be tolerated.

## Responsible Disclosure Standard

Grin follows a
[community standard for responsible disclosure](https://github.com/RD-Crypto-Spec/Responsible-Disclosure/tree/82e08d2736ea9dbe43484a3317e4bce214163bd0#the-standard)
in cryptocurrency and related software. This document is a public commitment to
following the standard.

This standard provides detailed information for:
- [Initial Contact](https://github.com/RD-Crypto-Spec/Responsible-Disclosure/tree/82e08d2736ea9dbe43484a3317e4bce214163bd0#initial-contact):
how the initial contact process works
- [Giving Details](https://github.com/RD-Crypto-Spec/Responsible-Disclosure/tree/82e08d2736ea9dbe43484a3317e4bce214163bd0#giving-details):
what details to include with your disclosure after receiving a response to your
initial contact
- [Setting Dates](https://github.com/RD-Crypto-Spec/Responsible-Disclosure/tree/82e08d2736ea9dbe43484a3317e4bce214163bd0#setting-dates):
details for when to release updates and publicize details of the issue

Any expected deviations and necessary clarifications around the standard are
explained in the following sections.

## Receiving Disclosures

Grin is committed to working with researchers who submit security vulnerability
notifications to us to resolve those issues on an appropriate timeline and perform
a coordinated release, giving credit to the reporter if they would like.

Please submit issues to all of the following main points of contact for
security related issues according to the
[initial contact](https://github.com/RD-Crypto-Spec/Responsible-Disclosure/tree/82e08d2736ea9dbe43484a3317e4bce214163bd0#initial-contact)
and [details](https://github.com/RD-Crypto-Spec/Responsible-Disclosure/tree/82e08d2736ea9dbe43484a3317e4bce214163bd0#giving-details)
guidelines. More information is available about the
[expected timelines for the full disclosure cycle](https://github.com/RD-Crypto-Spec/Responsible-Disclosure/tree/82e08d2736ea9dbe43484a3317e4bce214163bd0#standard-disclosure-timelines).

For all security related issues, Grin has 4 main points of contact:

* Daniel Lehnberg, daniel.lehnberg at protonmail.com
* hashmap, hashmap.dev at protonmail.com
* John Woeltz, joltz at protonmail.com

Send all communications PGP encrypted to all parties.
[PGP public keys](#public-keys) can be found at the end of this document.

## Sending Disclosures

In the case where we become aware of security issues affecting other projects
that has never affected Grin, our intention is to inform those projects of
security issues on a best effort basis.

In the case where we fix a security issue in Grin that also affects the
following neighboring projects, our intention is to engage in responsible
disclosures with them as described in the adopted
[standard](https://github.com/RD-Crypto-Spec/Responsible-Disclosure/tree/82e08d2736ea9dbe43484a3317e4bce214163bd0#a-standard-for-responsible-disclosure-in-cryptocurrency-and-related-software),
subject to the deviations described in the
[deviations section](#deviations-from-the-standard) of this document.

## Bilateral Responsible Disclosure Agreements

_Grin does not currently have any established bilateral disclosure agreements._

## Recognition and Bug Bounties

Grin's responsible disclosure standard includes some general language about
[Bounty Payments](https://github.com/RD-Crypto-Spec/Responsible-Disclosure/tree/82e08d2736ea9dbe43484a3317e4bce214163bd0#bounty-payments)
and [Acknowledgements](https://github.com/RD-Crypto-Spec/Responsible-Disclosure/tree/82e08d2736ea9dbe43484a3317e4bce214163bd0#acknowledgements).

Grin is a **traditional open source project with limited to no direct funding**.
As such, we have little means with which to compensate security researchers for
their contributions. We recognize this is a shame and intend to do our best to
still make these worth while by:

* Advertising the vulnerability, the researchers, or their team on a public
page linked from our website, with a links of their choosing.
* Acting as reference whenever this is needed.
* Setting up retroactive bounties whenever possible.

There is not currently a formal bug bounty program for Grin as it would require
a high level of resources and engagement to operate in good faith. More
[funding](https://grin-tech.org/funding) can help provide the necessary
resources to run one in the future for the Grin community.

## Deviations from the Standard

Grin is a technology that provides strong privacy with zero-knowledge
commitments and rangeproofs. Due to the nature of the cryptography used, if a
counterfeiting bug results it could be exploited without a way to identify
which data was corrupted. This renders rollbacks or other fork-based attempted
fixes ineffective.

The standard describes reporters of vulnerabilities including full details of
an issue, in order to reproduce it. This is necessary for instance in the case
of an external researcher both demonstrating and proving that there really is a
security issue, and that security issue really has the impact that they say it
has - allowing the development team to accurately prioritize and resolve the issue.

In the case of a counterfeiting or privacy-breaking bug, however, we might decide
not to include those details with our reports to partners ahead of coordinated
release, so long as we are sure that they are vulnerable.

## Canary

```
---===[ Grin Canary #001 ]===---


Statements
-----------

The Grin security contacts who have digitally signed this canary state the following:

1. The date of issue of this canary is September 17, 2019.

2. The latest Grin release is v2.0.0 `8f3be49`

3. No warrants have ever been served to us with regard to the Grin
Project (e.g. to hand out the private signing keys or to introduce
backdoors).

4. We plan to publish the next of these canary statements in the first
two weeks of January 2020. Special note should be taken if no new canary
is published by that time or if the list of statements changes without
plausible explanation.

Special announcements
----------------------

None.

Disclaimers and notes
----------------------

This canary scheme is not infallible. Although signing the declaration
makes it very difficult for a third party to produce arbitrary
declarations, it does not prevent them from using force or other
means, like blackmail or compromising the signers' laptops, to coerce
us to produce false declarations.

The block hashes quoted below (Proof of freshness) serve to demonstrate
that this canary could not have been created prior to the date stated.
It shows that a series of canaries was not created in advance.

This declaration is merely a best effort and is provided without any
guarantee or warranty. It is not legally binding in any way to
anybody. None of the signers should be ever held legally responsible
for any of the statements made here.

Proof of freshness
-------------------

$ date -R -u && grin client status | grep 'Last block' | cut -c 18- && curl -s 'https://blockstream.info/api/blocks/tip/hash'; echo && curl -s 'https://api.blockcypher.com/v1/ltc/main' | grep '"hash' | cut -c 12-75 && curl -s 'https://api.blockcypher.com/v1/eth/main' | grep '"hash' | cut -c 12-75
Tue, 17 Sep 2019 16:19:21 +0000
0000036044b32b7805403f651ccddf634e48bf8601d8af95409871e0848f317c
0000000000000000000fdf50819c8c280908cba1b570e614e9c970464609ec00
2418c5b533acf4212270d5898da283256df78c8fdfb0e927a7e2854e078c18f4
df30073244358cff78d2161f8d4465ae5210209019d0bb3ca14ce672ad8bcf5c
```
Don't just trust the contents of this canary blindly! Verify the digital
signatures and proof of freshness! The signatures below should be valid
for the above canary message. The proof of freshness should contain valid
block hashes on the date the canary was issued for the GRIN, BTC, LTC and ETH
blockchains respectively.

### Daniel Lehnberg

### hashmap

### John Woeltz
```
-----BEGIN PGP SIGNATURE-----

iQIzBAABCgAdFiEEpwlDvRCYWLUDTiOsmWn1cMLvYW8FAl2BCFEACgkQmWn1cMLv
YW/FLBAAkTy8mcKrb878SqAps7vBSDY/s/zsgFmeXvIAPHAVdpXnT8BvxN2MR8w4
Z96o9rlyqw91QoB1dgO7M+ujy+heZSOCVsjhhADCos92BfeVI8qmJ9isu7uEm7Yp
074AjvYdddtw60CX12fjmaM5vxZ/2aHUjqEooLtvlaOlap0DKW8n5Mba4X5fQLYi
z2SX+WD7v+WCfBgalBbsrzwZrkhn24BK7Xp9VA8HzkUxOnY4KK8sPVwgJjkzXFq6
gV7dOsgvrOTxvRJrF+7vQ0RoaAf0eRlo2lYOoGvrS5FZdLOWJmR3J83BFtkmaNkC
JCXa3QCrJhnnGKcLVEXVvFGg8821S2JfJd/NUnazRftPcmb7h1RE1pNjh2LTuonj
fPrFnvc1bLwKFnrnBj/eBD/YL6YWCy7jFR5Vh2c46hq+swkl8GzKA7oSoSrNNcql
RbKuiMnmg9XJp3MqHINgEmJc1nkAC2LBP5sBu2NVD8BDUTl728OFuh2mELbLJJ0z
0X4GA5YKSsWbhsHF6jftLf6QxrsHa/9aM3KnI3NwhhxA8P2xW4VODwwJaCDoXADT
VW1TYnMt6TT//a+lou+brj7+Hqq2lIUmb7uveqI4YLewPdnKgeQUNJCVYcpfbi/u
VyRZ2aJxaCvyo8BUaLRBEmErg5zeVgawhFXG30NfMLbGeCsztYY=
=NXLF
-----END PGP SIGNATURE-----
```

## Public Keys

### Daniel Lehnberg
```
-----BEGIN PGP PUBLIC KEY BLOCK-----

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
Yt4GsNSSB0khmbq31wIGbll/ZGsSH60h
=pLZJ
-----END PGP PUBLIC KEY BLOCK-----

```
### hashmap
```
-----BEGIN PGP PUBLIC KEY BLOCK-----

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
cN4UNoeD4hpgWl16VHn1wtOl5AEGkg==
=/+Vo
-----END PGP PUBLIC KEY BLOCK-----
```
### John Woeltz
```
-----BEGIN PGP PUBLIC KEY BLOCK-----

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
OXCaSbMWSCzC0g==
=BniA
-----END PGP PUBLIC KEY BLOCK-----
```
