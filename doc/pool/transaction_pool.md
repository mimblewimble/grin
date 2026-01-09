## Transaction Pool

Grin's transaction pool is designed to hold all transactions that are not yet included in a block.

The transaction pool is split into a stempool and a txpool. The stempool contains "stem" transactions, which are less actively propagated to the rest of the network, as well as txs received via Dandelion "stem" phase. The txpool contains transactions that may be directly propagated to the network, as well as txs received via Dandelion "fluff" phase.

### Reconciliation

The `Pool::reconcile` function validates transactions in the stempool or txpool against a given block header and removes invalid or duplicated transactions (present in txpool). The optimized implementation filters entries in-place, reducing validations from O(n² + n*m) to O(n + m), where n is the number of transactions in the pool being reconciled and m is the number of transactions in txpool.

Reconciliation logs include:
- Number of entries before/after reconciliation
- Count of invalid or duplicated transactions removed

Example:
```
INFO: Starting transaction pool reconciliation with 200 entries
WARN: Skipping duplicate transaction: <hash>
WARN: Invalid transaction <hash>: Validation failed
INFO: Reconciliation complete: retained 180 entries, removed 10 invalid, 10 duplicates
```
