import assert from "node:assert/strict";
import test from "node:test";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import Database from "better-sqlite3";
import Stripe from "stripe";
import request from "supertest";
import { clientIpForRateLimit, createApp, isPerpetualNoBsPrice, trustCloudflareRailwayProxy } from "../server/app.js";
import { loadConfig } from "../server/config.js";
import { generateLicenceKey } from "../server/license.js";
import { createStore } from "../server/store.js";

const signingSecret = "whsec_test_nobs_pdf";
const stripeSdk = new Stripe("sk_test_placeholder");
const config = {
  stripeWebhookSecret: signingSecret,
  stripePriceId: "price_nobs_test",
  appBaseUrl: "http://localhost:4173",
  releaseVersion: "1.0.0",
  activationLimit: 2,
  releases: { macOS: false, Windows: true },
  downloads: { macOS: "", Windows: "https://downloads.example.test/1.0.0/nobs.exe" },
};

function perpetualPrice(overrides = {}) {
  return {
    id: "price_nobs_test", active: true, currency: "gbp", unit_amount: 999,
    type: "one_time", recurring: null, tax_behavior: "inclusive",
    product: { id: "prod_nobs_test", name: "NoBS PDF", active: true },
    ...overrides,
  };
}

function paidSession(overrides = {}) {
  return {
    id: "cs_test_1234567890", created: 1_700_000_000, payment_status: "paid", mode: "payment",
    customer_details: { email: "buyer@example.com" }, customer: "cus_test_123",
    payment_intent: "pi_test_123", ...overrides,
  };
}

function harness({ session = paidSession(), price = perpetualPrice(), appConfig = config } = {}) {
  const store = createStore(":memory:");
  const calls = [];
  const stripe = {
    webhooks: stripeSdk.webhooks,
    prices: { retrieve: async (_id, options) => { calls.push({ priceOptions: options }); return price; } },
    checkout: { sessions: {
      create: async (params) => { calls.push({ checkout: params }); return { id: "cs_test_created123", url: "https://checkout.stripe.test/session" }; },
      retrieve: async () => session,
      listLineItems: async (_id, options) => {
        calls.push({ lineItemOptions: options });
        return { data: [{ quantity: 1, price }] };
      },
    } },
  };
  return { app: createApp({ stripe, store, config: appConfig }), store, calls };
}

function seedLicence(store, suffix = "seed") {
  const result = store.recordPurchase({ id: `evt_seed_${suffix}`, type: "checkout.session.completed" }, {
    customerEmail: "buyer@example.com", stripeCustomerId: "cus_test",
    checkoutSessionId: `cs_test_${suffix}12345678`, paymentIntentId: `pi_${suffix}`,
    productId: "prod_nobs_test", priceId: "price_nobs_test",
    purchaseTimestamp: new Date().toISOString(), releaseVersion: "1.0.0",
  });
  return result.purchase.licence_key;
}

function activationBody(key, device = "00000000-0000-4000-8000-000000000001") {
  return { license_key: key, device_identifier: device, app_version: "1.0.0", platform: "windows" };
}

function signedWebhook(type, object, eventId) {
  const payload = JSON.stringify({ id: eventId, type, data: { object } });
  const signature = stripeSdk.webhooks.generateTestHeaderString({ payload, secret: signingSecret });
  return { payload, signature };
}

async function deliver(app, event) {
  return request(app).post("/webhook").set("stripe-signature", event.signature)
    .set("content-type", "application/json").send(event.payload);
}

test("configuration safety rules remain enforced", () => {
  assert.throws(() => loadConfig({}), /STRIPE_SECRET_KEY.*STRIPE_WEBHOOK_SECRET.*STRIPE_PRICE_ID.*APP_BASE_URL/);
  assert.throws(() => loadConfig({
    NODE_ENV: "production", STRIPE_SECRET_KEY: "sk_test_wrong", STRIPE_WEBHOOK_SECRET: "whsec_test",
    STRIPE_PRICE_ID: "price_live", APP_BASE_URL: "http://localhost", DATABASE_PATH: "/tmp/nobs.sqlite",
    NOBS_RELEASE_VERSION: "1.0.0", MACOS_RELEASE_ENABLED: "false", WINDOWS_RELEASE_ENABLED: "false",
  }), /Unsafe production configuration/);
});

test("perpetual Price validation requires the exact active GBP 9.99 inclusive one-time Product", () => {
  assert.equal(isPerpetualNoBsPrice(perpetualPrice(), config.stripePriceId), true);
  assert.equal(isPerpetualNoBsPrice(perpetualPrice({ product: { id: "prod_nobs_11", name: "NoBS PDF 1.1", active: true } }), config.stripePriceId), true);
  for (const price of [
    perpetualPrice({ id: "price_wrong" }), perpetualPrice({ active: false }),
    perpetualPrice({ currency: "usd" }), perpetualPrice({ unit_amount: 2500 }),
    perpetualPrice({ type: "recurring", recurring: { interval: "year", interval_count: 1 } }),
    perpetualPrice({ tax_behavior: "exclusive" }),
    perpetualPrice({ product: { id: "prod_wrong", name: "Another Product", active: true } }),
  ]) assert.equal(isPerpetualNoBsPrice(price, config.stripePriceId), false);
});

test("Checkout uses one-time payment mode, automatic tax, and configured Price", async (t) => {
  const { app, store, calls } = harness(); t.after(() => store.close());
  const response = await request(app).post("/api/checkout").send({});
  assert.equal(response.status, 201);
  const checkout = calls.find(call => call.checkout).checkout;
  assert.equal(checkout.mode, "payment");
  assert.deepEqual(checkout.line_items, [{ price: config.stripePriceId, quantity: 1 }]);
  assert.deepEqual(checkout.automatic_tax, { enabled: true });
  assert.match(checkout.success_url, /session_id=\{CHECKOUT_SESSION_ID\}$/);
});

test("wrong or recurring configured Price prevents Checkout", async (t) => {
  for (const price of [perpetualPrice({ id: "price_wrong" }), perpetualPrice({ type: "recurring", recurring: { interval: "year", interval_count: 1 } })]) {
    const { app, store } = harness({ price }); t.after(() => store.close());
    assert.equal((await request(app).post("/api/checkout").send({})).status, 502);
  }
});

test("public configuration exposes the current release version for manual update checks", async (t) => {
  const { app, store } = harness(); t.after(() => store.close());
  const response = await request(app).get("/api/config");
  assert.equal(response.status, 200);
  assert.equal(response.type, "application/json");
  assert.equal(response.body.releaseVersion, "1.0.0");
  assert.deepEqual(response.body.releases, config.releases);
});

test("Checkout is unavailable when no platform has been released", async (t) => {
  const unavailable = { ...config, releases: { macOS: false, Windows: false }, downloads: { macOS: "", Windows: "" } };
  const { app, store, calls } = harness({ appConfig: unavailable }); t.after(() => store.close());
  const response = await request(app).post("/api/checkout").send({});
  assert.equal(response.status, 503);
  assert.equal(response.type, "application/json");
  assert.match(response.body.error, /coming soon/i);
  assert.equal(calls.length, 0);
});

test("unpaid Checkout creates no licence", async (t) => {
  const session = paidSession({ payment_status: "unpaid" });
  const { app, store } = harness({ session }); t.after(() => store.close());
  const response = await deliver(app, signedWebhook("checkout.session.completed", { id: session.id }, "evt_unpaid"));
  assert.equal(response.status, 200);
  assert.equal(response.body.fulfilled, false);
  assert.equal(store.findPurchaseBySession(session.id), null);
});

test("paid Checkout creates exactly one perpetual licence and duplicate delivery is idempotent", async (t) => {
  const session = paidSession();
  const { app, store } = harness({ session }); t.after(() => store.close());
  const event = signedWebhook("checkout.session.completed", { id: session.id }, "evt_paid");
  assert.equal((await deliver(app, event)).status, 200);
  const purchase = store.findPurchaseBySession(session.id);
  assert.ok(purchase);
  assert.equal(purchase.entitlement_type, "perpetual");
  assert.equal(purchase.payment_status, "paid");
  assert.equal(purchase.licence_status, "active");
  assert.equal(purchase.stripe_payment_intent_id, session.payment_intent);
  assert.equal(purchase.current_period_end, null);
  assert.equal(purchase.stripe_subscription_id, null);
  const duplicate = await deliver(app, event);
  assert.equal(duplicate.body.duplicate, true);
  assert.equal(store.processedEventCount("evt_paid"), 1);
  assert.equal(store.findPurchaseBySession(session.id).licence_key, purchase.licence_key);
});

test("wrong Checkout Price grants no entitlement", async (t) => {
  const session = paidSession();
  const { app, store } = harness({ session, price: perpetualPrice({ id: "price_wrong" }) }); t.after(() => store.close());
  const response = await deliver(app, signedWebhook("checkout.session.completed", { id: session.id }, "evt_wrong_price"));
  assert.equal(response.status, 400);
  assert.equal(store.findPurchaseBySession(session.id), null);
});

test("two devices activate, third is rejected, and active verification has no expiry fields", async (t) => {
  const { app, store } = harness(); t.after(() => store.close());
  const key = seedLicence(store, "devices");
  const first = await request(app).post("/api/license/activate").send(activationBody(key));
  const second = await request(app).post("/api/license/activate").send(activationBody(key, "00000000-0000-4000-8000-000000000002"));
  const third = await request(app).post("/api/license/activate").send(activationBody(key, "00000000-0000-4000-8000-000000000003"));
  assert.equal(first.status, 201); assert.equal(second.status, 201); assert.equal(third.status, 409);
  assert.equal(first.body.entitlement_state, "PERPETUAL");
  assert.equal("paid_through" in first.body, false);
  assert.equal("cancel_at_period_end" in first.body, false);
  const verified = await request(app).post("/api/license/verify")
    .set("authorization", `Bearer ${first.body.activation_token}`).send({ activation_id: first.body.activation_id });
  assert.equal(verified.status, 200);
  assert.deepEqual(Object.keys(verified.body).sort(), ["activation_id", "entitlement_state", "platform", "release_version", "state", "valid"]);
});

test("full refund revokes, partial refund does not, and duplicate refund is idempotent", async (t) => {
  const { app, store } = harness(); t.after(() => store.close());
  const partialKey = seedLicence(store, "partial");
  const partial = signedWebhook("charge.refunded", { id: "ch_partial", refunded: false, payment_intent: "pi_partial" }, "evt_partial");
  assert.equal((await deliver(app, partial)).body.updated, false);
  assert.equal((await request(app).post("/api/license/activate").send(activationBody(partialKey))).status, 201);

  const key = seedLicence(store, "full");
  const full = signedWebhook("charge.refunded", { id: "ch_full", refunded: true, payment_intent: "pi_full" }, "evt_full");
  assert.equal((await deliver(app, full)).body.updated, true);
  assert.equal((await deliver(app, full)).body.duplicate, true);
  const rejected = await request(app).post("/api/license/activate").send(activationBody(key));
  assert.equal(rejected.status, 403);
  assert.equal(rejected.body.state, "REVOKED");
  assert.equal(store.processedEventCount("evt_full"), 1);
});

test("subscription-only webhooks are ignored and Customer Portal is absent", async (t) => {
  const { app, store } = harness(); t.after(() => store.close());
  const event = signedWebhook("invoice.paid", { id: "in_old" }, "evt_old_invoice");
  assert.deepEqual((await deliver(app, event)).body, { received: true });
  assert.equal(store.processedEventCount("evt_old_invoice"), 0);
  assert.equal((await request(app).post("/api/billing/portal").send({})).status, 404);
});

test("invalid webhook signature and malformed licence requests fail safely", async (t) => {
  const { app, store } = harness(); t.after(() => store.close());
  assert.equal((await request(app).post("/webhook").set("stripe-signature", "bad").set("content-type", "application/json").send("{}")).status, 400);
  const malformed = await request(app).post("/api/license/activate").send({ license_key: "bad" });
  assert.equal(malformed.status, 400);
  assert.equal(malformed.body.state, "INVALID");
});

test("legacy paid subscription-era purchase migrates to perpetual ACTIVE without data loss", (t) => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "nobs-legacy-"));
  const filename = path.join(directory, "legacy.sqlite");
  const db = new Database(filename);
  db.exec(`CREATE TABLE purchases (
    id INTEGER PRIMARY KEY AUTOINCREMENT, licence_key TEXT NOT NULL UNIQUE, customer_email TEXT NOT NULL,
    stripe_customer_id TEXT, stripe_checkout_session_id TEXT NOT NULL UNIQUE, stripe_payment_intent_id TEXT,
    stripe_product_id TEXT NOT NULL, stripe_price_id TEXT NOT NULL, purchase_timestamp TEXT NOT NULL,
    release_version TEXT NOT NULL, activation_status TEXT NOT NULL DEFAULT 'inactive', activation_count INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL, updated_at TEXT NOT NULL, payment_status TEXT NOT NULL DEFAULT 'paid',
    licence_status TEXT NOT NULL DEFAULT 'active', revoked_at TEXT, stripe_subscription_id TEXT,
    subscription_status TEXT NOT NULL DEFAULT 'incomplete', current_period_end TEXT, cancel_at_period_end INTEGER NOT NULL DEFAULT 0
  )`);
  const now = new Date().toISOString();
  db.prepare(`INSERT INTO purchases (licence_key,customer_email,stripe_checkout_session_id,stripe_product_id,stripe_price_id,
    purchase_timestamp,release_version,created_at,updated_at,payment_status,licence_status,stripe_subscription_id,
    subscription_status,current_period_end) VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?)`)
    .run("NOBS-AAAA-BBBB-CCCC-DDDD", "legacy@example.com", "cs_live_legacy123", "prod_nobs", "price_old_yearly",
      now, "1.0.0", now, now, "paid", "active", "sub_legacy", "active", "2000-01-01T00:00:00.000Z");
  db.close();
  const store = createStore(filename);
  t.after(() => { store.close(); fs.rmSync(directory, { recursive: true, force: true }); });
  const purchase = store.findPurchaseBySession("cs_live_legacy123");
  assert.equal(purchase.entitlement_type, "perpetual");
  const activation = store.activateLicence({ licenceKey: purchase.licence_key, deviceIdentifier: "00000000-0000-4000-8000-000000000001", appVersion: "1.0.0", platform: "windows", limit: 2 });
  assert.equal(activation.state, "ACTIVE");
});

test("proxy and licence-key security helpers remain correct", () => {
  assert.equal(trustCloudflareRailwayProxy("10.0.0.1", 0), true);
  assert.equal(trustCloudflareRailwayProxy("173.245.48.10", 1), true);
  assert.equal(trustCloudflareRailwayProxy("203.0.113.10", 1), false);
  const req = { get: name => ({ "x-real-ip": "198.51.100.20", "x-forwarded-for": "203.0.113.1" })[name.toLowerCase()] || "", socket: { remoteAddress: "10.0.0.1" } };
  assert.equal(clientIpForRateLimit(req), "198.51.100.20");
  const keys = new Set(Array.from({ length: 100 }, () => generateLicenceKey()));
  assert.equal(keys.size, 100);
});
