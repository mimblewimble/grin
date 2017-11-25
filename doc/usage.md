# Using grin
Welcome! grin is so far useable for testing only. Currently on Testnet1.
Please try it out and help finding bugs.

## Participate on Testnet1

Please participate in testing grin!
Ask about errors you see, and try retelling the answers in your own words.
That brings attention to them, so we can make them more understandable for
users, and improve the [troubleshooting](FAQ.md#troubleshooting) section.

As a user, you can already try to:

- ☐ sync the chain
  - BUG: Some block(s) have transaction(s) which seem to crash `grin server`
  - BUG: chain minority forks occur often, and are not quickly abandoned
  - BUG: `grin server` can jump between chains and fail to converge on the
      chain with the most accumulated work.
- ☑ mine, using any of these PoW implementations:
  - ☑ reference Cuckoo implementation
  - ☑ cpu_mean_compat mining plugin (typical performance: 1000 graphs/s)
  - ☑ cpu_mean mining plugin (typical performance: 2000 graphs/s)
  - ☐ GPU mining not yet functional
- ☑ view your wallets "outputs" (that *sometimes* equals spendable assets)
  - BUG: spendable outputs might be hidden or forgotten.
    WORKAROUND: Backup, then edit wallet.dat manually.
    A wallet reconstruction command is in development
    [#295](https://github.com/mimblewimble/grin/issues/295)
  - BUG: when chain sync fails, coins might be incorrectly marked "Spent".
    WORKAROUND: Search-replace "Spent" with "Unconfirmed" and resync
  - BUG: when you activate mining, outputs are created that can be shown as
    having a value (50.0) and just awaiting confirmation; but if you don't get
    at least 1 confirmation within 1-5 minutes you can be sure they never will.
    WORKAROUND: wait until chain can fully sync before you start mining, and
    then babysit your mining process in case it forks off
- ☑ send simple transactions (the transaction format is not final)
  - BUG: after you create a transaction, the outputs it consumes are Locked
    until the transaction confirms.
    WORKAROUND: create transactions on the command line, use `-s smallest`
    and if your recipient fails to claim in a reasonable time, you can always
    claim it back yourself, loosing only the fee. Or do a full resync...

Legend for the above: `☑ Probably yes now, or ☐ Probably not now.`

## Known problems
When you run into problems, please see [troubleshooting](FAQ.md#troubleshooting)
and ask on [gitter chat](https://gitter.im/grin_community/Lobby).

Especially if you're not sure you've found a new bug,
[please ask on the chat]()
or on the [maillist](https://launchpad.net/~mimblewimble)

And before you file a new bug, please take a quick look through
[known bugs](https://github.com/mimblewimble/grin/issues?utf8=%E2%9C%93&q=label%3Abug+)
and other existing issues.

### Developers, developers, developers!

Please see our list of
[known requests and bugs](https://github.com/mimblewimble/grin/issues)
and of course come say hello in the chat or the maillist linked above.
