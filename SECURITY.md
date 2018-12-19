# Grin's Security Process

Grin has a [code of conduct](CODE_OF_CONDUCT.md) and the handling of vulnerability disclosure is no exception. We are committed to conduct our security process in a professional and civil manner. Public shaming, under-reporting or misrepresentation of vulnerabilities will not be tolerated.

## Responsible Disclosure

For all security related issues, Grin has two main points of contact:

* Daniel Lehnberg, daniel.lehnberg at protonmail.com
* Ignotus Peverell, igno.peverell at protonmail.com

Send all communications to both parties and expect a reply within 48h. Public keys can be found at the end of this document.

## Vulnerability Handling

Upon reception of a vulnerability disclosure, the Grin team will:

* Reply within a 48h window.
* Within a week, a [CVVS v3](https://nvd.nist.gov/vuln-metrics/cvss/v3-calculator) severity score should be attributed.
* Keep communicating regularly about the state of a fix, especially for High or Critical severity vulnerabilities.
* Once a fix has been identified, agree on a timeline for release and public disclosure.

Releasing a fix should include the following steps:

* Creation of a CVE number for all Medium and above severity vulnerabilities.
* Notify all package maintainers or distributors.
* Inclusion of a vulnerability explanation, the CVE and the security researcher or team who found the vulnerability in release notes and project vulnerability list (link TBD).
* Publicize the vulnerability commensurately with severity and encourage fast upgrades (possibly with additional documentation to explain who is affected, the risks and what to do about it).

_Note: Before Grin mainnet is released, we will be taking some liberty in applying the above steps, notably in issuing a CVE and upgrades._

## Recognition and Bug Bounties

As of this writing, Grin is a **traditional open source project** with limited to no direct funding. As such, we have little means with which to compensate  security researchers for their contributions. We recognize this is a shame and intend to do our best to still make these worth while by:

* Advertising the vulnerability, the researchers, or their team on a public page linked from our website, with a links of their choosing.
* Acting as reference whenever this is needed.
* Setting up retroactive bounties whenever possible.

It is our hope that after mainnet release, participants in the ecosystem will be willing to more widely donate to benefit the further development of Grin. When this is the case we will:

* Setup a bounty program.
* Decide on the amounts rewarded based on available funds and CVVS score.

## Code Reviews and Audits

While we intend to undergo more formal audits before release, continued code reviews and audits are required for security. As such, we encourage interested security researchers to:

* Review our code, even if no contributions are planned.
* Publish their findings whichever way they choose, even if no particular bug or vulnerability was found. We can all learn from new sets of eyes and benefit from increased scrutiny.
* Audit the project publicly. While we may disagree with some small points of design or trade-offs, we will always do so respectfully.

## Chain Splits

The Grin Team runs a chain split monitoring tool at (TBD). It is encouraged to monitor it regularly and setup alerts. In case of an accidental chain split:

* Exchanges and merchants should either cease operation or extend considerably confirmation delays.
* Miners and mining pools should immediately consult with Grin's development team on regular channels (Grin's Gitter mainly) to diagnose the split and determine a course of events.
* In the likely event of an emergency software patch, all actors should upgrade as soon as possible.

## Useful References

* [Reducing the Risks of Catastrophic Cryptocurrency Bugs](https://medium.com/mit-media-lab-digital-currency-initiative/reducing-the-risk-of-catastrophic-cryptocurrency-bugs-dcdd493c7569)
* [Security Process for Open Source Projects](https://alexgaynor.net/2013/oct/19/security-process-open-source-projects/)
* [Choose-Your-Own-Security-Disclosure-Adventure](http://hackingdistributed.com/2018/05/30/choose-your-own-security-disclosure-adventure/)
* [CVE HOWTO](https://github.com/RedHatProductSecurity/CVE-HOWTO)
* [National Vulnerability Database](https://nvd.nist.gov/)

## Public Keys

### Daniel Lehnberg
  ````
-----BEGIN PGP PUBLIC KEY BLOCK-----

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
Yt4GsNSSB0khmbq31wIGbll/ZGsSH60h
=pLZJ
-----END PGP PUBLIC KEY BLOCK-----

````
### Ignotus Peverell
```
-----BEGIN PGP PUBLIC KEY BLOCK-----

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
VaXAUOUL1MeJOiXI96Q=
=xdp/
-----END PGP PUBLIC KEY BLOCK-----
```
