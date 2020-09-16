# P2P Protocol

## P2P Messages

#### WARNING: This document is still in progress and has not yet been reviewed by any of the core Grin developers.

* All fields are serialized in Big-Endian byte order unless otherwise specified.
* Variable-length strings (VAR_STR) are encoded in UTF8, and are preceded with a uint64 indicating their lengths.

### Message Types

There are currently 19 different types of P2P Messages:

| Id | Message Type     | Description                                                                                                                              |
|----|------------------|------------------------------------------------------------------------------------------------------------------------------------------|
| 0  | Error            | Sent when an issue is found during communication with a peer. Usually followed by closing the connection.                                |
| 1  | Hand             | First part of a handshake, sender advertises its version and characteristics.                                                            |
| 2  | Shake            | Second part of a handshake, receiver of the first part replies with its own version and characteristics.                                 |
| 3  | Ping             | Sent to confirm that the connection is still valid, and used to advertise the node's total_difficulty to confirm whether sync is needed. |
| 4  | Pong             | The response to a ping message.                                                                                                          |
| 5  | GetPeerAddrs     | Used to request addresses of new peers to connect to.                                                                                    |
| 6  | PeerAddrs        | Peer addresses sent in response to a GetPeerAddrs message.                                                                               |
| 7  | GetHeaders       | Used to request block headers from a peer.                                                                                               |
| 8  | Header           | A single block header received from a peer.                                                                                              |
| 9  | Headers          | Multiple block headers received from a peer in response to a GetHeaders message.                                                         |
| 10 | GetBlock         | Used to request a block from a peer.                                                                                                     |
| 11 | Block            | A single block received from a peer.                                                                                                     |
| 12 | GetCompactBlock  | Used to request a compact block from a peer.                                                                                             |
| 13 | CompactBlock     | A single compact block received from a peer.                                                                                             |
| 14 | StemTransaction  | A stem transaction received from a peer.                                                                                                 |
| 15 | Transaction      | A transaction received from a peer.                                                                                                      |
| 16 | TxHashSetRequest | Used to request the transaction hashset from a peer.                                                                                     |
| 17 | TxHashSetArchive | The transaction hashset in response to the TxHashSetRequest message.                                                                     |
| 18 | BanReason        | Contains the reason your node was banned by a peer.                                                                                      |

### Message structure

All P2P messages follow a generic message structure as follows.

| Size | Name    | Data Type       | Description/Comments                                                              |
|------|---------|-----------------|-----------------------------------------------------------------------------------|
| 2    | Magic   | uint8[2]        | Magic number used to identify Grin packets. Always hard-coded as {0x1E, 0xC5}.    |
| 1    | Type    | MessageTypeEnum | Identifier of the packet content.                                                 |
| 8    | Length  | uint64          | The total length of the message. This will not include the header size (11 bytes).|
| ?    | Payload | uint8[]         | The actual data.                                                                  |

TODO: Provide example

### Common structures

##### SocketAddress

| Size | Name            | Data Type          | Description/Comments                                        |
|------|-----------------|--------------------|-------------------------------------------------------------|
| 1    | IPAddressFamily | uint8              | Identifies the IP address family. 0 = IPv4, 1 = IPv6.       |
| 4/16 | IPAddress       | uint8[4]/uint16[8] | The IP address. 4 octets for IPv4 or 8 hexadecets for IPv6. |
| 2    | Port            | uint16             | The TCP/IP port number.                                     |

##### CapabilitiesMask

| Value | Name              | Description                                                                |
|-------|-------------------|----------------------------------------------------------------------------|
| 0x00  | Unknown           | We don't know (yet) what the peer can do.                                  |
| 0x01  | Full History      | Full archival node, has the whole history without any pruning.             |
| 0x02  | TxHashSet History | Can provide block headers and the TxHashSet for some recent-enough height. |
| 0x04  | Peer List         | Can provide a list of healthy peers.                                       |
| 0x06  | Fast Sync Node    | Both "TxHashSet History" and "Peer List"                                   |
| 0x07  | Full Node         | "Full History", "TxHashSet History", and "Peer List"                       |

##### TransactionBody

| Size | Name    | Data Type  | Description/Comments                                                     |
|------|---------|------------|--------------------------------------------------------------------------|
| ?    | Inputs  | TxInput[]  | List of inputs spent by the transaction.                                 |
| ?    | Outputs | TxOutput[] | List of outputs the transaction produces.                                |
| ?    | Kernels | TxKernel[] | List of kernels that make up this transaction (usually a single kernel). |

##### CompactBlockBody

| Size | Name        | Data Type  | Description/Comments                                                                    |
|------|-------------|------------|-----------------------------------------------------------------------------------------|
| ?    | FullOutputs | TxOutput[] | List of full outputs - specifically the coinbase output(s).                             |
| ?    | FullKernels | TxKernel[] | List of full kernels - specifically the coinbase kernel(s).                             |
| ?    | KernelIds   | ShortId[]  | List of transaction kernels, excluding those in the full list. Each ShortId is 6 bytes. |

##### TxInput

##### TxOutput

##### TxKernel

##### ProofOfWork

| Size | Name              | Data Type  | Description/Comments                                            |
|------|-------------------|------------|-----------------------------------------------------------------|
| 8    | TotalDifficulty   | uint64     | Total accumulated difficulty since genesis block.               |
| 4    | ScalingDifficulty | uint32     | Difficulty scaling factor between the different proofs of work. |
| 8    | Nonce             | uint64     | Nonce increment used to mine the block.                         |
| 1    | EdgeBits          | uint8      | Power of 2 used for the size of the cuckoo graph.               |
| 336  | ProofNonces       | uint64[42] | The cuckoo proof nonces.                                        |

### Messages

##### Error

| Size | Name    | Data Type | Description/Comments                         |
|------|---------|-----------|----------------------------------------------|
| 4    | Code    | uint32    | Error Code. TODO: Determine possible values. |
| ?    | Message | VAR_STR   | Slightly more user-friendly message          |

##### Hand

| Size | Name            | Data Type        | Description/Comments                                                                       |
|------|-----------------|------------------|--------------------------------------------------------------------------------------------|
| 4    | Version         | uint32           | Protocol version of the sender.                                                            |
| 1    | Capabilities    | CapabilitiesMask | Bitmask representing the capabilities of the sender.                                       |
| 8    | Nonce           | uint64           | Randomly generated for each handshake to help detect connections to yourself.              |
| 8    | TotalDifficulty | uint64           | Total difficulty accumulated by the sender. Used to check whether sync may be needed.      |
| 7/19 | SenderAddress   | SocketAddress    | Network address of the sender. 7 bytes for IPv4 or 19 bytes for IPv6.                      |
| 7/19 | ReceiverAddress | SocketAddress    | Network address of the receiver. 7 bytes for IPv4 or 19 bytes for IPv6.                    |
| ?    | UserAgent       | VAR_STR          | Name and version of the software. Example: "MW/Grin 0.1.2"                                 |
| 32   | Hash            | uint8[32]        | Genesis block of the current chain. Testnet1/2/3 and mainnet all have a different genesis. |

##### Shake

| Size | Name            | Data Type        | Description/Comments                                                                       |
|------|-----------------|------------------|--------------------------------------------------------------------------------------------|
| 4    | Version         | uint32           | Protocol version of the sender.                                                            |
| 1    | Capabilities    | CapabilitiesMask | Bitmask representing the capabilities of the sender.                                       |
| 8    | Nonce           | uint64           | Randomly generated for each handshake to help detect connections to yourself.              |
| 8    | TotalDifficulty | uint64           | Total difficulty accumulated by the sender. Used to check whether sync may be needed.      |
| ?    | UserAgent       | VAR_STR          | Name and version of the software. Example: "MW/Grin 0.1.2"                                 |
| 32   | Hash            | uint8[32]        | Genesis block of the current chain. Testnet1/2/3 and mainnet all have a different genesis. |

##### Ping

| Size | Name            | Data Type | Description/Comments                                                                                |
|------|-----------------|-----------|-----------------------------------------------------------------------------------------------------|
| 8    | TotalDifficulty | uint64    | Total difficulty accumulated by the sender. Used to check whether sync may be needed.               |
| 8    | Height          | uint64    | Total block height accumulated by the sender. See: https://github.com/mimblewimble/grin/issues/1779 |

##### Pong

| Size | Name            | Data Type | Description/Comments                                                                                |
|------|-----------------|-----------|-----------------------------------------------------------------------------------------------------|
| 8    | TotalDifficulty | uint64    | Total difficulty accumulated by the sender. Used to check whether sync may be needed.               |
| 8    | Height          | uint64    | Total block height accumulated by the sender. See: https://github.com/mimblewimble/grin/issues/1779 |

##### GetPeerAddrs

| Size | Name         | Data Type        | Description/Comments                   |
|------|--------------|------------------|----------------------------------------|
| 1    | Capabilities | CapabilitiesMask | The capabilities the peer should have. |

##### PeerAddrs

| Size | Name  | Data Type       | Description/Comments                                                      |
|------|-------|-----------------|---------------------------------------------------------------------------|
| 4    | Size  | uint32_t        | The number of peer addresses received.                                    |
| ?    | Peers | SocketAddress[] | Peer addresses that match the criteria from the GetPeerAddresses request. |

##### GetHeaders

| Size | Name   | Data Type | Description/Comments                                             |
|------|--------|-----------|------------------------------------------------------------------|
| 1    | Size   | uint8_t   | The number of headers being requested.                           |
| ?    | Hashes | Hash[]    | The 32-byte Blake2b hashes of the block headers being requested. |

##### Header

| Size | Name              | Data Type      | Description/Comments                                                           |
|------|-------------------|----------------|--------------------------------------------------------------------------------|
| 2    | Version           | uint16         | The version of the block.                                                      |
| 8    | Height            | uint64         | Height of this block since the genesis block (height 0).                       |
| 8    | Timestamp         | int64          | Timestamp at which the block was built.                                        |
| 32   | Previous          | Hash           | Blake2b hash of the block previous to this in the chain.                       |
| 32   | PreviousRoot      | Hash           | Merklish root of all the commitments in the previous block's TxHashSet.        |
| 32   | OutputRoot        | Hash           | Merklish root of all the commitments in the TxHashSet.                         |
| 32   | RangeProofRoot    | Hash           | Merklish root of all range proofs in the TxHashSet.                            |
| 32   | KernelRoot        | Hash           | Merklish root of all transaction kernels in the TxHashSet.                     |
| 32   | TotalKernelOffset | BlindingFactor | Total accumulated sum of kernel offsets since genesis block.                   |
| 8    | OutputMMRSize     | uint64         | Total size of the output Merkle Mountain Range(MMR) after applying this block. |
| 8    | KernelMMRSize     | uint64         | Total size of the kernel MMR after applying this block.                        |
| 178  | ProofOfWork       | ProofOfWork    | Proof of work and related.                                                     |

##### Headers

| Size | Name    | Data Type     | Description/Comments                                                     |
|------|---------|---------------|--------------------------------------------------------------------------|
| 2    | Size    | uint16_t      | The number of headers received.                                          |
| ?    | Headers | BlockHeader[] | The headers matching the hashes provided in the GetBlockHeaders request. |

##### GetBlock

| Size | Name      | Data Type | Description/Comments                           |
|------|-----------|-----------|------------------------------------------------|
| 32   | BlockHash | Hash      | The Blake2b hash of the block being requested. |

##### Block

| Size | Name   | Data Type       | Description/Comments                                                 |
|------|--------|-----------------|----------------------------------------------------------------------|
| 405  | Header | BlockHeader     | The block header.                                                    |
| ?    | Body   | TransactionBody | The block transaction containing the inputs, outputs, and kernel(s). |

##### GetCompactBlock

| Size | Name      | Data Type | Description/Comments                                   |
|------|-----------|-----------|--------------------------------------------------------|
| 32   | BlockHash | Hash      | The Blake2b hash of the compact block being requested. |

##### CompactBlock

| Size | Name   | Data Type        | Description/Comments                                                 |
|------|--------|------------------|----------------------------------------------------------------------|
| 405  | Header | BlockHeader      | The header with metadata and commitments to the rest of the data.    |
| 8    | Nonce  | uint64           | Nonce for connection specific short_ids.                             |
| ?    | Body   | CompactBlockBody | Container for out_full, kern_full and kern_ids in the compact block. |

##### StemTransaction

| Size | Name   | Data Type       | Description/Comments                                                |
|------|--------|-----------------|---------------------------------------------------------------------|
| 32   | Offset | BlindingFactor  | The kernel "offset" k2.                                             |
| ?    | Body   | TransactionBody | The transaction body containing the inputs, outputs, and kernel(s). |

##### Transaction

| Size | Name   | Data Type       | Description/Comments                                                |
|------|--------|-----------------|---------------------------------------------------------------------|
| 32   | Offset | BlindingFactor  | The kernel "offset" k2.                                             |
| ?    | Body   | TransactionBody | The transaction body containing the inputs, outputs, and kernel(s). |

##### TxHashSetRequest

| Size | Name      | Data Type | Description/Comments                                                  |
|------|-----------|-----------|-----------------------------------------------------------------------|
| 32   | BlockHash | Hash      | Blake2b hash of the block for which the TxHashSet should be provided. |
| 8    | Height    | uint64    | Height of the corresponding block.                                    |

##### TxHashSetArchive

The response to a TxHashSetRequest. Includes a zip stream of the archive after the message body.

| Size | Name      | Data Type | Description/Comments                                            |
|------|-----------|-----------|-----------------------------------------------------------------|
| 32   | BlockHash | Hash      | Blake2b hash of the block for which the txhashset are provided. |
| 8    | Height    | uint64    | Height of the corresponding block.                              |
| 8    | Bytes     | uint64    | Size in bytes of the archive.                                   |


##### BanReason

| Size | Name      | Data Type        | Description/Comments    |
|------|-----------|------------------|-------------------------|
| 4    | BanReason | ReasonForBanEnum | The reason for the ban. |


## Protocol Versions and Capabilities

### Capabilities

Any feature that users will have the ability to disable should be implemented as a new capability (See CapabilitiesMask above). This includes things like archive mode/full history.

### Protocol Version

Any change to the p2p protocol that is not toggled by the addition of a new capability should result in an increase in protocol version (See Hand & Shake messages above). This includes any addition or removal of a field or entire message, or any backward-incompatible behavior change to an existing field or message. When interacting with peers on an older protocol version, backward compatibility must be maintained, so the newer node should follow the rules of the older protocol version.

##### Phasing out old peers

To reduce the long-term complexity of the code, we can periodically bump the "major" protocol version. Although the protocol version is just a uint32, we can consider every increase by 1000 a new "major" protocol version. We can then gracefully phase out stubborn peers that refuse to upgrade by only supporting 1 major protocol version in the past. Here's what this would look like in practice:

* Peers with a protocol version in the range [0-999] should be able to interact with any peer in the range [0-1999].
* Peers with a protocol version in the range [1000-1999] should be able to interact with any peer in the range [0-2999].
* Peers with a protocol version in the range [2000-2999] should be able to interact with any peer in the range [1000-3999].

Determining when to increase the "major" version is left up to the discretion of the developers. Care must be taken to ensure we are not increasing too quickly however, as any major bump could result in the inability to connect to certain older peers.
