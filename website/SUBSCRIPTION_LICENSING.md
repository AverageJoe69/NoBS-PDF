# NoBS PDF annual subscription licensing

NoBS PDF has one Stripe subscription: GBP 25.00 per year, with an inclusive-tax
recurring Price. There are no monthly plans or tiers. The configured
`STRIPE_PRICE_ID` is retrieved and checked at Checkout time: it must be active,
GBP 2500, recurring yearly with interval count one, and `tax_behavior=inclusive`.

## Entitlement state

Stripe is the payment source of truth. SQLite stores the minimum server-side
projection needed for licensing: Stripe Customer, Checkout Session,
Subscription, Product and Price identifiers; subscription status; paid-through
timestamp; scheduled-cancellation flag; licence state; and activation records.
Card and billing details are never stored or returned to the desktop.

An entitlement is `ACTIVE` while its server-confirmed `current_period_end` is in
the future and the licence has not been explicitly revoked. Setting
`cancel_at_period_end` does not revoke entitlement. A past-due payment does not
extend the paid-through date, but the already-paid period remains usable. A
later paid invoice updates the subscription and extends the date. Once the date
passes, activation/verification returns authenticated `EXPIRED`. A refunded
charge for the subscription explicitly revokes the licence.

Webhook event writes are signature-verified, recorded in `processed_events`, and
performed transactionally with the entitlement mutation. Duplicate event IDs
are acknowledged without applying the mutation twice.

## Required Stripe webhook events

- `checkout.session.completed`: creates the initial licence only after Stripe
  confirms a paid subscription Checkout Session with the configured Price.
- `checkout.session.async_payment_succeeded`: creates the same initial
  entitlement for delayed payment methods.
- `invoice.paid`: records successful annual renewal or payment recovery and
  advances the paid-through date from Stripe's Subscription.
- `invoice.payment_failed`: records `past_due` without extending or immediately
  revoking the already-paid entitlement.
- `customer.subscription.updated`: records status, paid-through date, and
  `cancel_at_period_end`, including cancellation scheduling and reversal.
- `customer.subscription.deleted`: records terminal cancellation; access still
  lasts only through any remaining paid-through date.
- `charge.refunded`: revokes the associated entitlement after resolving its
  invoice and Subscription.

Subscribe the production endpoint `https://nobspdf.com/webhook` only to these
events. Store its own live Dashboard `whsec_...` value as
`STRIPE_WEBHOOK_SECRET`; a Stripe CLI secret is not interchangeable.

## Desktop offline policy

Activation and successful verification return only authenticated entitlement
state, paid-through Unix timestamp, release/platform information, and the opaque
activation credential. The full credential is stored in the operating-system
credential store.

The normal verification schedule remains approximately every 30 days with
deterministic jitter and one/three/seven-day retry intervals. Before the known
paid-through date the app works offline. After that date it remains usable for a
30-day offline grace period while verification quietly retries. Once the grace
ends, processing requires a successful online check. Network, DNS, timeout,
redirect, malformed-response, rate-limit and server errors never change a local
credential to invalid or revoked. Only strict authenticated protocol responses
can do that. PDF commands consult only the local licence gate and perform no
licensing HTTP requests.

## Customer Portal

Use Stripe's hosted Customer Portal rather than implementing billing forms.
Enable payment-method updates, invoice history and subscription cancellation in
Stripe. The website creates Portal sessions server-side from the verified
Checkout Session and returns only Stripe's short-lived Portal URL.

## Production Stripe checklist

1. Create one live NoBS PDF Product.
   Set `tax_code=txcd_10202001` (downloadable non-recreational software); test
   Product settings do not carry into live mode.
2. Create one recurring Price: GBP 25.00, yearly, tax behaviour inclusive. Do
   not create a monthly Price for this product.
3. Enable Stripe Tax/automatic tax and complete the relevant registrations and
   business tax settings. Confirm Checkout presents the advertised tax-inclusive
   total in every intended market.
4. Set the live Price's `price_...` identifier as `STRIPE_PRICE_ID`.
5. Configure the Customer Portal for cancellation at period end, payment-method
   updates and invoices. Do not enable unsupported plan switching.
6. Create the live webhook with the event set above and set its signing secret.
7. Exercise initial payment, renewal, failure/recovery, cancellation and refund
   in Stripe test mode before switching production traffic.
