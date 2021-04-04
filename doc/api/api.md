# Grin API Documentation

Used to query a node about various information on the blockchain, networks and peers. By default, the API will listen on `localhost:3413`. The API is started as the same time as the Grin node.
This endpoint requires, by default, [Basic Authentication](https://en.wikipedia.org/wiki/Basic_access_authentication). The username is `grin`.

## Node API

This API version uses jsonrpc for its requests. It is split up in a foreign API and an owner API. The documentation for these endpoints is automatically generated:
- [Owner API](https://docs.rs/grin_api/latest/grin_api/trait.OwnerRpc.html)
- [Foreign API](https://docs.rs/grin_api/latest/grin_api/trait.ForeignRpc.html)

Basic auth passwords can be found in `.node_owner_api_secret`/`.node_foreign_api_secret` files respectively. Note that as well it may be `.api_secret`/`.foreign_api_secret` if you are using an old version of grin
