# Grin API Documentation

This document contains the documentation for the 3 Grin REST APIs. These endpoints can be grouped in two categories.

## Node API

This endpoint is used to query a node about various information on the blockchain, networks and peers. By default, this REST API will listen on `localhost:13413`. This API is started as the same time as the Grin node.
To learn about what specific calls can be made read the [node API doc](node_api.md).

## Wallet APIs

### Foreign Wallet API

The foreign API is an endpoint mainly designed to receiving grins through a network. This REST API can be started with the `grin wallet listen` command and by default will listen on `localhost:13415`.
To learn about what specific calls can be made read the [wallet foreign API doc](wallet_foreign_api.md).

### Wallet Owner API

The wallet owner API is an endpoint to manage the user wallet: broadcast transaction, sign transaction, see the current balance... This REST API can be started with the `grin wallet owner_api` command and will listen on `localhost:13420`. This endpoint must **never** be exposed to the outside world.
To learn about what specific calls can be made read the [wallet owner API doc](wallet_owner_api.md).
