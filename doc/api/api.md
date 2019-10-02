# Grin API Documentation

This document contains the documentation for the Node Grin REST API.

## Node API

This endpoint is used to query a node about various information on the blockchain, networks and peers. By default, this REST API will listen on `localhost:3413`. This API is started as the same time as the Grin node.
This endpoint requires, by default, [Basic Authentication](https://en.wikipedia.org/wiki/Basic_access_authentication). The username is `grin` and the password can be found in the `.api_secret` file.
To learn about what specific calls can be made read the [node API doc](node_api.md).

## Ports above 10000?

All ports should be below 10000 when running with default settings on mainnet. If your grin owner_api is using the 13420 port but is on mainnet, then you're using an outdated version of grin.
