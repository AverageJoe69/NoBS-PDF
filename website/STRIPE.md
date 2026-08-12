# Stripe production setup

NoBS PDF uses server-created Stripe Checkout Sessions for one GBP 9.99
tax-inclusive payment. The verified webhook—not the success page—issues the
perpetual licence. See `PERPETUAL_LICENSING.md` for the state model.

## Required variables

```text
STRIPE_SECRET_KEY=sk_live_...
STRIPE_WEBHOOK_SECRET=whsec_...
STRIPE_PRICE_ID=price_...
APP_BASE_URL=https://nobs-pdf.com
```

Never commit these values.

## Live-mode Price preparation

1. In Stripe live mode, open the existing correct **NoBS PDF** downloadable-
   software Product. Confirm its tax code remains the intended downloadable
   software tax code.
2. Add a new Price to that Product: **GBP 9.99**, **one time**, tax behaviour
   **inclusive**, active. Do not create a recurring interval.
3. Copy its `price_...` ID. Do not use the Product ID.
4. Do not change Railway yet. First deploy/test this code with a matching Stripe
   test-mode one-time Price.
5. For production cutover, set Railway `STRIPE_PRICE_ID` to the new live Price
   and deploy the application together. The server refuses checkout if any
   Price property is wrong.
6. After production checkout/refund verification, archive the old GBP 25/year
   Price so it cannot be selected for new sales. Do not delete the Product or
   historical Price.

## Webhook

The endpoint is:

```text
https://nobs-pdf.com/webhook
```

Configure exactly these events:

```text
checkout.session.completed
checkout.session.async_payment_succeeded
charge.refunded
```

Remove invoice and customer-subscription events after the perpetual deployment
is healthy. Changing the event selection does not require rotating the signing
secret. If a new endpoint is created, use its newly issued `whsec_...` value.

Customer Portal is intentionally not used: there is no subscription to manage.
Stripe's normal receipt/payment email provides the purchase receipt.
