[![Build Status](https://travis-ci.org/mimblewimble/grin.svg?branch=master)](https://travis-ci.org/mimblewimble/grin) [![Gitter chat](https://badges.gitter.im/grin_community/Lobby.png)](https://gitter.im/grin_community/Lobby) [![Codecov coverage status](https://codecov.io/gh/mimblewimble/grin/branch/master/graph/badge.svg)](https://codecov.io/gh/mimblewimble/grin)

# Grin

Grin is an in-progress implementation of the MimbleWimble protocol. Many characteristics are still undefined but the following constitutes a first set of choices:

  * Clean and minimal implementation, aiming to stay as such.
  * Follows the MimbleWimble protocol, which provides great anonymity and scaling characteristics.
  * Cuckoo Cycle proof of work.
  * Relatively fast block time (a minute).
  * Fixed block reward over time with a decreasing dilution.
  * Transaction fees are based on the number of Outputs created/destroyed and total transaction size.
  * Smooth curve for difficulty adjustments.

To learn more, read our [introduction to MimbleWimble and Grin](doc/intro.md).

## Status

Grin is still an infant, much is left to be done and [contributions](CONTRIBUTING.md) are welcome (see below). Check our [mailing list archives](https://lists.launchpad.net/mimblewimble/) for the latest status.

## Contributing

To get involved, read our [contributing docs](CONTRIBUTING.md).

Find us:

* Chat: [Gitter](https://gitter.im/grin_community/Lobby).
* Mailing list: join the [~MimbleWimble team](https://launchpad.net/~mimblewimble) and subscribe on Launchpad.
* Twitter for the Grin council: [@grincouncil](https://twitter.com/grincouncil)

## Getting Started

To learn more about the technology, read our [introduction](doc/intro.md).

To build and try out Grin, see the [build docs](doc/build.md).

## Philosophy

Grin likes itself small and easy on the eyes. It wants to be inclusive and welcoming for all walks of life, without judgement. Grin is terribly ambitious, but not at the detriment of others, rather to further us all. It may have strong opinions to stay in line with its objectives, which doesn't mean disrespect of others' ideas.

We believe in pull requests, data and scientific research. We do not believe in unfounded beliefs.

## Credits

Tom Elvis Jedusor for the first formulation of MimbleWimble.

Andrew Poelstra for his related work and improvements.

John Tromp for the Cuckoo Cycle proof of work.

J.K. Rowling for making it despite extraordinary adversity.

## License

Apache License v2.0.
