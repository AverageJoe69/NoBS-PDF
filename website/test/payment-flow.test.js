import assert from "node:assert/strict";
import test from "node:test";
import Stripe from "stripe";
import request from "supertest";
import fs from "node:fs";
import { createApp } from "../server/app.js";
import { loadConfig } from "../server/config.js";
import { generateLicenceKey } from "../server/license.js";
import { createStore } from "../server/store.js";
import { normalizeLicenceKey } from "../shared/license.js";

const signingSecret = "whsec_test_nobs_pdf";
const stripeSdk = new Stripe("sk_test_placeholder");
const config = {
  stripeWebhookSecret: signingSecret,
  stripePriceId: "price_nobs_test",
  appBaseUrl: "http://localhost:4173",
  releaseVersion: "1.0.0",
  activationLimit: 2,
  downloads: { macOS: "https://downloads.example.test/nobs.dmg", Windows: "https://downloads.example.test/nobs.exe" },
};

function paidSession(overrides = {}) {
  const subscription = {
    id: "sub_test_123", status: "active", cancel_at_period_end: false,
    current_period_end: Math.floor(Date.now() / 1000) + 365 * 86400,
    items: { data: [{ price: { id: "price_nobs_test", product: { id: "prod_nobs_test" } } }] },
  };
  return {
    id: "cs_test_1234567890",
    created: 1_700_000_000,
    payment_status: "paid",
    mode: "subscription",
    customer_details: { email: "buyer@example.com" },
    customer: "cus_test_123",
    subscription,
    ...overrides,
  };
}

function seedLicence(store, suffix = "seed", state = {}) {
  const result = store.recordSubscription({ id: `evt_${suffix}`, type: "checkout.session.completed" }, {
    customerEmail: "buyer@example.com", stripeCustomerId: "cus_test", checkoutSessionId: `cs_test_${suffix}12345678`,
    paymentIntentId: null, productId: "prod_nobs_test", priceId: "price_nobs_test", subscriptionId: `sub_${suffix}`,
    subscriptionStatus: "active", currentPeriodEnd: new Date(Date.now() + 365 * 86400_000).toISOString(), cancelAtPeriodEnd: false,
    purchaseTimestamp: new Date().toISOString(), releaseVersion: "1.0.0",
  });
  const key = result.purchase.licence_key;
  if (Object.keys(state).length) store.setLicenceState(key, state);
  return key;
}

function activationBody(key, device = "00000000-0000-4000-8000-000000000001") {
  return { license_key: key, device_identifier: device, app_version: "1.0.0", platform: "macos" };
}

function harness(session = paidSession()) {
  const store = createStore(":memory:");
  const calls = [];
  const stripe = {
    webhooks: stripeSdk.webhooks,
    prices: { retrieve: async () => ({ id: "price_nobs_test", active: true, currency: "gbp", unit_amount: 2500, type: "recurring", recurring: { interval: "year", interval_count: 1 }, tax_behavior: "inclusive", product: { name: "NoBS PDF" } }) },
    checkout: { sessions: {
      create: async (params) => { calls.push(params); return { id: "cs_test_created123", url: "https://checkout.stripe.test/session" }; },
      retrieve: async (_id, options) => {
        assert.deepEqual(options?.expand, ["subscription"]);
        return session;
      },
    } },
    subscriptions: { retrieve: async (id, options) => {
      assert.deepEqual(options?.expand, ["items.data.price.product"]);
      return typeof session.subscription === "object" ? session.subscription : ({ ...paidSession().subscription, id });
    } },
    invoices: { retrieve: async () => ({ subscription: typeof session.subscription === "object" ? session.subscription.id : session.subscription }) },
    paymentIntents: { retrieve: async () => ({ payment_details: { order_reference: session.id } }) },
    billingPortal: { sessions: { create: async (params) => { calls.push({ portal: params }); return { url: "https://billing.stripe.test/session" }; } } },
  };
  return { app: createApp({ stripe, store, config }), store, calls };
}

function signedEvent(session = paidSession(), eventId = "evt_test_123") {
  const payload = JSON.stringify({ id: eventId, type: "checkout.session.completed", data: { object: { id: session.id } } });
  const signature = stripeSdk.webhooks.generateTestHeaderString({ payload, secret: signingSecret });
  return { payload, signature };
}

function signedWebhook(type, object, eventId) {
  const payload = JSON.stringify({ id: eventId, type, data: { object } });
  const signature = stripeSdk.webhooks.generateTestHeaderString({ payload, secret: signingSecret });
  return { payload, signature };
}

async function deliver(app, event) {
  return request(app).post("/webhook").set("stripe-signature", event.signature).set("content-type", "application/json").send(event.payload);
}

test("missing environment variables produce a clear error", () => {
  assert.throws(() => loadConfig({}), /STRIPE_SECRET_KEY.*STRIPE_WEBHOOK_SECRET.*STRIPE_PRICE_ID.*APP_BASE_URL/);
});

test("production configuration rejects test keys, localhost, unsafe storage, and unversioned downloads", () => {
  assert.throws(() => loadConfig({
    NODE_ENV: "production", STRIPE_SECRET_KEY: "sk_test_wrong", STRIPE_WEBHOOK_SECRET: "whsec_test",
    STRIPE_PRICE_ID: "price_live", APP_BASE_URL: "http://localhost:4173", DATABASE_PATH: "/tmp/nobs.sqlite",
    NOBS_RELEASE_VERSION: "1.0.0", MACOS_DOWNLOAD_URL: "http://localhost/app.dmg", WINDOWS_DOWNLOAD_URL: "https://downloads.example.com/app.exe",
  }), /Unsafe production configuration/);
});

test("production configuration accepts explicit live HTTPS versioned values", () => {
  const result = loadConfig({
    NODE_ENV: "production", STRIPE_SECRET_KEY: "sk_live_example", STRIPE_WEBHOOK_SECRET: "whsec_example",
    STRIPE_PRICE_ID: "price_example", APP_BASE_URL: "https://nobspdf.com", DATABASE_PATH: "/var/lib/nobspdf/nobs.sqlite",
    NOBS_RELEASE_VERSION: "1.0.0", MACOS_DOWNLOAD_URL: "https://downloads.nobspdf.com/1.0.0/app.dmg",
    WINDOWS_DOWNLOAD_URL: "https://downloads.nobspdf.com/1.0.0/app.exe",
  });
  assert.equal(result.production, true);
  assert.equal(result.releaseVersion, "1.0.0");
});

test("licence keys are random and correctly formatted", () => {
  const keys = new Set(Array.from({ length: 100 }, () => generateLicenceKey()));
  assert.equal(keys.size, 100);
  for (const key of keys) assert.match(key, /^NOBS-[A-F0-9]{4}(?:-[A-F0-9]{4}){3}$/);
});

test("Checkout Session uses the configured annual subscription Price", async (t) => {
  const { app, store, calls } = harness(); t.after(() => store.close());
  const response = await request(app).post("/api/checkout").send({});
  assert.equal(response.status, 201);
  assert.equal(response.body.url, "https://checkout.stripe.test/session");
  assert.equal(calls[0].mode, "subscription");
  assert.deepEqual(calls[0].line_items, [{ price: config.stripePriceId, quantity: 1 }]);
  assert.deepEqual(calls[0].automatic_tax, { enabled: true });
  assert.equal(calls[0].customer_creation, undefined);
  assert.match(calls[0].success_url, /session_id=\{CHECKOUT_SESSION_ID\}$/);
});

test("invalid webhook signatures are rejected", async (t) => {
  const { app, store } = harness(); t.after(() => store.close());
  const response = await request(app).post("/webhook").set("stripe-signature", "invalid").set("content-type", "application/json").send("{}");
  assert.equal(response.status, 400);
  assert.equal(store.findPurchaseBySession("cs_test_1234567890"), null);
});

test("valid paid checkout.session.completed creates a purchase and licence", async (t) => {
  const session = paidSession();
  const { app, store } = harness(session); t.after(() => store.close());
  const { payload, signature } = signedEvent(session);
  const response = await request(app).post("/webhook").set("stripe-signature", signature).set("content-type", "application/json").send(payload);
  assert.equal(response.status, 200);
  const purchase = store.findPurchaseBySession(session.id);
  assert.equal(purchase.customer_email, "buyer@example.com");
  assert.equal(purchase.stripe_customer_id, "cus_test_123");
  assert.equal(purchase.stripe_subscription_id, "sub_test_123");
  assert.equal(purchase.stripe_product_id, "prod_nobs_test");
  assert.equal(purchase.stripe_price_id, "price_nobs_test");
  assert.equal(purchase.activation_status, "inactive");
  assert.equal(purchase.activation_count, 0);
  assert.equal(purchase.subscription_status, "active");
  assert.ok(Date.parse(purchase.current_period_end) > Date.now());
  assert.match(purchase.licence_key, /^NOBS-/);
});

test("duplicate webhook delivery does not create a second licence", async (t) => {
  const session = paidSession();
  const { app, store } = harness(session); t.after(() => store.close());
  const event = signedEvent(session, "evt_duplicate_123");
  const first = await request(app).post("/webhook").set("stripe-signature", event.signature).set("content-type", "application/json").send(event.payload);
  const licence = store.findPurchaseBySession(session.id).licence_key;
  const second = await request(app).post("/webhook").set("stripe-signature", event.signature).set("content-type", "application/json").send(event.payload);
  assert.equal(first.body.duplicate, false);
  assert.equal(second.body.duplicate, true);
  assert.equal(store.findPurchaseBySession(session.id).licence_key, licence);
  assert.equal(store.processedEventCount("evt_duplicate_123"), 1);
});

test("wrong Stripe Price does not create subscription entitlement", async (t) => {
  const session = paidSession();
  session.subscription.items.data[0].price.id = "price_wrong";
  const { app, store } = harness(session); t.after(() => store.close());
  const response = await deliver(app, signedEvent(session, "evt_wrong_price"));
  assert.equal(response.status, 400);
  assert.equal(store.findPurchaseBySession(session.id), null);
});

test("annual renewal extends paid-through; failed renewal does not; recovery does", async (t) => {
  const session = paidSession();
  const { app, store } = harness(session); t.after(() => store.close());
  await deliver(app, signedEvent(session, "evt_initial_subscription"));
  const initial = store.findPurchaseBySession(session.id).current_period_end;

  session.subscription.current_period_end += 365 * 86400;
  const renewal = await deliver(app, signedWebhook("invoice.paid", { id: "in_renewal", subscription: session.subscription.id }, "evt_renewal"));
  assert.equal(renewal.status, 200);
  const renewed = store.findPurchaseBySession(session.id).current_period_end;
  assert.ok(Date.parse(renewed) > Date.parse(initial));

  session.subscription.current_period_end -= 100;
  session.subscription.status = "past_due";
  await deliver(app, signedWebhook("invoice.payment_failed", { id: "in_failed", subscription: session.subscription.id }, "evt_failed"));
  const failed = store.findPurchaseBySession(session.id);
  assert.equal(failed.subscription_status, "past_due");
  assert.equal(failed.current_period_end, renewed);

  session.subscription.status = "active";
  session.subscription.current_period_end += 400 * 86400;
  await deliver(app, signedWebhook("invoice.paid", { id: "in_recovered", subscription: session.subscription.id }, "evt_recovered"));
  const recovered = store.findPurchaseBySession(session.id);
  assert.equal(recovered.subscription_status, "active");
  assert.ok(Date.parse(recovered.current_period_end) > Date.parse(renewed));
});

test("cancellation at period end stays active until paid-through, then expires", async (t) => {
  const session = paidSession();
  const { app, store } = harness(session); t.after(() => store.close());
  await deliver(app, signedEvent(session, "evt_cancel_initial"));
  const key = store.findPurchaseBySession(session.id).licence_key;
  const activation = await request(app).post("/api/license/activate").send(activationBody(key));
  assert.equal(activation.status, 201);

  session.subscription.cancel_at_period_end = true;
  await deliver(app, signedWebhook("customer.subscription.updated", session.subscription, "evt_cancel_scheduled"));
  const stillActive = await request(app).post("/api/license/verify").set("authorization", `Bearer ${activation.body.activation_token}`).send({ activation_id: activation.body.activation_id });
  assert.equal(stillActive.status, 200);
  assert.equal(stillActive.body.cancel_at_period_end, true);

  session.subscription.status = "canceled";
  await deliver(app, signedWebhook("customer.subscription.deleted", session.subscription, "evt_cancelled"));
  const paidTerm = await request(app).post("/api/license/verify").set("authorization", `Bearer ${activation.body.activation_token}`).send({ activation_id: activation.body.activation_id });
  assert.equal(paidTerm.status, 200);

  store.setLicenceState(key, { subscriptionStatus: "canceled", currentPeriodEnd: new Date(Date.now() - 1000).toISOString() });
  const expired = await request(app).post("/api/license/verify").set("authorization", `Bearer ${activation.body.activation_token}`).send({ activation_id: activation.body.activation_id });
  assert.equal(expired.status, 403);
  assert.equal(expired.body.state, "EXPIRED");
});

test("stale subscription event snapshot cannot overwrite current Stripe state", async (t) => {
  const session = paidSession();
  const { app, store } = harness(session); t.after(() => store.close());
  await deliver(app, signedEvent(session, "evt_order_initial"));
  const currentPeriodEnd = session.subscription.current_period_end;
  const stale = { ...session.subscription, status: "past_due", current_period_end: currentPeriodEnd - 86400, cancel_at_period_end: true };
  const response = await deliver(app, signedWebhook("customer.subscription.updated", stale, "evt_stale_update"));
  assert.equal(response.status, 200);
  const purchase = store.findPurchaseBySession(session.id);
  assert.equal(purchase.subscription_status, "active");
  assert.equal(purchase.cancel_at_period_end, 0);
  assert.equal(purchase.current_period_end, new Date(currentPeriodEnd * 1000).toISOString());
});

test("refunded subscription is explicitly revoked and duplicate lifecycle event is idempotent", async (t) => {
  const session = paidSession();
  const { app, store } = harness(session); t.after(() => store.close());
  await deliver(app, signedEvent(session, "evt_refund_initial"));
  const refund = signedWebhook("charge.refunded", { id: "ch_refunded", refunded: true, invoice: "in_refunded" }, "evt_refunded");
  const first = await deliver(app, refund);
  const second = await deliver(app, refund);
  assert.equal(first.status, 200);
  assert.equal(second.body.duplicate, true);
  assert.equal(store.processedEventCount("evt_refunded"), 1);
  const key = store.findPurchaseBySession(session.id).licence_key;
  assert.equal((await request(app).post("/api/license/activate").send(activationBody(key))).body.state, "REVOKED");
});

test("Managed Payments full refund maps PaymentIntent order reference to subscription; partial refund does not revoke", async (t) => {
  const session = paidSession();
  const { app, store } = harness(session); t.after(() => store.close());
  await deliver(app, signedEvent(session, "evt_managed_refund_initial"));
  const key = store.findPurchaseBySession(session.id).licence_key;
  const partial = await deliver(app, signedWebhook("charge.refunded", { id: "ch_partial", refunded: false, payment_intent: "pi_managed" }, "evt_partial_refund"));
  assert.equal(partial.status, 200);
  assert.equal((await request(app).post("/api/license/activate").send(activationBody(key))).body.state, "ACTIVE");
  const full = await deliver(app, signedWebhook("charge.refunded", { id: "ch_full", refunded: true, payment_intent: "pi_managed" }, "evt_full_refund"));
  assert.equal(full.status, 200);
  assert.equal((await request(app).post("/api/license/activate").send(activationBody(key))).body.state, "REVOKED");
});

test("verified purchase can be retrieved and download is authorized", async (t) => {
  const session = paidSession();
  const { app, store } = harness(session); t.after(() => store.close());
  const event = signedEvent(session);
  await request(app).post("/webhook").set("stripe-signature", event.signature).set("content-type", "application/json").send(event.payload);
  const purchase = await request(app).get(`/api/purchases/${session.id}`);
  assert.equal(purchase.status, 200);
  assert.equal(purchase.body.purchase.email, "buyer@example.com");
  assert.match(purchase.body.purchase.licenceKey, /^NOBS-/);
  assert.equal(purchase.body.purchase.stripeCustomerId, undefined);
  const download = await request(app).get(`/api/download/mac?session_id=${session.id}`);
  assert.equal(download.status, 303);
  assert.equal(download.headers.location, config.downloads.macOS);
});

test("verified subscriber can open Stripe hosted Customer Portal without customer data exposure", async (t) => {
  const session = paidSession();
  const { app, store, calls } = harness(session); t.after(() => store.close());
  await deliver(app, signedEvent(session, "evt_portal"));
  const response = await request(app).post("/api/billing/portal").send({ session_id: session.id });
  assert.equal(response.status, 201);
  assert.deepEqual(response.body, { url: "https://billing.stripe.test/session" });
  assert.equal(calls.at(-1).portal.customer, session.customer);
  assert.equal(response.body.stripe_customer_id, undefined);
});

test("unpaid or unknown purchase cannot retrieve licence or download", async (t) => {
  const session = paidSession({ payment_status: "unpaid" });
  const { app, store } = harness(session); t.after(() => store.close());
  const event = signedEvent(session);
  const webhook = await request(app).post("/webhook").set("stripe-signature", event.signature).set("content-type", "application/json").send(event.payload);
  assert.equal(webhook.body.fulfilled, false);
  assert.equal((await request(app).get(`/api/purchases/${session.id}`)).status, 202);
  assert.equal((await request(app).get(`/api/download/mac?session_id=${session.id}`)).status, 403);
});

test("incomplete subscription does not create entitlement even if Checkout reports paid", async (t) => {
  const session = paidSession();
  session.subscription.status = "incomplete";
  const { app, store } = harness(session); t.after(() => store.close());
  const response = await deliver(app, signedEvent(session, "evt_incomplete"));
  assert.equal(response.status, 200);
  assert.equal(response.body.fulfilled, false);
  assert.equal(store.findPurchaseBySession(session.id), null);
});

test("licence input normalization tolerates case, spaces, and hyphens", () => {
  assert.equal(normalizeLicenceKey(" nobs ab12-cd34 ef56 7890 "), "NOBS-AB12-CD34-EF56-7890");
  assert.equal(normalizeLicenceKey("invalid"), "");
});

test("valid licence activates first and second devices; duplicate device is idempotent", async (t) => {
  const { app, store } = harness(); t.after(() => store.close());
  const key = seedLicence(store, "activate");
  const first = await request(app).post("/api/license/activate").send(activationBody(key));
  assert.equal(first.status, 201);
  assert.equal(first.body.state, "ACTIVE");
  assert.match(first.body.activation_token, /^[A-Za-z0-9_-]+$/);
  const duplicate = await request(app).post("/api/license/activate").send(activationBody(key));
  assert.equal(duplicate.status, 201);
  assert.equal(store.activeActivationCount(key), 1);
  const second = await request(app).post("/api/license/activate").send(activationBody(key, "00000000-0000-4000-8000-000000000002"));
  assert.equal(second.status, 201);
  assert.equal(store.activeActivationCount(key), 2);
});

test("third device is rejected; deactivation frees its slot", async (t) => {
  const { app, store } = harness(); t.after(() => store.close());
  const key = seedLicence(store, "limit");
  const first = await request(app).post("/api/license/activate").send(activationBody(key));
  await request(app).post("/api/license/activate").send(activationBody(key, "00000000-0000-4000-8000-000000000002"));
  const thirdBody = activationBody(key, "00000000-0000-4000-8000-000000000003");
  const third = await request(app).post("/api/license/activate").send(thirdBody);
  assert.equal(third.status, 409);
  assert.equal(third.body.state, "ACTIVATION_LIMIT_REACHED");
  const deactivated = await request(app).post("/api/license/deactivate").set("authorization", `Bearer ${first.body.activation_token}`).send({ activation_id: first.body.activation_id });
  assert.equal(deactivated.status, 200);
  const reusedToken = await request(app).post("/api/license/verify").set("authorization", `Bearer ${first.body.activation_token}`).send({ activation_id: first.body.activation_id });
  assert.equal(reusedToken.status, 401);
  assert.equal((await request(app).post("/api/license/activate").send(thirdBody)).status, 201);
  assert.equal(store.activeActivationCount(key), 2);
});

test("revoked licence cannot retrieve purchase access or downloads", async (t) => {
  const session = paidSession();
  const { app, store } = harness(session); t.after(() => store.close());
  const event = signedEvent(session, "evt_revoke_download");
  await request(app).post("/webhook").set("stripe-signature", event.signature).set("content-type", "application/json").send(event.payload);
  const key = store.findPurchaseBySession(session.id).licence_key;
  store.setLicenceState(key, { licenceStatus: "revoked" });
  assert.equal((await request(app).get(`/api/purchases/${session.id}`)).status, 403);
  assert.equal((await request(app).get(`/api/download/mac?session_id=${session.id}`)).status, 403);
});

test("malformed activation requests are rejected", async (t) => {
  const { app, store } = harness(); t.after(() => store.close());
  const response = await request(app).post("/api/license/activate").send({ license_key: "bad", device_identifier: "hardware-serial", app_version: "debug", platform: "other" });
  assert.equal(response.status, 400);
  assert.equal(response.body.state, "INVALID");
});

test("invalid, unpaid, and revoked licences cannot activate", async (t) => {
  const { app, store } = harness(); t.after(() => store.close());
  assert.equal((await request(app).post("/api/license/activate").send(activationBody("NOBS-AB12-CD34-EF56-7890"))).status, 404);
  const unpaid = seedLicence(store, "unpaid", { currentPeriodEnd: new Date(Date.now() - 1000).toISOString(), subscriptionStatus: "past_due" });
  assert.equal((await request(app).post("/api/license/activate").send(activationBody(unpaid))).status, 403);
  const revoked = seedLicence(store, "revoked", { licenceStatus: "revoked" });
  const response = await request(app).post("/api/license/activate").send(activationBody(revoked));
  assert.equal(response.status, 403);
  assert.equal(response.body.state, "REVOKED");
});

test("activation credential verifies without exposing payment data", async (t) => {
  const { app, store } = harness(); t.after(() => store.close());
  const key = seedLicence(store, "verify");
  const activation = await request(app).post("/api/license/activate").send(activationBody(key));
  const response = await request(app).post("/api/license/verify").set("authorization", `Bearer ${activation.body.activation_token}`).send({ activation_id: activation.body.activation_id });
  assert.equal(response.status, 200);
  assert.equal(response.body.valid, true);
  assert.equal(response.body.stripe_customer_id, undefined);
  assert.equal(response.body.customer_email, undefined);
  assert.equal(response.body.activation_token, undefined);
});

test("complete licence lifecycle activates, verifies, deactivates, and invalidates the credential", async (t) => {
  const { app, store } = harness(); t.after(() => store.close());
  const key = seedLicence(store, "lifecycle");
  const activation = await request(app).post("/api/license/activate").send(activationBody(key));
  assert.equal(activation.status, 201);
  assert.deepEqual(Object.keys(activation.body).sort(), [
    "activation_id", "activation_token", "cancel_at_period_end", "entitlement_state", "license_id", "paid_through", "platform", "release_version", "state", "valid",
  ]);

  const authorization = `Bearer ${activation.body.activation_token}`;
  const verified = await request(app).post("/api/license/verify").set("authorization", authorization)
    .send({ activation_id: activation.body.activation_id });
  assert.equal(verified.status, 200);
  assert.deepEqual(verified.body, {
    valid: true,
    state: "ACTIVE",
    entitlement_state: "ACTIVE",
    activation_id: activation.body.activation_id,
    release_version: "1.0.0",
    platform: "macos",
    paid_through: activation.body.paid_through,
    cancel_at_period_end: false,
  });

  const deactivated = await request(app).post("/api/license/deactivate").set("authorization", authorization)
    .send({ activation_id: activation.body.activation_id });
  assert.equal(deactivated.status, 200);
  assert.deepEqual(deactivated.body, { state: "NOT_ACTIVATED", deactivated: true });

  const after = await request(app).post("/api/license/verify").set("authorization", authorization)
    .send({ activation_id: activation.body.activation_id });
  assert.equal(after.status, 401);
  assert.deepEqual(after.body, { valid: false, state: "INVALID", message: "This activation is not valid." });
});

test("missing and malformed licence requests fail without accepting client-controlled state", async (t) => {
  const { app, store } = harness(); t.after(() => store.close());
  const missing = await request(app).post("/api/license/activate").send({});
  assert.equal(missing.status, 400);
  assert.deepEqual(missing.body, { state: "INVALID", message: "The licence or device information is malformed." });

  const injected = await request(app).post("/api/license/activate").send({
    ...activationBody("NOBS-AB12-CD34-EF56-7890"), state: "ACTIVE", valid: true, activation_limit: 100,
  });
  assert.equal(injected.status, 404);
  assert.deepEqual(injected.body, { state: "INVALID", message: "This licence key is not valid." });

  const malformed = await request(app).post("/api/license/activate")
    .set("content-type", "application/json").send('{"license_key":');
  assert.equal(malformed.status, 400);
  assert.deepEqual(malformed.body, { error: "The request body is malformed." });
  assert.equal(malformed.text.includes("stack"), false);
});

test("database failures return safe JSON and health reports unavailable", async () => {
  const failingStore = {
    activateLicence() { throw new Error("sqlite path and internal details must not escape"); },
    healthCheck() { throw new Error("database unavailable"); },
  };
  const stripe = { webhooks: stripeSdk.webhooks };
  const app = createApp({ stripe, store: failingStore, config });

  const response = await request(app).post("/api/license/activate")
    .send(activationBody("NOBS-AB12-CD34-EF56-7890"));
  assert.equal(response.status, 500);
  assert.deepEqual(response.body, { error: "The request could not be completed." });
  assert.equal(response.text.includes("sqlite"), false);
  assert.equal(response.text.includes("stack"), false);

  const health = await request(app).get("/healthz");
  assert.equal(health.status, 503);
  assert.deepEqual(health.body, { status: "unavailable" });
});

test("health check returns JSON when the database is available", async (t) => {
  const { app, store } = harness(); t.after(() => store.close());
  const response = await request(app).get("/healthz");
  assert.equal(response.status, 200);
  assert.deepEqual(response.body, { status: "ok" });
});

test("unknown API routes return JSON rather than the website fallback", async (t) => {
  const { app, store } = harness(); t.after(() => store.close());
  const response = await request(app).get("/api/license/activate");
  assert.equal(response.status, 404);
  assert.match(response.headers["content-type"], /^application\/json/);
  assert.deepEqual(response.body, { error: "API endpoint not found." });
});

test("browser and desktop source contain no server secrets", () => {
  const sources = ["src/App.tsx", "src/config.ts", "../desktop/src/App.tsx", "../desktop/src-tauri/src/licensing.rs"]
    .map(file => fs.readFileSync(new URL(file, import.meta.url.replace("test/payment-flow.test.js", "")), "utf8")).join("\n");
  assert.doesNotMatch(sources, /sk_(?:test|live)_|whsec_|STRIPE_SECRET_KEY|STRIPE_WEBHOOK_SECRET/);
});
