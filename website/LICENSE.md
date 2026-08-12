# Licence activation architecture

NoBS PDF uses perpetual, offline-first licensing. A verified one-time Stripe
payment creates one licence that may be active on two devices.

## API

- `POST /api/license/activate` validates the licence, release major version,
  platform, device identifier, and two-device limit. It returns an opaque
  activation ID/token. The token is stored only as a SHA-256 hash server-side.
- `POST /api/license/verify` compares the bearer token hash in constant time and
  returns strict `ACTIVE`, `INVALID`, or `REVOKED` state.
- `POST /api/license/deactivate` releases that device activation.

Responses contain no email, Stripe secret, customer payment data, document
name, path, content, metadata, or processing statistics.

## Desktop behaviour

Initial activation requires internet access. The active credential and stable
device identifier are stored in the operating-system credential store. The app
then works offline indefinitely and quietly attempts verification roughly every
30 days, with one-, three-, and seven-day retry throttling after failures.

Network errors, timeouts, DNS failures, malformed responses, redirects, HTTP
5xx, and mismatched HTTP/state combinations preserve ACTIVE local access. Only
`401 + INVALID` and `403 + REVOKED` disable it. PDF processing uses only the
local activation gate and makes zero licence/network requests.

## Stored server data

Purchases retain the licence key, customer email, Stripe Customer/Checkout
Session/PaymentIntent/Product/Price identifiers, purchase timestamp, release
version, payment/licence status, activation counts, and audit timestamps.
Activations retain hashed device identifiers, hashed activation tokens,
application version, platform, and activation/last-seen/deactivation times.

Legacy subscription columns remain temporarily for reversible migration and
support reconciliation, but do not control perpetual entitlement.
