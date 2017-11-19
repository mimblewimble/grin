# Grin - Basic Wallet

## Wallet Files

A Grin wallet maintains its state in the following files -

```
wallet.seed  # *** passphrase protected seed file (keep this private) ***
wallet.dat   # wallet outputs (both spent and unspent)
wallet.lock  # lock file, prevents multiple processes writing to wallet.dat
```

By default Grin will look for these in the current working directory.

## Basic Wallet Commands

`grin wallet --help` will display usage info about the following. 

### grin wallet init

Before using a wallet a new seed file `wallet.seed` needs to be generated via `grin wallet init` -

```
grin wallet init
Generating wallet seed file at: ./wallet.seed
```

### grin wallet info

Some (very) basic information about current wallet outputs can be displayed with `grin wallet info` -

```
grin wallet -p "password" info
Using wallet seed file at: ./wallet.seed
Outputs -
key_id, height, lock_height, status, spendable?, coinbase?, value
----------------------------------
96805837571719c692b6, 21, 24, Spent, false, true, 50000000000
...
```

### grin wallet listen

(tbd)

### grin wallet send

(tbd)

### grin wallet receive

(tbd)

### grin wallet burn

[tbd]
