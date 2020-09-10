# Grin Stratum RPC 协议

*阅读其它语言版本: [Korean](../stratum_KR.md)，[English](stratum.md).*

本文说明在 Grin 部署目前的 Stratum RPC 协议。

## 目录

1. [信息](#messages)
    1. [获得工作模板](#getjobtemplate)
    1. [工作](#job)
    1. [保持在线](#keepalive)
    1. [登录](#login)
    1. [状态](#status)
    1. [提交](#submit)
1. [错误信息](#error-messages)
1. [矿工行为](#miner-behavior)
1. [参考部署](#reference-implementation)

## 信息

本节我们讨论每种信息和可能的回复。

如果矿工随时最初以下请求（登录除外），且需要登录，矿工会收到以下错误信息。

| 栏位    | 内容                                     |
| :------ | :--------------------------------------- |
| id      | ID of the request（提出请求的 ID）       |
| jsonrpc | "2.0"                                    |
| method  | method sent by the miner（矿工发送方法） |
| error   | {"code":-32500,"message":"login first"}  |

范例：

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

如果不是如下请求，stratum 服务器会返回错误回复：

| Field   | Content                                      |
| :------ | :------------------------------------------- |
| id      | ID of the request                            |
| jsonrpc | "2.0"                                        |
| method  | method sent by the miner                     |
| error   | {"code":-32601,"message":"Method not found"} |

范例：

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

矿工发起的信息。
矿工可以这一信息申请工作。

#### 请求

| Field栏位 | Content           |
| :-------- | :---------------- |
| id        | ID of the request |
| jsonrpc   | "2.0"             |
| method    | "getjobtemplate"  |
| params    | null              |

范例：

``` JSON
{
   "id":"2",
   "jsonrpc":"2.0",
   "method":"getjobtemplate",
   "params":null
}
```

#### 回复

回复可分为两种类型：

##### 确认回复

范例：

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

##### 错误回复

如果节点在同步，会发送如下信息：

| Field         | Content                                                   |
| :------------ | :-------------------------------------------------------- |
| id            | ID of the request                                         |
| jsonrpc       | "2.0"                                                     |
| method        | "getjobtemplate"                                          |
| error         | {"code":-32701,"message":"Node is syncing - Please wait"} |

范例：

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

### 工作

Stratum 服务器发起新消息。Stratum 服务器会自动发送工作给连接的矿工。如果 `job_id = 0` 矿工应该切断目前的工作，并且目前的图形完成后用这一个代替现有工作。

#### 请求

| Field         | Content                                                                   |
| :------------ | :------------------------------------------------------------------------- |
| id            | ID of the request                                                         |
| jsonrpc       | "2.0"                                                                     |
| method        | "job"                                                                     |
| params        | Int `difficulty`, `height`, `job_id` and string `pre_pow` |

范例：

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

#### 回复

本消息不需要回复

### 保持在线

为保持在线，矿工发起消息。

#### 请求

| Field         | Content                |
| :------------ | :--------------------- |
| id            | ID of the request      |
| jsonrpc       | "2.0"                  |
| method        | "keepalive"            |
| params        | null                   |

范例：

``` JSON
{
   "id":"2",
   "jsonrpc":"2.0",
   "method":"keepalive",
   "params":null
}
```

#### 回复

| Field         | Content                        |
| :------------ | :----------------------------- |
| id            | ID of the request              |
| jsonrpc       | "2.0"                          |
| method        | "keepalive"                    |
| result        | "ok"                           |
| error         | null                           |

范例：

``` JSON
{
   "id":"2",
   "jsonrpc":"2.0",
   "method":"keepalive",
   "result":"ok",
   "error":null
}
```

### 登录

***

矿工发起消息。矿工用用户名、密码和代理（通常由矿工程序设置）登录 Grin Stratum 服务器。

#### 请求

| Field         | Content                        |
| :------------ | :----------------------------- |
| id            | ID of the request              |
| jsonrpc       | "2.0"                          |
| method        | "login"                        |
| params        | Strings: login, pass and agent |

范例：

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
#### 回复

回复可为两种类型：

##### 确认回复

| Field         | Content                        |
| :------------ | :----------------------------- |
| id            | ID of the request              |
| jsonrpc       | "2.0"                          |
| method        | "login"                        |
| result        | "ok"                           |
| error         | null                           |

范例：

``` JSON
{
   "id":"1",
   "jsonrpc":"2.0",
   "method":"login",
   "result":"ok",
   "error":null
}
```

##### 错误回复

未部署。应该返回`error -32500`，若需要登录，“首先登录”。

### 状态

矿工发起消息。本消息允许矿工获得目前矿工和网络状态。

#### 请求

| Field         | Content                |
| :------------ | :--------------------- |
| id            | ID of the request      |
| jsonrpc       | "2.0"                  |
| method        | "status"               |
| params        | null                   |

范例：

``` JSON
{
   "id":"2",
   "jsonrpc":"2.0",
   "method":"status",
   "params":null
}
```

#### 回复

回复如下

| Field         | Content                                                                                                  |
| :------------ | :------------------------------------------------------------------------------------------------------- |
| id            | ID of the request                                                                                        |
| jsonrpc       | "2.0"                                                                                                    |
| method        | "status"                                                                                                 |
| result        | String `id`. Integers `height`, `difficulty`, `accepted`, `rejected` and `stale` |
| error         | null                                                                                                     |

范例：

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

### 提交

矿工发起消息。矿工挖到一份，会提交给节点。

#### 请求

矿工向 Stratum 服务器提交工作解决方案。

| Field         | Content                                                                     |
| :------------ | :-------------------------------------------------------------------------- |
| id            | ID of the request                                                           |
| jsonrpc       | "2.0"                                                                       |
| method        | "submit"                                                                    |
| params        | Int `edge_bits`,`nonce`, `height`, `job_id` and array of integers `pow` |

范例：

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

#### 回复

回复可为三种类型。

##### 确认回复

Stratum 接受份额，但不是目前网络目标难度的有效 cuck(at)oo 解决方案。

| Field         | Content                        |
| :------------ | :----------------------------- |
| id            | ID of the request              |
| jsonrpc       | "2.0"                          |
| method        | "submit"                       |
| result        | "ok"                           |
| error         | null                           |

范例：

``` JSON
{
   "id":"2",
   "jsonrpc":"2.0",
   "method":"submit",
   "result":"ok",
   "error":null
}
```

##### 发现区块回复

份额被 Stratum 接受，是目前网络目标难度的有效 cuck(at)oo 解决方案。

| Field         | Content                        |
| :------------ | :----------------------------- |
| id            | ID of the request              |
| jsonrpc       | "2.0"                          |
| method        | "submit"                       |
| result        | "block - " + hash of the block |
| error         | null                           |

范例：

``` JSON
{
   "id":"6",
   "jsonrpc":"2.0",
   "method":"submit",
   "result":"blockfound - 23025af9032de812d15228121d5e4b0e977d30ad8036ab07131104787b9dcf10",
   "error":null
}
```

##### 错误回复

错误回复有两种类型：过期和被拒绝。

##### 过期份额错误回复

份额是上一工作而不是目前工作的有效解决方案。

| Field         | Content                                                   |
| :------------ | :-------------------------------------------------------- |
| id            | ID of the request                                         |
| jsonrpc       | "2.0"                                                     |
| method        | "submit"                                          |
| error         | {"code":-32503,"message":"Solution submitted too late"} |

范例：

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

##### 拒绝份额错误回复

两种可能：解决方案无效或解决方案难度太低。

###### 未能验证解决方案错误

提交的解决方案无法验证。

| Field         | Content                                                   |
| :------------ | :-------------------------------------------------------- |
| id            | ID of the request                                         |
| jsonrpc       | "2.0"                                                     |
| method        | "submit"                                          |
| error         | {"code":-32502,"message":"Failed to validate solution"} |

范例：

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

###### 因低难度错误份额拒绝

提交的解决方案难度太低。

| Field         | Content                                                          |
| :------------ | :--------------------------------------------------------------- |
| id            | ID of the request                                                |
| jsonrpc       | "2.0"                                                            |
| method        | "submit"                                                         |
| error         | {"code":-32501,"message":"Share rejected due to low difficulty"} |

范例：

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

## 错误信息

Grin Stratum 协议部署包含以下错误信息：

| Error code  | Error Message                          |
| :---------- | :------------------------------------- |
| -32000      | Node is syncing - please wait          |
| -32500      | Login first                            |
| -32501      | Share rejected due to low difficulty   |
| -32502      | Failed to validate solution            |
| -32503      | Solution Submitted too late            |
| -32600      | Invalid Request                        |
| -32601      | Method not found                       |

## 矿工行为准则

矿工须遵守以下规则：

- 矿工开始挖矿前需要随机算工作随机数
- 矿工必须一直进行相同工作，直到服务器发送新的，尽管矿工有可能随时申请工作
- 矿工不得从服务器给工作请求发送 RPC 回复
- 矿工可设置 RPC "id" 并获得回复
- 矿工可发送保持在线信息
- 矿工可发送登录请求（确认哪个矿工在历史记录发现份额 / 解决方案），登录请求必须有三个参数
- 矿工提交信息必须提供 job_id

## 部署参考

挖矿部署请参阅：[mimblewimble/grin-miner](https://github.com/mimblewimble/grin-miner/blob/master/src/bin/client.rs)
