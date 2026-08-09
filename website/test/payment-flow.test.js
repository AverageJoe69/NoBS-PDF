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
  return {
    id: "cs_test_1234567890",
    created: 1_700_000_000,
    payment_status: "paid",
    customer_details: { email: "buyer@example.com" },
    customer: "cus_test_123",
    payment_intent: "pi_test_123",
    line_items: { data: [{ price: { id: "price_nobs_test", product: { id: "prod_nobs_test" } } }] },
    ...overrides,
  };
}

function seedLicence(store, suffix = "seed", state = {}) {
  const result = store.recordPurchase({ id: `evt_${suffix}`, type: "checkout.session.completed" }, {
    customerEmail: "buyer@example.com", stripeCustomerId: "cus_test", checkoutSessionId: `cs_test_${suffix}12345678`,
    paymentIntentId: "pi_test", productId: "prod_nobs_test", priceId: "price_nobs_test",
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
    prices: { retrieve: async () => ({ currency: "gbp", unit_amount: 4900, product: { name: "NoBS PDF" } }) },
    checkout: { sessions: {
      create: async (params) => { calls.push(params); return { id: "cs_test_created123", url: "https://checkout.stripe.test/session" }; },
      retrieve: async () => session,
    } },
  };
  return { app: createApp({ stripe, store, config }), store, calls };
}

function signedEvent(session = paidSession(), eventId = "evt_test_123") {
  const payload = JSON.stringify({ id: eventId, type: "checkout.session.completed", data: { object: { id: session.id } } });
  const signature = stripeSdk.webhooks.generateTestHeaderString({ payload, secret: signingSecret });
  return { payload, signature };
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

test("Checkout Session uses the configured one-time Price", async (t) => {
  const { app, store, calls } = harness(); t.after(() => store.close());
  const response = await request(app).post("/api/checkout").send({});
  assert.equal(response.status, 201);
  assert.equal(response.body.url, "https://checkout.stripe.test/session");
  assert.equal(calls[0].mode, "payment");
  assert.deepEqual(calls[0].line_items, [{ price: config.stripePriceId, quantity: 1 }]);
  assert.equal(calls[0].customer_creation, "always");
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
  assert.equal(purchase.stripe_payment_intent_id, "pi_test_123");
  assert.equal(purchase.stripe_product_id, "prod_nobs_test");
  assert.equal(purchase.stripe_price_id, "price_nobs_test");
  assert.equal(purchase.activation_status, "inactive");
  assert.equal(purchase.activation_count, 0);
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

test("unpaid or unknown purchase cannot retrieve licence or download", async (t) => {
  const session = paidSession({ payment_status: "unpaid" });
  const { app, store } = harness(session); t.after(() => store.close());
  const event = signedEvent(session);
  const webhook = await request(app).post("/webhook").set("stripe-signature", event.signature).set("content-type", "application/json").send(event.payload);
  assert.equal(webhook.body.fulfilled, false);
  assert.equal((await request(app).get(`/api/purchases/${session.id}`)).status, 202);
  assert.equal((await request(app).get(`/api/download/mac?session_id=${session.id}`)).status, 403);
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
  const unpaid = seedLicence(store, "unpaid", { paymentStatus: "unpaid" });
  assert.equal((await request(app).post("/api/license/activate").send(activationBody(unpaid))).status, 404);
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

test("browser and desktop source contain no server secrets", () => {
  const sources = ["src/App.tsx", "src/config.ts", "../desktop/src/App.tsx", "../desktop/src-tauri/src/licensing.rs"]
    .map(file => fs.readFileSync(new URL(file, import.meta.url.replace("test/payment-flow.test.js", "")), "utf8")).join("\n");
  assert.doesNotMatch(sources, /sk_(?:test|live)_|whsec_|STRIPE_SECRET_KEY|STRIPE_WEBHOOK_SECRET/);
});
