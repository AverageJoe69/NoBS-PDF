# NoBS PDF perpetual licensing

NoBS PDF is sold as one GBP 9.99, tax-inclusive, one-time purchase. A verified
Stripe webhook creates one perpetual licence for up to two activated devices.
There is no subscription, renewal, paid-through date, expiry, or offline grace
deadline.

## Entitlement

A paid purchase is `ACTIVE` indefinitely unless it is explicitly refunded or
revoked. The server is authoritative for activation and periodic verification,
but a temporary network, DNS, Railway, Stripe, or server failure never disables
an already active local credential.

The desktop activates online once, stores the opaque activation ID/token in the
operating-system credential store, and verifies quietly about every 30 days.
Only the strict protocol combinations `401 + INVALID` or `403 + REVOKED` change
the stored ACTIVE state. PDF commands perform no licensing HTTP requests.

## Stripe events

The production webhook listens only for:

- `checkout.session.completed`
- `checkout.session.async_payment_succeeded`
- `charge.refunded`

Checkout fulfilment requires a server-retrieved paid `mode=payment` Session
containing exactly one configured Price. The Price must be active, GBP 999,
one-time/non-recurring, inclusive-tax, and attached to the active `NoBS PDF`
Product. The browser success redirect never grants entitlement.

A full `charge.refunded` event revokes the matching PaymentIntent purchase.
Partial refunds do not revoke automatically. Processed Stripe event IDs make
checkout and refund delivery idempotent.

## Database migration

`purchases.entitlement_type` records `perpetual`. On startup, paid non-revoked
subscription-era rows are marked perpetual without deleting any historical
Stripe Subscription, status, period-end, or cancellation fields. Those legacy
columns remain only as reversible migration/support evidence and do not decide
entitlement.

This intentionally converts the existing paid production test purchase to a
perpetual ACTIVE licence while preserving its licence key and activations. A
previously revoked/refunded row remains revoked.
