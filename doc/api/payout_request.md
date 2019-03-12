# Grin Business to Customer Payout API

## Abstarct
We propose an API to simplify the process of getting B2C payouts, for example withdrawals from pools and exchanges. 

## Motivation
Currently two methods of getting payouts are used in the most cases - file exchange and HTTP(S) `send`. File exchange is a low-level method which is also used under the hood by HTTP method. It provides a great flexibility but at the same time pretty complex and delivers a poor user experience. HTTP(S) `send` requires an HTTP(S) server listening on the receiver (customer) side, which is a high bar for the majority of home users and mobile devices, because client's port must be reachable from the sender.

We propose a method to eliminate the need of a server side component on the receiver side (customer) and at the same time to keep the simplicity of HTTP(S) interaction.

## User story
We have 2 actors - Customer and Business. The Customer needs to receive payout from the Business.

1. Customer logins to the Busines's web site and starts a payout request procedure. This part is completely up to Business, in the simplest from it could be just a login/password form. Any additional authentication methods could be used, multi factor authentication, WebAuthn [1], phone call, face to face meeting etc. As the result Customer is provided with an unique URL (may be a short-lived) which can be used to request a payout, for example `https://business.com/customer1/payout-requests/deadbeef`. Optionally Customer can set the amount of payout which must be stored on the side of Buisness along with URL.

2. Customer sends a POST request to this address (eg `grin wallet request URL`) from their wallet with their credentials (login/password). We suggest to use HTTP Basic authentication scheme[2], however other schemes could be used, including custom ones. It would require support on the wallet side though. In the simlplest form the body of a request is empty, in this case amount defined in step 1 (or all remaining funds) will be transfered to Customer wallet. For more fine grained control an invoice could be sent as the requets body, however Grin wallet doesn't support invoice functionality at the moment.

3. Business checks the request and credentials, if the user is authenticated but sent an invalid request more than N times in a row their account should be locked. If the request is valid, Business generates a slate (more or less `grin wallet send` functionality) and sends it back as HTTP(S) response. 

4. Customer receives the response, their wallet executes the same steps as in case of `grin wallet receive` and sends the result back to the original URL + `/finalize`

5. Business checks the request and proceeds with finalization steps, sending `HTTP 201 Created` as a response.

This flows provides configurable security in a flexible way and at the same time enables one click payout requests in Customer's wallet without requiring any server side components on their side (can work with NAT'd home connection as well as mobile devices).

## Specification

TBD

[1] https://en.wikipedia.org/wiki/WebAuthn
[2] http://www.rfc-editor.org/info/rfc7617
