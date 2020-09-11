# Grin Stratum RPC Protocol

*Read this in other languages: [Korean](translations/stratum_KR.md), [简体中文](translations/stratum_ZH-CN.md).*

This document describes the current Stratum RPC protocol implemented in Grin.

## Table of Contents

1. [Messages](#messages)
    1. [getjobtemplate](#getjobtemplate)
    1. [job](#job)
    1. [keepalive](#keepalive)
    1. [login](#login)
    1. [status](#status)
    1. [submit](#submit)
1. [Error Messages](#error-messages)
1. [Miner Behavior](#miner-behavior)
1. [Reference Implementation](#reference-implementation)

## Messages

In this section, we detail each message and the potential response.

At any point, if miner the tries to do one of the following request (except login) and login is required, the miner will receive the following error message.

| Field         | Content                                 |
| :------------ | :-------------------------------------- |
| id            | ID of the request                       |
| jsonrpc       | "2.0"                                   |
| method        | method sent by the miner                |
| error         | {"code":-32500,"message":"login first"} |

Example:

```JSON
{
   "id":"10",
   "jsonrpc":"2.0",
   "method":"getjobtemplate",
   "error":{
      "code":-32500,
      "message":"login first"
   }
}
```

if the request is not one of the following, the stratum server will give this error response:

| Field         | Content                                      |
| :------------ | :------------------------------------------- |
| id            | ID of the request                            |
| jsonrpc       | "2.0"                                        |
| method        | method sent by the miner                     |
| error         | {"code":-32601,"message":"Method not found"} |

Example:

```JSON
{
   "id":"10",
   "jsonrpc":"2.0",
   "method":"getgrins",
   "error":{
      "code":-32601,
      "message":"Method not found"
   }
}
```

### `getjobtemplate`

A message initiated by the miner.
Miner can request a job with this message.

#### Request

| Field         | Content                        |
| :------------ | :----------------------------- |
| id            | ID of the request              |
| jsonrpc       | "2.0"                          |
| method        | "getjobtemplate"               |
| params        | null                           |

Example:

``` JSON
{
   "id":"2",
   "jsonrpc":"2.0",
   "method":"getjobtemplate",
   "params":null
}
```

#### Response

The response can be of two types:

##### OK response

Example:

``` JSON
{
   "id":"0",
   "jsonrpc":"2.0",
   "method":"getjobtemplate",
   "result":{
      "difficulty":1,
      "height":13726,
      "job_id":4,
      "pre_pow":"00010000000000003c4d0171369781424b39c81eb39de10cdf4a7cc27bbc6769203c7c9bc02cc6a1dfc6000000005b50f8210000000000395f123c6856055aab2369fe325c3d709b129dee5c96f2db60cdbc0dc123a80cb0b89e883ae2614f8dbd169888a95c0513b1ac7e069de82e5d479cf838281f7838b4bf75ea7c9222a1ad7406a4cab29af4e018c402f70dc8e9ef3d085169391c78741c656ec0f11f62d41b463c82737970afaa431c5cabb9b759cdfa52d761ac451276084366d1ba9efff2db9ed07eec1bcd8da352b32227f452dfa987ad249f689d9780000000000000b9e00000000000009954"
   }
}
```

##### Error response

If the node is syncing, it will send the following message:

| Field         | Content                                                   |
| :------------ | :-------------------------------------------------------- |
| id            | ID of the request                                         |
| jsonrpc       | "2.0"                                                     |
| method        | "getjobtemplate"                                          |
| error         | {"code":-32701,"message":"Node is syncing - Please wait"} |

Example:

```JSON
{
   "id":"10",
   "jsonrpc":"2.0",
   "method":"getjobtemplate",
   "error":{
      "code":-32000,
      "message":"Node is syncing - Please wait"
   }
}
```

### `job`

A message initiated by the Stratum server.
Stratum server will send job automatically to connected miners.
The miner SHOULD interrupt current job if job_id = 0, and SHOULD replace the current job with this one after the current graph is complete.

#### Request

| Field         | Content                                                                   |
| :------------ | :------------------------------------------------------------------------- |
| id            | ID of the request                                                         |
| jsonrpc       | "2.0"                                                                     |
| method        | "job"                                                                     |
| params        | Int `difficulty`, `height`, `job_id` and string `pre_pow` |

Example:

``` JSON
{
   "id":"Stratum",
   "jsonrpc":"2.0",
   "method":"job",
   "params":{
      "difficulty":1,
      "height":16375,
      "job_id":5,
      "pre_pow":"00010000000000003ff723bc8c987b0c594794a0487e52260c5343288749c7e288de95a80afa558c5fb8000000005b51f15f00000000003cadef6a45edf92d2520bf45cbd4f36b5ef283c53d8266bbe9aa1b8daaa1458ce5578fcb0978b3995dd00e3bfc5a9277190bb9407a30d66aec26ff55a2b50214b22cdc1f3894f27374f568b2fe94d857b6b3808124888dd5eff7e8de7e451ac805a4ebd6551fa7a529a1b9f35f761719ed41bfef6ab081defc45a64a374dfd8321feac083741f29207b044071d93904986fa322df610e210c543c2f95522c9bdaef5f598000000000000c184000000000000a0cf"
   }
}
```

#### Response

No response is required for this message.

### `keepalive`

A message initiated by the miner in order to keep the connection alive.

#### Request

| Field         | Content                |
| :------------ | :--------------------- |
| id            | ID of the request      |
| jsonrpc       | "2.0"                  |
| method        | "keepalive"            |
| params        | null                   |

Example:

``` JSON
{
   "id":"2",
   "jsonrpc":"2.0",
   "method":"keepalive",
   "params":null
}
```

#### Response

| Field         | Content                        |
| :------------ | :----------------------------- |
| id            | ID of the request              |
| jsonrpc       | "2.0"                          |
| method        | "keepalive"                    |
| result        | "ok"                           |
| error         | null                           |

Example:

``` JSON
{
   "id":"2",
   "jsonrpc":"2.0",
   "method":"keepalive",
   "result":"ok",
   "error":null
}
```

### `login`

***

A message initiated by the miner.
Miner can log in on a Grin Stratum server with a login, password and agent (usually statically set by the miner program).

#### Request

| Field         | Content                        |
| :------------ | :----------------------------- |
| id            | ID of the request              |
| jsonrpc       | "2.0"                          |
| method        | "login"                        |
| params        | Strings: login, pass and agent |

Example:

``` JSON

{
   "id":"0",
   "jsonrpc":"2.0",
   "method":"login",
   "params":{
      "login":"login",
      "pass":"password",
      "agent":"grin-miner"
   }
}

```
#### Response

The response can be of two types:

##### OK response

| Field         | Content                        |
| :------------ | :----------------------------- |
| id            | ID of the request              |
| jsonrpc       | "2.0"                          |
| method        | "login"                        |
| result        | "ok"                           |
| error         | null                           |

Example:

``` JSON
{
   "id":"1",
   "jsonrpc":"2.0",
   "method":"login",
   "result":"ok",
   "error":null
}
```

##### Error response

Not yet implemented. Should return error -32500 "Login first" when login is required.

### `status`

A message initiated by the miner.
This message allows a miner to get the status of its current worker and the network.

#### Request

| Field         | Content                |
| :------------ | :--------------------- |
| id            | ID of the request      |
| jsonrpc       | "2.0"                  |
| method        | "status"               |
| params        | null                   |

Example:

``` JSON
{
   "id":"2",
   "jsonrpc":"2.0",
   "method":"status",
   "params":null
}
```

#### Response

The response is the following:

| Field         | Content                                                                                                  |
| :------------ | :------------------------------------------------------------------------------------------------------- |
| id            | ID of the request                                                                                        |
| jsonrpc       | "2.0"                                                                                                    |
| method        | "status"                                                                                                 |
| result        | String `id`. Integers `height`, `difficulty`, `accepted`, `rejected` and `stale` |
| error         | null                                                                                                     |

Example:

```JSON
{
   "id":"5",
   "jsonrpc":"2.0",
   "method":"status",
   "result":{
      "id":"5",
      "height":13726,
      "difficulty":1,
      "accepted":0,
      "rejected":0,
      "stale":0
   },
   "error":null
}
```

### `submit`

A message initiated by the miner.
When a miner find a share, it will submit it to the node.

#### Request

The miner submit a solution to a job to the Stratum server.

| Field         | Content                                                                     |
| :------------ | :-------------------------------------------------------------------------- |
| id            | ID of the request                                                           |
| jsonrpc       | "2.0"                                                                       |
| method        | "submit"                                                                    |
| params        | Int `edge_bits`,`nonce`, `height`, `job_id` and array of integers `pow` |

Example:

``` JSON
{
   "id":"0",
   "jsonrpc":"2.0",
   "method":"submit",
   "params":{
      "edge_bits":29,
      "height":16419,
      "job_id":0,
      "nonce":8895699060858340771,
      "pow":[
         4210040,10141596,13269632,24291934,28079062,84254573,84493890,100560174,100657333,120128285,130518226,140371663,142109188,159800646,163323737,171019100,176840047,191220010,192245584,198941444,209276164,216952635,217795152,225662613,230166736,231315079,248639876,263910393,293995691,298361937,326412694,330363619,414572127,424798984,426489226,466671748,466924466,490048497,495035248,496623057,502828197, 532838434
         ]
   }
}
```

#### Response

The response can be of three types.

##### OK response

The share is accepted by the Stratum but is not a valid cuck(at)oo solution at the network target difficulty.

| Field         | Content                        |
| :------------ | :----------------------------- |
| id            | ID of the request              |
| jsonrpc       | "2.0"                          |
| method        | "submit"                       |
| result        | "ok"                           |
| error         | null                           |

Example:

``` JSON
{
   "id":"2",
   "jsonrpc":"2.0",
   "method":"submit",
   "result":"ok",
   "error":null
}
```

##### Blockfound response

The share is accepted by the Stratum and is a valid cuck(at)oo solution at the network target difficulty.

| Field         | Content                        |
| :------------ | :----------------------------- |
| id            | ID of the request              |
| jsonrpc       | "2.0"                          |
| method        | "submit"                       |
| result        | "block - " + hash of the block |
| error         | null                           |

Example:

``` JSON
{
   "id":"6",
   "jsonrpc":"2.0",
   "method":"submit",
   "result":"blockfound - 23025af9032de812d15228121d5e4b0e977d30ad8036ab07131104787b9dcf10",
   "error":null
}
```

##### Error response

The error response can be of two types: stale and rejected.

##### Stale share error response

The share is a valid solution to a previous job not the current one.

| Field         | Content                                                   |
| :------------ | :-------------------------------------------------------- |
| id            | ID of the request                                         |
| jsonrpc       | "2.0"                                                     |
| method        | "submit"                                          |
| error         | {"code":-32503,"message":"Solution submitted too late"} |

Example:

```JSON
{
   "id":"5",
   "jsonrpc":"2.0",
   "method":"submit",
   "error":{
      "code":-32503,
      "message":"Solution submitted too late"
   }
}
```

##### Rejected share error responses

Two possibilities: the solution cannot be validated or the solution is of too low difficulty.

###### Failed to validate solution error

The submitted solution cannot be validated.

| Field         | Content                                                   |
| :------------ | :-------------------------------------------------------- |
| id            | ID of the request                                         |
| jsonrpc       | "2.0"                                                     |
| method        | "submit"                                          |
| error         | {"code":-32502,"message":"Failed to validate solution"} |

Example:

```JSON
{
   "id":"5",
   "jsonrpc":"2.0",
   "method":"submit",
   "error":{
      "code":-32502,
      "message":"Failed to validate solution"
   }
}
```

###### Share rejected due to low difficulty error

The submitted solution is of too low difficulty.

| Field         | Content                                                          |
| :------------ | :--------------------------------------------------------------- |
| id            | ID of the request                                                |
| jsonrpc       | "2.0"                                                            |
| method        | "submit"                                                         |
| error         | {"code":-32501,"message":"Share rejected due to low difficulty"} |

Example:

```JSON
{
   "id":"5",
   "jsonrpc":"2.0",
   "method":"submit",
   "error":{
      "code":-32501,
      "message":"Share rejected due to low difficulty"
   }
}
```

## Error Messages

Grin Stratum protocol implementation contains the following error message:

| Error code  | Error Message                          |
| :---------- | :------------------------------------- |
| -32000      | Node is syncing - please wait          |
| -32500      | Login first                            |
| -32501      | Share rejected due to low difficulty   |
| -32502      | Failed to validate solution            |
| -32503      | Solution Submitted too late            |
| -32600      | Invalid Request                        |
| -32601      | Method not found                       |

## Miner behavior

Miners SHOULD, MAY or MUST respect the following rules:

- Miners SHOULD randomize the job nonce before starting
- Miners MUST continue mining the same job until the server sends a new one, though a miner MAY request a new job at any time
- Miners MUST NOT send an rpc response to a job request from the server
- Miners MAY set the RPC "id" and expect responses to have that same id
- Miners MAY send a keepalive message
- Miners MAY send a login request (to identify which miner finds shares / solutions in the logs), the login request MUST have all 3 params.
- Miners MUST return the supplied job_id with submit messages.

## Reference Implementation

The current reference implementation is available at [mimblewimble/grin-miner](https://github.com/mimblewimble/grin-miner/blob/master/src/bin/client.rs).
