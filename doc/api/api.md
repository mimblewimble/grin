# Grin API Documentation

This document contains the documentation for the 3 Grin REST APIs. These endpoints can be grouped in two categories.

## Node API

This endpoint is used to query a node about various information on the blockchain, networks and peers. By default, this REST API will listen on `localhost:3413`. This API is started as the same time as the Grin node.
This endpoint requires, by default, [Basic Authentication](https://en.wikipedia.org/wiki/Basic_access_authentication). The username is `grin` and the password can be found in the `.api_secret` file.
To learn about what specific calls can be made read the [node API doc](node_api.md).

## Wallet APIs

### Foreign Wallet API

The foreign API is an endpoint mainly designed to receiving grins through a network. This REST API can be started with the `grin wallet listen` command and by default will listen on `localhost:3415`.
To learn about what specific calls can be made read the [wallet foreign API doc](wallet_foreign_api.md).

### Wallet Owner API

The wallet owner API is an endpoint to manage the user wallet: broadcast transaction, sign transaction, see the current balance... This REST API can be started with the `grin wallet owner_api` command and will listen on `localhost:3420`.

__This endpoint must **never** be exposed to the outside world.__

This endpoint requires, by default, Basic Authentication. The username is `grin` and the password can be found in the `.api_secret` file.
To learn about what specific calls can be made read the [wallet owner API doc](wallet_owner_api.md).

## Ports above 10000?

All ports should be below 10000 when running with default settings on mainnet. If your grin owner_api is using the 13420 port but is on mainnet, then you're using an outdated version of grin.
