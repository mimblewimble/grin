# Wallet Owner API Documentation

## Table of Contents

1. [Wallet Owner Endpoint](#wallet-owner-endpoint)
    1. [GET Retrieve Outputs](#get-retrieve-outputs)
    1. [GET Retrieve Summary Info](#get-retrieve-summary-info)
    1. [GET Node Height](#get-node-height)
    1. [GET Retrieve Txs](#get-retrieve-txs)
    1. [GET Dump Stored Tx](#get-dump-stored-tx)
    1. [POST Issue Send Tx](#post-issue-send-tx)
    1. [POST Finalize Tx](#post-finalize-tx)
    1. [POST Cancel Tx](#post-cancel-tx)
    1. [POST Issue Burn Tx](#post-issue-burn-tx)

## Wallet Owner Endpoint

### GET Retrieve Outputs

Attempt to update and retrieve outputs.

* **URL**

  * /v1/wallet/owner/retrieve_outputs
  * /v1/wallet/owner/retrieve_outputs?refresh&show_spent&tx_id=x&tx_id=y

* **Method:**

  `GET`
  
* **URL Params**

  **Optional:**

  `refresh` to refresh from node
  `show_spent` to show spent outputs
  `tx_id=[number]` to retrieve only the specified output

* **Data Params**

  None

* **Success Response:**

  * **Code:** 200
  * **Content:** Array of

    | Field                | Type     | Description                                                    |
    |:---------------------|:---------|:---------------------------------------------------------------|
    |                      | bool     | Whether it was refreshed from node                             |
    | -                    | []object | Array                                                          |
    | - -                  | []object | Array of Output Data                                           |
    | - - -                | object   | Output Data                                                    |
    | - - - - root_key_id  | string   | Root key_id that the key for this output is derived from       |
    | - - - - key_id       | string   | Derived key for this output                                    |
    | - - - - n_child      | number   | How many derivations down from the root key                    |
    | - - - - value        | number   | Value of the output, necessary to rebuild the commitment       |
    | - - - - status       | string   | Current status of the output                                   |
    | - - - - height       | number   | Height of the output                                           |
    | - - - - lock_height  | number   | Height we are locked until                                     |
    | - - - - is_coinbase  | bool     | Is this a coinbase output? Is it subject to coinbase locktime? |
    | - - - - tx_log_entry | number   | Optional corresponding internal entry in tx entry log          |
    | - - -                | []number | Pedersen Commitment                                            |

* **Error Response:**

  * **Code:** 400

* **Sample Call:**

  ```javascript
    $.ajax({
      url: "/v1/wallet/owner/retrieve_outputs?refresh&id=3",
      dataType: "json",
      type : "GET",
      success : function(r) {
        console.log(r);
      }
    });
  ```

### GET Retrieve Summary Info

 Retrieve summary info for wallet.

* **URL**

  * /v1/wallet/owner/retrieve_summary_info
  * /v1/wallet/owner/retrieve_summary_info?refresh

* **Method:**

  `GET`
  
* **URL Params**

  **Optional:**

  `refresh` to refresh from node

* **Data Params**

  None

* **Success Response:**

  * **Code:** 200
  * **Content:** Array of

    | Field                             | Type     | Description                             |
    |:----------------------------------|:---------|:----------------------------------------|
    |                                   | bool     | Whether it was refreshed from node      |
    | -                                 | object   | Wallet information                      |
    | - -  last_confirmed_height        | number   | Height from which info was taken        |
    | - -  total                        | number   | Total amount in the wallet              |
    | - -  amount_awaiting_confirmation | number   | Amount awaiting confirmation            |
    | - -  amount_immature              | number   | Coinbases waiting for lock height       |
    | - -  amount_currently_spendable   | number   | Amount currently spendable              |
    | - -  amount_locked                | number   | Amount locked via previous transactions |

* **Error Response:**

  * **Code:** 400

* **Sample Call:**

  ```javascript
    $.ajax({
      url: "/v1/wallet/owner/retrieve_summary_info",
      dataType: "json",
      type : "GET",
      success : function(r) {
        console.log(r);
      }
    });
  ```

### GET Node Height

Retrieve current height from node.

* **URL**

  /v1/wallet/owner/node_height

* **Method:**

  `GET`
  
* **URL Params**

  None

* **Data Params**

  None

* **Success Response:**

  * **Code:** 200
  * **Content:** Array of

    | Field  | Type     | Description                                |
    |:-------|:---------|:-------------------------------------------|
    |        | number   | Node height                                |
    |        | bool     | Wether the update from node was successful |

* **Error Response:**

  * **Code:** 400

* **Sample Call:**

  ```javascript
    $.ajax({
      url: "/v1/wallet/owner/node_height",
      dataType: "json",
      type : "GET",
      success : function(r) {
        console.log(r);
      }
    });
  ```

### GET Retrieve Txs

Attempt to update outputs and retrieve transactions.
Return whether the outputs were validated against a node and an array of TxLogEntry.

* **URL**

  * /v1/wallet/owner/retrieve_txs
  * /v1/wallet/owner/retrieve_txs?refresh&id=x
  * /v1/wallet/owner/retrieve_txs?tx_id=x

* **Method:**

  `GET`
  
* **URL Params**

  **Optional:**

  * `refresh` to refresh from node
  * `id=[number]` to retrieve only the specified output by id
  * `tx_id=[string]` to retrieve only the specified output by tx id

* **Data Params**

  None

* **Success Response:**

  * **Code:** 200
  * **Content:** Array of

    | Field                 | Type     | Description                                                                 |
    |:----------------------|:---------|:----------------------------------------------------------------------------|
    |                       | bool     | Whether it was refreshed from node                                          |
    | -                     | []object | Array of transactions                                                       |
    | - -                   | object   | TxLogEntry                                                                  |
    | - - - id              | number   | Local id for this transaction (distinct from a slate transaction id)        |
    | - - - tx_slate_id     | string   | Slate transaction this entry is associated with, if any                     |
    | - - - tx_type         | string   | Transaction type                                                            |
    | - - - creation_ts     | string   | Time this tx entry was created                                              |
    | - - - confirmation_ts | string   | Time this tx was confirmed (by this wallet)                                 |
    | - - - confirmed       | bool     | Whether the inputs+outputs involved in this transaction have been confirmed |
    | - - - num_inputs      | number   | number of inputs involved in TX                                             |
    | - - - num_outputs     | number   | number of outputs involved in TX                                            |
    | - - - amount_credited | number   | Amount credited via this transaction                                        |
    | - - - amount_debited  | number   | Amount debited via this transaction                                         |
    | - - - fee             | number   |  Fee                                                                        |
    | - - - tx_hex          | string   | The transaction json itself, stored for reference or resending              |

  *Note on transaction type*: transaction type can be either `ConfirmedCoinbase`, `TxReceived`, `TxSent`, `TxReceivedCancelled` and `TxSentCancelled`.

* **Error Response:**

  * **Code:** 400

* **Sample Call:**

  ```javascript
    $.ajax({
      url: "/v1/wallet/foreign/retrieve_txs",
      dataType: "json",
      type : "GET",
      success : function(r) {
        console.log(r);
      }
    });
  ```

### GET Dump Stored Tx

Retrieves a given transaction.

* **URL**

  /v1/wallet/owner/dump_stored_tx?id=x

* **Method:**

  `GET`
  
* **URL Params**

  **Required:**
  `id=[number]`

* **Data Params**

  None

* **Success Response:**

  * **Code:** 200
  * **Content:** A deserialized transaction.

    | Field                 | Type     | Description                                                               |
    |:----------------------|:---------|:--------------------------------------------------------------------------|
    |                       | object   | The core transaction data (inputs, outputs, kernels and kernel offset)    |
    | - offset              | []number | The kernel "offset" k2, excess is k1G after splitting the key k = k1 + k2 |
    | - body                | object   | The transaction body - inputs/outputs/kernels                             |
    | - - inputs            | []object | List of inputs spent by the transaction                                   |
    | - - - features        | object   | The features of the output being spent                                    |
    | - - - - bits          | number   | Representation of the features in bits                                    |
    | - - - commit          | []number | The commit referencing the output being spent                             |
    | - - outputs           | []object | List of outputs the transaction produces                                  |
    | - - - features        | object   | Options for an output's structure or use                                  |
    | - - - - bits          | number   | Representation of the features in bits                                    |
    | - - - commit          | []number | The homomorphic commitment representing the output amount                 |
    | - - - proof           | []number | A proof that the commitment is in the right range                         |
    | - - kernels           | []object | List of kernels that make up this transaction (usually a single kernel)   |
    | - - - features        | object   | Options for a kernel's structure or use                                   |
    | - - - - bits          | number   | Representation of the features in bits                                    |
    | - - - fee             | number   | Fee originally included in the transaction this proof is for              |
    | - - - lock_height     | number   | The max lock_height of all inputs to this transaction                     |
    | - - - excess          | []number | Remainder of the sum of all transaction commitments                       |
    | - - - excess_sig      | []number | The signature proving the excess is a valid public key (signs the tx fee) |

* **Error Response:**

  * **Code:** 400

* **Sample Call:**

  ```javascript
    $.ajax({
      url: "/v1/wallet/owner/dump_stored_tx?id=13",
      dataType: "json",
      type : "GET",
      success : function(r) {
        console.log(r);
      }
    });
  ```

### POST Issue Send Tx

Send a transaction either directly by http or file (then display the slate)

* **URL**

  /v1/wallet/owner/issue_send_tx

* **Method:**

  `POST`
  
* **URL Params**

  None

* **Data Params**

  **Required:**

    | Field                         | Type     | Description                          |
    |:------------------------------|:---------|:-------------------------------------|
    | amount                        | number   | Amount to send                       |
    | minimum_confirmations         | number   | Minimum confirmations                |
    | method                        | string   | Payment method                       |
    | dest                          | string   | Destination url                      |
    | max_outputs                   | number   | Max number of outputs                |
    | num_change_outputs            | number   | Number of change outputs to generate |
    | selection_strategy_is_use_all | bool     | Whether to use all outputs (combine) |
    | fluff                         | bool     | Dandelion control                    |

* **Success Response:**

  * **Code:** 200
  * **Content:** A new transaction slate in JSON.

    | Field                 | Type     | Description                                                               |
    |:----------------------|:---------|:--------------------------------------------------------------------------|
    | num_participants      | number   | The number of participants intended to take part in this transaction      |
    | id                    | number   | Unique transaction ID, selected by sender                                 |
    | tx                    | object   | The core transaction data (inputs, outputs, kernels and kernel offset)    |
    | - offset              | []number | The kernel "offset" k2, excess is k1G after splitting the key k = k1 + k2 |
    | - body                | object   | The transaction body - inputs/outputs/kernels                             |
    | - - inputs            | []object | List of inputs spent by the transaction                                   |
    | - - - features        | object   | The features of the output being spent                                    |
    | - - - - bits          | number   | Representation of the features in bits                                    |
    | - - - commit          | []number | The commit referencing the output being spent                             |
    | - - outputs           | []object | List of outputs the transaction produces                                  |
    | - - - features        | object   | Options for an output's structure or use                                  |
    | - - - - bits          | number   | Representation of the features in bits                                    |
    | - - - commit          | []number | The homomorphic commitment representing the output amount                 |
    | - - - proof           | []number | A proof that the commitment is in the right range                         |
    | - - kernels           | []object | List of kernels that make up this transaction (usually a single kernel)   |
    | - - - features        | object   | Options for a kernel's structure or use                                   |
    | - - - - bits          | number   | Representation of the features in bits                                    |
    | - - - fee             | number   | Fee originally included in the transaction this proof is for              |
    | - - - lock_height     | number   | The max lock_height of all inputs to this transaction                     |
    | - - - excess          | []number | Remainder of the sum of all transaction commitments                       |
    | - - - excess_sig      | []number | The signature proving the excess is a valid public key (signs the tx fee) |
    | amount                | number   | Base amount (excluding fee)                                               |
    | fee                   | number   | Fee amount                                                                |
    | height                | number   | Block height for the transaction                                          |
    | lock_height           | number   | Lock height                                                               |
    | participant_data      | object   | Participant data                                                          |
    | - id                  | number   | Id of participant in the transaction. (For now, 0=sender, 1=rec)          |
    | - public_blind_excess | []number | Public key corresponding to private blinding factor                       |
    | - public_nonce        | []number | Public key corresponding to private nonce                                 |
    | - part_sig            | []number | Public partial signature                                                  |

* **Error Response:**

  * **Code:** 400

* **Sample Call:**

  ```javascript
    $.ajax({
      url: "/v1/wallet/owner/issue_send_tx",
      dataType: "json",
      type : "POST",
      success : function(r) {
        console.log(r);
      },
      data: {
        file: tx_args.json
      },
    });
  ```

### POST Finalize Tx

Sender finalization of the transaction. Takes the slate returned by the sender as well as the private file generate on the first send step.
Builds the complete transaction and sends it to a grin node for propagation.

* **URL**

  /v1/wallet/owner/finalize_tx

* **Method:**

  `POST`
  
* **URL Params**

  None

* **Data Params**

  **Required:** A transaction slate in JSON.

    | Field                 | Type     | Description                                                               |
    |:----------------------|:---------|:--------------------------------------------------------------------------|
    | num_participants      | number   | The number of participants intended to take part in this transaction      |
    | id                    | number   | Unique transaction ID, selected by sender                                 |
    | tx                    | object   | The core transaction data (inputs, outputs, kernels and kernel offset)    |
    | - offset              | []number | The kernel "offset" k2, excess is k1G after splitting the key k = k1 + k2 |
    | - body                | object   | The transaction body - inputs/outputs/kernels                             |
    | - - inputs            | []object | List of inputs spent by the transaction                                   |
    | - - - features        | object   | The features of the output being spent                                    |
    | - - - - bits          | number   | Representation of the features in bits                                    |
    | - - - commit          | []number | The commit referencing the output being spent                             |
    | - - outputs           | []object | List of outputs the transaction produces                                  |
    | - - - features        | object   | Options for an output's structure or use                                  |
    | - - - - bits          | number   | Representation of the features in bits                                    |
    | - - - commit          | []number | The homomorphic commitment representing the output amount                 |
    | - - - proof           | []number | A proof that the commitment is in the right range                         |
    | - - kernels           | []object | List of kernels that make up this transaction (usually a single kernel)   |
    | - - - features        | object   | Options for a kernel's structure or use                                   |
    | - - - - bits          | number   | Representation of the features in bits                                    |
    | - - - fee             | number   | Fee originally included in the transaction this proof is for              |
    | - - - lock_height     | number   | The max lock_height of all inputs to this transaction                     |
    | - - - excess          | []number | Remainder of the sum of all transaction commitments                       |
    | - - - excess_sig      | []number | The signature proving the excess is a valid public key (signs the tx fee) |
    | amount                | number   | Base amount (excluding fee)                                               |
    | fee                   | number   | Fee amount                                                                |
    | height                | number   | Block height for the transaction                                          |
    | lock_height           | number   | Lock height                                                               |
    | participant_data      | object   | Participant data                                                          |
    | - id                  | number   | Id of participant in the transaction. (For now, 0=sender, 1=rec)          |
    | - public_blind_excess | []number | Public key corresponding to private blinding factor                       |
    | - public_nonce        | []number | Public key corresponding to private nonce                                 |
    | - part_sig            | []number | Public partial signature                                                  |

* **Success Response:**

  * **Code:** 200
  * **Content:** A new transaction slate in JSON.

    | Field                 | Type     | Description                                                               |
    |:----------------------|:---------|:--------------------------------------------------------------------------|
    | num_participants      | number   | The number of participants intended to take part in this transaction      |
    | id                    | number   | Unique transaction ID, selected by sender                                 |
    | tx                    | object   | The core transaction data (inputs, outputs, kernels and kernel offset)    |
    | - offset              | []number | The kernel "offset" k2, excess is k1G after splitting the key k = k1 + k2 |
    | - body                | object   | The transaction body - inputs/outputs/kernels                             |
    | - - inputs            | []object | List of inputs spent by the transaction                                   |
    | - - - features        | object   | The features of the output being spent                                    |
    | - - - - bits          | number   | Representation of the features in bits                                    |
    | - - - commit          | []number | The commit referencing the output being spent                             |
    | - - outputs           | []object | List of outputs the transaction produces                                  |
    | - - - features        | object   | Options for an output's structure or use                                  |
    | - - - - bits          | number   | Representation of the features in bits                                    |
    | - - - commit          | []number | The homomorphic commitment representing the output amount                 |
    | - - - proof           | []number | A proof that the commitment is in the right range                         |
    | - - kernels           | []object | List of kernels that make up this transaction (usually a single kernel)   |
    | - - - features        | object   | Options for a kernel's structure or use                                   |
    | - - - - bits          | number   | Representation of the features in bits                                    |
    | - - - fee             | number   | Fee originally included in the transaction this proof is for              |
    | - - - lock_height     | number   | The max lock_height of all inputs to this transaction                     |
    | - - - excess          | []number | Remainder of the sum of all transaction commitments                       |
    | - - - excess_sig      | []number | The signature proving the excess is a valid public key (signs the tx fee) |
    | amount                | number   | Base amount (excluding fee)                                               |
    | fee                   | number   | Fee amount                                                                |
    | height                | number   | Block height for the transaction                                          |
    | lock_height           | number   | Lock height                                                               |
    | participant_data      | object   | Participant data                                                          |
    | - id                  | number   | Id of participant in the transaction. (For now, 0=sender, 1=rec)          |
    | - public_blind_excess | []number | Public key corresponding to private blinding factor                       |
    | - public_nonce        | []number | Public key corresponding to private nonce                                 |
    | - part_sig            | []number | Public partial signature                                                  |

* **Error Response:**

  * **Code:** 400

* **Sample Call:**

  ```javascript
    $.ajax({
      url: "/v1/wallet/owner/finalize_tx",
      dataType: "json",
      type : "POST",
      success : function(r) {
        console.log(r);
      },
      data: {
        file: tx.json
      },
    });
  ```

### POST Cancel Tx

Roll back a transaction and all associated outputs with a given transaction id This means delete all change outputs, (or recipient output if you're recipient), and unlock all locked outputs associated with the transaction used when a transaction is created but never posted.

* **URL**

  * /v1/wallet/owner/cancel_tx?id=x
  * /v1/wallet/owner/cancel_tx?tx_id=x

* **Method:**

  `POST`
  
* **URL Params**

  **Required:**
  * `id=[number]` the transaction id
  * `tx_id=[string]`the transaction slate id

* **Data Params**

  None

* **Success Response:**

  * **Code:** 200

* **Error Response:**

  * **Code:** 400

* **Sample Call:**

  ```javascript
    $.ajax({
      url: "/v1/wallet/owner/cancel_tx?id=3",
      dataType: "json",
      type : "POST",
      success : function(r) {
        console.log(r);
      }
    });
  ```

### POST Post Tx

Push new transaction to the connected node transaction pool. Add `?fluff` at the end of the URL to bypass Dandelion relay.

* **URL**

  /v1/wallet/owner/post_tx

* **Method:**

  `POST`
  
* **URL Params**

  None

* **Data Params**

  **Required:** A transaction slate in JSON.

    | Field                 | Type     | Description                                                               |
    |:----------------------|:---------|:--------------------------------------------------------------------------|
    | num_participants      | number   | The number of participants intended to take part in this transaction      |
    | id                    | number   | Unique transaction ID, selected by sender                                 |
    | tx                    | object   | The core transaction data (inputs, outputs, kernels and kernel offset)    |
    | - offset              | []number | The kernel "offset" k2, excess is k1G after splitting the key k = k1 + k2 |
    | - body                | object   | The transaction body - inputs/outputs/kernels                             |
    | - - inputs            | []object | List of inputs spent by the transaction                                   |
    | - - - features        | object   | The features of the output being spent                                    |
    | - - - - bits          | number   | Representation of the features in bits                                    |
    | - - - commit          | []number | The commit referencing the output being spent                             |
    | - - outputs           | []object | List of outputs the transaction produces                                  |
    | - - - features        | object   | Options for an output's structure or use                                  |
    | - - - - bits          | number   | Representation of the features in bits                                    |
    | - - - commit          | []number | The homomorphic commitment representing the output amount                 |
    | - - - proof           | []number | A proof that the commitment is in the right range                         |
    | - - kernels           | []object | List of kernels that make up this transaction (usually a single kernel)   |
    | - - - features        | object   | Options for a kernel's structure or use                                   |
    | - - - - bits          | number   | Representation of the features in bits                                    |
    | - - - fee             | number   | Fee originally included in the transaction this proof is for              |
    | - - - lock_height     | number   | The max lock_height of all inputs to this transaction                     |
    | - - - excess          | []number | Remainder of the sum of all transaction commitments                       |
    | - - - excess_sig      | []number | The signature proving the excess is a valid public key (signs the tx fee) |
    | amount                | number   | Base amount (excluding fee)                                               |
    | fee                   | number   | Fee amount                                                                |
    | height                | number   | Block height for the transaction                                          |
    | lock_height           | number   | Lock height                                                               |
    | participant_data      | object   | Participant data                                                          |
    | - id                  | number   | Id of participant in the transaction. (For now, 0=sender, 1=rec)          |
    | - public_blind_excess | []number | Public key corresponding to private blinding factor                       |
    | - public_nonce        | []number | Public key corresponding to private nonce                                 |
    | - part_sig            | []number | Public partial signature                                                  |

* **Success Response:**

  * **Code:** 200

* **Error Response:**

  * **Code:** 400

* **Sample Call:**

  ```javascript
    $.ajax({
      url: "/v1/wallet/owner/post_tx",
      dataType: "json",
      type : "POST",
      success : function(r) {
        console.log(r);
      },
      data: {
        file: tx.json
      },
    });
  ```

### POST Issue Burn Tx

Issue a burn TX.

* **URL**

  /v1/wallet/owner/issue_burn_tx

* **Method:**

  `POST`
  
* **URL Params**

  None

* **Data Params**

  None

* **Success Response:**

  * **Code:** 200

* **Error Response:**

  * **Code:** 400

* **Sample Call:**

  ```javascript
    $.ajax({
      url: "/v1/wallet/owner/issue_burn_tx",
      dataType: "json",
      type : "POST",
      success : function(r) {
        console.log(r);
      }
    });
  ```
