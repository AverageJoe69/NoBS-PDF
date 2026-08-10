# NoBS PDF Stripe integration

This site uses server-created, Stripe-hosted Checkout Sessions for a £25/year subscription. A verified webhook—not the browser redirect—creates and maintains licence entitlement. Subscription records are stored in SQLite and download destinations remain server-side. See `SUBSCRIPTION_LICENSING.md` for the authoritative lifecycle and event set.

Official references:

- [Checkout Sessions](https://docs.stripe.com/payments/checkout-sessions)
- [Webhooks](https://docs.stripe.com/webhooks)
- [Webhook signatures](https://docs.stripe.com/webhooks/signature)

## Required environment

Copy `.env.example` to `.env` and configure:

```dotenv
STRIPE_SECRET_KEY=sk_test_...
STRIPE_WEBHOOK_SECRET=whsec_...
STRIPE_PRICE_ID=price_...
APP_BASE_URL=http://localhost:4173
PORT=4242
DATABASE_PATH=./data/nobs.sqlite
NOBS_RELEASE_VERSION=1.0.0
MACOS_DOWNLOAD_URL=
WINDOWS_DOWNLOAD_URL=
```

`STRIPE_SECRET_KEY`, `STRIPE_WEBHOOK_SECRET`, and the private download destinations are server-only. Never prefix them with `VITE_`, expose them to browser code, commit `.env`, or log them.

## Stripe Dashboard setup

1. Turn on **Test mode**.
2. Open **Product catalogue**, choose NoBS PDF, and confirm it has one GBP 25.00 yearly recurring Price with inclusive tax behaviour.
3. Copy that Price's `price_...` identifier into `STRIPE_PRICE_ID`. Do not use the `prod_...` Product ID here.
4. For production, open **Workbench → Webhooks**, create an account webhook endpoint at `https://YOUR_DOMAIN/webhook`, and subscribe to the event set in `SUBSCRIPTION_LICENSING.md`.
5. Reveal that endpoint's signing secret and set it as `STRIPE_WEBHOOK_SECRET`. A Dashboard endpoint secret is different from the local Stripe CLI secret.

The displayed price comes from the configured Stripe Price. Checkout is created server-side with `mode: subscription`, quantity `1`, and automatic tax enabled; Stripe Checkout collects the customer's email.

## Run locally in test mode

Install dependencies and create local configuration:

```bash
cd "/Users/joeconway/NoBS PDF/website"
npm install
cp .env.example .env
```

Fill in the test secret key and test Price ID. In terminal one, start the frontend and backend:

```bash
npm run dev
```

In terminal two, authenticate the Stripe CLI and forward only the fulfilment events:

```bash
stripe login
stripe listen \
  --events checkout.session.completed,checkout.session.async_payment_succeeded,invoice.paid,invoice.payment_failed,customer.subscription.updated,customer.subscription.deleted,charge.refunded \
  --forward-to localhost:4242/webhook
```

Copy the `whsec_...` printed by `stripe listen` into `.env` as `STRIPE_WEBHOOK_SECRET`, then restart `npm run dev`.

## Complete test purchase

1. Start the website/backend locally.
2. Start Stripe CLI webhook forwarding.
3. Open `http://localhost:4173`.
4. Click **Subscribe to NoBS PDF**.
5. Complete Checkout with Stripe's test card `4242 4242 4242 4242`, any future expiry, any CVC, and a test email.
6. Stripe sends `checkout.session.completed`.
7. The backend verifies the signature against the unmodified raw request body.
8. The paid Session is retrieved from Stripe and its configured Price is checked.
9. The purchase is committed and a licence is generated atomically.
10. Stripe redirects the customer to `/success?session_id={CHECKOUT_SESSION_ID}`.
11. The page polls the backend until the verified webhook record exists, then displays the email and licence.
12. Mac and Windows download buttons are shown.
13. Verify the payment and Session in the Stripe test Dashboard.
14. In Stripe Workbench, resend the same successful event. The response reports it as duplicate and the original licence remains unchanged.

Run automated coverage with:

```bash
npm test
npm run build
npm audit
```

## Fulfilment and licences

The webhook creates a licence only from a paid subscription Checkout Session whose Subscription contains `STRIPE_PRICE_ID`. Lifecycle events then update the existing entitlement as documented in `SUBSCRIPTION_LICENSING.md`.

Each licence uses 64 bits of cryptographic randomness and the format `NOBS-XXXX-XXXX-XXXX-XXXX`. SQLite uniqueness constraints cover the licence key and Checkout Session ID. The Stripe event ID is also stored. Duplicate event or Session delivery returns successfully without issuing another licence.

Stored fields include the licence key, email, Stripe Customer/Session/Subscription/Product/Price identifiers, subscription status and paid-through date, cancellation scheduling, release, activation status/count, and created/updated timestamps. Card data is never stored.

## Downloads

The browser receives only local authorization endpoints such as:

```text
/api/download/mac?session_id=cs_...
/api/download/windows?session_id=cs_...
```

The backend verifies that the Session has a recorded paid purchase and then redirects to the server-side configured destination. Until `MACOS_DOWNLOAD_URL` or `WINDOWS_DOWNLOAD_URL` is configured, the endpoint returns a clear `503` response. For production, replace simple redirects with short-lived signed object-storage URLs if package URLs must remain non-transferable.

## Production deployment

1. Build with `npm run build` and run with `npm start`.
2. Serve the Node process behind HTTPS and set `APP_BASE_URL` to the exact public origin.
3. Use live-mode `sk_live_...`, `price_...`, and the live Dashboard webhook's `whsec_...`; test and live objects are separate.
4. Mount durable storage for the absolute `DATABASE_PATH`. SQLite is suitable for a single application instance. Before horizontally scaling or deploying to an ephemeral/serverless filesystem, migrate the same schema and uniqueness constraints to managed PostgreSQL.
5. Back up the purchase database and restrict filesystem access to the application user.
6. Configure both platform download destinations and test their installers.
7. Configure real support, privacy, terms, refund, tax, licence-policy, and fulfilment-email content.
8. Add transactional email delivery for the licence and access instructions. The success page works now, but email delivery still requires a provider.

## Security notes

- Webhooks use Stripe SDK signature verification over `express.raw()` bytes. Unverified requests are rejected before processing.
- A success URL is never proof of payment. Unknown or unpaid Sessions cannot retrieve a licence or download.
- Public purchase responses omit Stripe Customer and PaymentIntent identifiers.
- Checkout Session IDs act as high-entropy access references. Do not log full success URLs. For stronger long-term customer retrieval, add expiring download tokens or authenticated email access.
- Restrict production CORS/origins at the reverse proxy, apply rate limiting to checkout and retrieval endpoints, and monitor repeated failures.
- Stripe recommends returning webhook responses promptly. This implementation performs only a Stripe retrieval and local transaction; move email or other slow fulfilment work to a queue before adding it.

Desktop activation, device limits, secure local credential storage, offline behavior, and deactivation are documented in [`LICENSE.md`](./LICENSE.md).
