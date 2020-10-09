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

For all security related issues, Grin has 2 main points of contact:

* Daniel Lehnberg, daniel.lehnberg at protonmail.com [PGP key](https://github.com/mimblewimble/grin-security/blob/master/keys/lehnberg.asc)
* John Woeltz, joltz at protonmail.com [PGP key](https://github.com/mimblewimble/grin-security/blob/master/keys/j01tz.asc)

Send all communications PGP encrypted to all parties.

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
[funding](https://grin.mw/fund) can help provide the necessary
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
release, as long as we are sure that they are not vulnerable to these.

## More Information

Additional security-related information about the Grin project including previous
audits, CVEs, canaries, signatures and PGP public keys can be found in the
[grin-security](https://github.com/mimblewimble/grin-security) repository.
