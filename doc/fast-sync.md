# Fast Sync

*Read this in other languages: [Español](translations/fast-sync_ES.md), [Korean](translations/fast-sync_KR.md), [简体中文](translations/fast-sync_ZH-CN.md).*

In Grin, we call "sync" the process of synchronizing a new node or a node that
hasn't been keeping up with the chain for a while, and bringing it up to the
latest known most-worked block. Initial Block Download (or IBD) is often used
by other blockchains, but this is problematic for Grin as it typically does not
download full blocks.

In short, a fast-sync in Grin does the following:

1. Download all block headers, by chunks, on the most worked chain, as
   advertized by other nodes.
1. Find a header sufficiently back from the chain head. This is called the node
   horizon as it's the furthest a node can reorganize its chain on a new fork if
   it were to occur without triggering another new full sync.
1. Download the full state as it was at the horizon, including the unspent
   output, range proof and kernel data, as well as all corresponding MMRs. This is
   just one large zip file.
1. Validate the full state.
1. Download full blocks since the horizon to get to the chain head.

In the rest of this section, we will elaborate on each of those steps.
