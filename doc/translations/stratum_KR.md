# Grin Stratum RPC 프로토콜

이 문서는 Grin에 구현되어 있는 현재 Stratum RPC protocol 을 설명한 것입니다.

## 목차

1. [Messages](#메세지_들)
    1. [getjobtemplate](#getjobtemplate)
    1. [job](#job)
    1. [keepalive](#keepalive)
    1. [login](#login)
    1. [status](#status)
    1. [submit](#submit)
1. [에러 메시지들](#error-messages)
1. [채굴자의 행동양식](#miner-behavior)
1. [참고 구현체](#reference-implementation)

## 메세지 들

이 섹션에서는 각 메시지와 그 응답에 대해서 상술합니다.
어느때든, 채굴자가 로그인을 제외한 다음 중 한 요청을 하고 login 이 요구된다면 채굴자는 다음과 같은 에러 메시지를 받게 됩니다.

| Field         | Content                                 |
| :------------ | :-------------------------------------- |
| id            | 요청 ID                                 |
| jsonrpc       | "2.0"                                   |
| method        | 채굴자가 보낸 method                        |
| error         | {"code":-32500,"message":"login first"} |

예시:

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
만약에 요청이 다음중 하나가 아니라면, Stratum 서버가 아래와 같은 에러 메시지를 보내게 됩니다.

| Field         | Content                                      |
| :------------ | :------------------------------------------- |
| id            | 요청 ID                                       |
| jsonrpc       | "2.0"                                        |
| method        | 채굴자가 보낸 method                            |
| error         | {"code":-32601,"message":"Method not found"} |

예시:

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

채굴자에 의해 시작되는 메시지입니다.
채굴자는 이 메시지로 작업을 요청 할 수 있습니다.

#### Request

| Field         | Content                        |
| :------------ | :----------------------------- |
| id            | 요청 ID                         |
| jsonrpc       | "2.0"                          |
| method        | "getjobtemplate"               |
| params        | null                           |

예시 :

``` JSON
{  
   "id":"2",
   "jsonrpc":"2.0",
   "method":"getjobtemplate",
   "params":null
}
```

#### Response

Response 는 두가지 타입이 될 수 있습니다.

##### OK response

예시

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

만약 노드가 동기화 중이라면, 다음과 같은 메시지를 보낼것입니다.

| Field         | Content                                                   |
| :------------ | :-------------------------------------------------------- |
| id            | 요청 ID                                                    |
| jsonrpc       | "2.0"                                                     |
| method        | "getjobtemplate"                                          |
| error         | {"code":-32701,"message":"Node is syncing - Please wait"} |

예시:

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

Stratum 서버로 인해 시작되는 메세지입니다.
Stratum 서버는 연결된 채굴자에게 작업을 자동적으로 보냅니다.
채굴자는 job_id=0 이면 현재의 작업을 중단해야 합니다. 그리고 현재의 작업을 현재 graph 가 완료되면 이 작업으로 대체해야 합니다.

#### Request

| Field         | Content                                                                   |
| :------------ | :------------------------------------------------------------------------- |
| id            | 요청 ID                                                                    |
| jsonrpc       | "2.0"                                                                     |
| method        | "job"                                                                     |
| params        | Int `difficulty`, `height`, `job_id` and string `pre_pow` |

예시:

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

이 메세지에는 Response 가 필요하지 않습니다.

### `keepalive`

연결을 계속 하기 위해서 채굴자에 의해 초기화 되는 메시지입니다.

#### Request

| Field         | Content                |
| :------------ | :--------------------- |
| id            | 요청 ID                 |
| jsonrpc       | "2.0"                  |
| method        | "keepalive"            |
| params        | null                   |

예시:

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
| id            | 요청 ID                         |
| jsonrpc       | "2.0"                          |
| method        | "keepalive"                    |
| result        | "ok"                           |
| error         | null                           |

예시:

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
채굴자에 의해 시작되는 메시지입니다.
채굴자는 보통 채굴 프로그램으로 고정적으로 정해지는 login, password, agent 로 Grin Stratum 서버에 로그인 할 수 있습니다.

#### Request

| Field         | Content                        |
| :------------ | :----------------------------- |
| id            | 요청 ID                         |
| jsonrpc       | "2.0"                          |
| method        | "login"                        |
| params        | Strings: login, pass and agent |

예시:

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

Response 는 두가지 타입이 될 수 있습니다.

##### OK response

| Field         | Content                        |
| :------------ | :----------------------------- |
| id            | 요청 ID                         |
| jsonrpc       | "2.0"                          |
| method        | "login"                        |
| result        | "ok"                           |
| error         | null                           |

예사:

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

아직 구현되지 않았습니다. login이 필요할때, -32500 "Login firtst" 라는 에러를 리턴합니다.

### `status`

채굴자에 의해 시작되는 메시지입니다.
이 메시지는 채굴자에게 현재의 워커와 네트워크의 상태를 얻을 수 있게 합니다.

#### Request

| Field         | Content                |
| :------------ | :--------------------- |
| id            | 요청 ID                 |
| jsonrpc       | "2.0"                  |
| method        | "status"               |
| params        | null                   |

예시:

``` JSON
{  
   "id":"2",
   "jsonrpc":"2.0",
   "method":"status",
   "params":null
}
```

#### Response

Response 는 아래와 같습니다.

| Field         | Content                                                                                                  |
| :------------ | :------------------------------------------------------------------------------------------------------- |
| id            | 요청 ID                                                                                                   |
| jsonrpc       | "2.0"                                                                                                    |
| method        | "status"                                                                                                 |
| result        | String `id`. Integers `height`, `difficulty`, `accepted`, `rejected` and `stale`                         |
| error         | null                                                                                                     |

예시:

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

채굴자에 의해 시작되는 메시지입니다.
마이너가 쉐어를 찾았을때, 노드에게 보내집니다.

#### Request

채굴자는 Stratum 서버에 작업 솔루션을 보냅니다.

| Field         | Content                                                                     |
| :------------ | :-------------------------------------------------------------------------- |
| id            | 요청 ID                                                                      |
| jsonrpc       | "2.0"                                                                       |
| method        | "submit"                                                                    |
| params        | Int `edge_bits`,`nonce`, `height`, `job_id` and array of integers `pow`     |

예시:

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

Response 는 세가지 타입이 될 수 있습니다.

##### OK response

이 타입은 Stratum 에 받아들여지지만 네트워크 타켓 난이도에서는 유효한 cuck(at)oo 솔루션이 아닙니다.

| Field         | Content                        |
| :------------ | :----------------------------- |
| id            | 요청 ID                         |
| jsonrpc       | "2.0"                          |
| method        | "submit"                       |
| result        | "ok"                           |
| error         | null                           |

예시:

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

이 타입은 Stratum 에 받아들여지고 네트워크 타켓 난이도에서는 유효한 cuck(at)oo 솔루션 입니다.

| Field         | Content                        |
| :------------ | :----------------------------- |
| id            | 요청 ID                         |
| jsonrpc       | "2.0"                          |
| method        | "submit"                       |
| result        | "block - " + hash of the block |
| error         | null                           |

예시:

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

에러 response는 stale과 rejected 라는 두가지 타입이 될 수 있습니다.

##### Stale share error response

이 타입은 유효한 솔루션이나 지난 작업이 현재의 것이 아닙니다.

| Field         | Content                                                   |
| :------------ | :-------------------------------------------------------- |
| id            | 요청 ID                                                    |
| jsonrpc       | "2.0"                                                     |
| method        | "submit"                                                  |
| error         | {"code":-32503,"message":"Solution submitted too late"}   |

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

솔루션이 유효하지 않거나 너무 낮은 난이도일 수 있는 두 가지 가능성이 있습니다.

###### Failed to validate solution error

재출된 솔루션이 유효하지 않을 수 았습니다.

| Field         | Content                                                   |
| :------------ | :-------------------------------------------------------- |
| id            | 요청 ID                                                    |
| jsonrpc       | "2.0"                                                     |
| method        | "submit"                                                  |
| error         | {"code":-32502,"message":"Failed to validate solution"}   |

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

제출된 솔루션의 난이도가 너무 낮습니다.

| Field         | Content                                                          |
| :------------ | :--------------------------------------------------------------- |
| id            | 요청 ID                                                           |
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

Grin Stratum protocole 구현체는 다음과 같은 에러 메시지를 포함하고 있습니다.

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

채굴자들은 반드시 다음과 같은 규칙들을 존중해야 할 것입니다.

- 마이너들은 작업을 시작하기 전에 작업 nounce를 랜덤화 시켜야 합니다.
- 채굴자들은 반드시 서버가 샤로운 작업을 보낼때끼지 같은 작업을 채굴해야 하지만 언제든 새로운 작업을 요청 할 수 있습니다.
- 채굴자들은 서버로 부터 온 작업 요청을 rpc response로 보내면 안됩니다.
- 채굴자들은 RPC "id"를 정할 수 있고 같은 id로 response를 받기를 요구 할 수 있습니다.
- 마이너들은 keepalive 메시지를 보낼수 있습니다.
- 채굴자들은 로그인 request를 보낼 수 있습니다.(어떤 채굴자가 쉐어를 찾았는지 확인하기 위해서 / Log안에서 솔루션을 확인하기 위해 ) 로그인 request는 3가지 파라미터를 가지고 있어야만 합니다.
- Miners MUST return the supplied job_id with submit messages.
- 채굴자들은 주어진 job_id를 제출하는 메시지에 리턴해야 합니다.

## Reference Implementation

현재 구현체는 [mimblewimble/grin-miner](https://github.com/mimblewimble/grin-miner/blob/master/src/bin/client.rs) 에서 참고하세요.
