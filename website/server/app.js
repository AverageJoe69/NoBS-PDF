import express from "express";
import { createHash } from "node:crypto";
import { LICENCE_PATTERN, normalizeLicenceKey } from "../shared/license.js";
import { noOpLogger, safeReference } from "./logger.js";

const SESSION_ID = /^cs_(?:test_|live_)?[A-Za-z0-9]{8,}$/;
const platforms = new Set(["macOS", "Windows"]);
const clientPlatforms = new Set(["macos", "windows"]);
const SUBSCRIPTION_EVENTS = new Set([
  "checkout.session.completed", "checkout.session.async_payment_succeeded",
  "invoice.paid", "invoice.payment_failed",
  "customer.subscription.updated", "customer.subscription.deleted",
  "charge.refunded",
]);

function subscriptionPeriodEnd(subscription) {
  const seconds = subscription?.current_period_end ?? subscription?.items?.data?.[0]?.current_period_end;
  return Number.isFinite(seconds) ? new Date(seconds * 1000).toISOString() : null;
}

function subscriptionPrice(subscription) {
  const price = subscription?.items?.data?.[0]?.price;
  const product = typeof price?.product === "object" ? price.product : null;
  return { priceId: typeof price === "string" ? price : price?.id, productId: typeof price?.product === "string" ? price.product : product?.id };
}

function paidThroughSeconds(value) {
  const milliseconds = Date.parse(value || "");
  return Number.isFinite(milliseconds) ? Math.floor(milliseconds / 1000) : null;
}

function isAnnualNoBsPrice(price, configuredId) {
  return price?.id === configuredId && price.active !== false && price.currency === "gbp" && price.unit_amount === 2500
    && price.type === "recurring" && price.recurring?.interval === "year" && price.recurring?.interval_count === 1
    && price.tax_behavior === "inclusive";
}

function activationRateLimit({ windowMs = 60_000, maximum = 10 } = {}) {
  const attempts = new Map();
  return (req, res, next) => {
    const key = createHash("sha256").update(req.ip || "unknown").digest("hex");
    const now = Date.now();
    const current = attempts.get(key);
    if (!current || current.expires <= now) {
      attempts.set(key, { count: 1, expires: now + windowMs });
      return next();
    }
    current.count += 1;
    if (current.count > maximum) return res.status(429).json({ state: "INVALID", message: "Too many activation attempts. Please wait a minute and try again." });
    return next();
  };
}

function bearerToken(req) {
  const value = req.get("authorization") || "";
  return value.startsWith("Bearer ") ? value.slice(7) : "";
}

function publicPurchase(row) {
  return {
    email: row.customer_email,
    licenceKey: row.licence_key,
    releaseVersion: row.release_version,
    purchasedAt: row.purchase_timestamp,
    downloads: { macOS: true, Windows: true },
  };
}

export function createApp({ stripe, store, config, logger = noOpLogger }) {
  const app = express();
  app.disable("x-powered-by");
  if (config.trustProxyHops) app.set("trust proxy", config.trustProxyHops);
  app.use((_req, res, next) => {
    res.set({
      "X-Content-Type-Options": "nosniff",
      "X-Frame-Options": "DENY",
      "Referrer-Policy": "strict-origin-when-cross-origin",
      "Permissions-Policy": "camera=(), microphone=(), geolocation=()",
      "Content-Security-Policy": "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'self'; form-action 'self'",
    });
    if (config.production) res.set("Strict-Transport-Security", "max-age=31536000; includeSubDomains");
    next();
  });

  app.post("/webhook", express.raw({ type: "application/json", limit: "1mb" }), async (req, res) => {
    const signature = req.get("stripe-signature");
    if (!signature) return res.status(400).json({ error: "Missing Stripe signature." });
    let event;
    try {
      event = stripe.webhooks.constructEvent(req.body, signature, config.stripeWebhookSecret);
    } catch {
      logger.warn("webhook.signature_rejected");
      return res.status(400).json({ error: "Invalid Stripe signature." });
    }

    if (!SUBSCRIPTION_EVENTS.has(event.type)) {
      return res.json({ received: true });
    }

    try {
      if (event.type.startsWith("checkout.session.")) {
        const session = await stripe.checkout.sessions.retrieve(event.data.object.id, { expand: ["subscription"] });
        if (session.mode !== "subscription" || session.payment_status !== "paid") return res.json({ received: true, fulfilled: false });
        const subscriptionId = typeof session.subscription === "string" ? session.subscription : session.subscription?.id;
        if (!subscriptionId) return res.json({ received: true, fulfilled: false });
        const subscription = await stripe.subscriptions.retrieve(subscriptionId, { expand: ["items.data.price.product"] });
        if (subscription.status !== "active") return res.json({ received: true, fulfilled: false });
        const { priceId, productId } = subscriptionPrice(subscription);
        const paidThrough = subscriptionPeriodEnd(subscription);
        if (priceId !== config.stripePriceId || !productId || !paidThrough) return res.status(400).json({ error: "Checkout Session does not contain the configured NoBS PDF annual subscription." });
        const customerEmail = session.customer_details?.email || session.customer_email;
        if (!customerEmail) return res.status(400).json({ error: "Paid Checkout Session has no customer email." });
        const result = store.recordSubscription(event, {
          customerEmail, stripeCustomerId: typeof session.customer === "string" ? session.customer : session.customer?.id ?? null,
          checkoutSessionId: session.id, paymentIntentId: null, productId, priceId,
          purchaseTimestamp: new Date((session.created ?? Math.floor(Date.now() / 1000)) * 1000).toISOString(),
          releaseVersion: config.releaseVersion, subscriptionId: subscription.id,
          subscriptionStatus: subscription.status, currentPeriodEnd: paidThrough,
          cancelAtPeriodEnd: Boolean(subscription.cancel_at_period_end),
        });
        logger.info(result.duplicate ? "webhook.duplicate" : "subscription.fulfilled", { stripeEvent: safeReference(event.id), session: safeReference(session.id), release: config.releaseVersion });
        if (!result.duplicate) logger.info("licence.created", { licence: safeReference(result.purchase?.licence_key), release: config.releaseVersion });
        return res.json({ received: true, duplicate: result.duplicate });
      }

      let subscription;
      if (event.type.startsWith("customer.subscription.")) {
        subscription = await stripe.subscriptions.retrieve(event.data.object.id, { expand: ["items.data.price.product"] });
      } else {
        const object = event.data.object;
        let subscriptionId = typeof object.subscription === "string" ? object.subscription : object.subscription?.id;
        if (!subscriptionId && object.invoice) {
          const invoice = await stripe.invoices.retrieve(typeof object.invoice === "string" ? object.invoice : object.invoice.id);
          subscriptionId = typeof invoice.subscription === "string" ? invoice.subscription : invoice.subscription?.id;
        }
        if (!subscriptionId && event.type === "charge.refunded" && object.payment_intent) {
          const paymentIntent = await stripe.paymentIntents.retrieve(typeof object.payment_intent === "string" ? object.payment_intent : object.payment_intent.id);
          const sessionId = paymentIntent.payment_details?.order_reference;
          if (typeof sessionId === "string" && SESSION_ID.test(sessionId)) subscriptionId = store.findPurchaseBySession(sessionId)?.stripe_subscription_id;
        }
        if (!subscriptionId) return res.json({ received: true, updated: false });
        subscription = await stripe.subscriptions.retrieve(subscriptionId, { expand: ["items.data.price.product"] });
      }
      const { priceId } = subscriptionPrice(subscription);
      if (priceId !== config.stripePriceId) return res.status(400).json({ error: "Subscription does not contain the configured NoBS PDF Price." });
      const refunded = event.type === "charge.refunded" && event.data.object.refunded;
      const nonPayingStatus = ["incomplete", "incomplete_expired", "past_due", "unpaid"].includes(subscription.status);
      const result = store.updateSubscription(event, {
        subscriptionId: subscription.id,
        subscriptionStatus: refunded ? "revoked" : subscription.status,
        currentPeriodEnd: subscriptionPeriodEnd(subscription),
        cancelAtPeriodEnd: Boolean(subscription.cancel_at_period_end),
        paymentStatus: refunded ? "refunded" : nonPayingStatus ? "failed" : "paid",
      });
      logger.info(result.duplicate ? "webhook.duplicate" : "subscription.updated", { stripeEvent: safeReference(event.id), subscription: safeReference(subscription.id) });
      return res.json({ received: true, duplicate: result.duplicate, updated: result.updated });
    } catch {
      logger.error("webhook.fulfilment_failed", { stripeEvent: safeReference(event.id) });
      return res.status(500).json({ error: "Unable to fulfil purchase." });
    }
  });

  app.use(express.json({ limit: "32kb" }));

  app.get("/healthz", (_req, res) => {
    try {
      store.healthCheck();
      return res.json({ status: "ok" });
    } catch {
      return res.status(503).json({ status: "unavailable" });
    }
  });

  app.post("/api/license/activate", activationRateLimit(), (req, res) => {
    const licenceKey = normalizeLicenceKey(req.body?.license_key);
    const deviceIdentifier = String(req.body?.device_identifier || "");
    const appVersion = String(req.body?.app_version || "");
    const platform = String(req.body?.platform || "");
    if (!LICENCE_PATTERN.test(licenceKey) || !/^[a-f0-9-]{36}$/i.test(deviceIdentifier) || !/^\d+\.\d+\.\d+(?:[-+][\w.-]+)?$/.test(appVersion) || !clientPlatforms.has(platform)) {
      return res.status(400).json({ state: "INVALID", message: "The licence or device information is malformed." });
    }
    const result = store.activateLicence({ licenceKey, deviceIdentifier, appVersion, platform, limit: config.activationLimit });
    const licenceReference = safeReference(licenceKey);
    if (result.state === "ACTIVE") logger.info("licence.activated", { licence: licenceReference, activation: safeReference(result.activationId), platform });
    else logger.warn("licence.activation_rejected", { licence: licenceReference, state: result.state, platform });
    if (result.state === "INVALID") return res.status(404).json({ state: "INVALID", message: "This licence key is not valid." });
    if (result.state === "REVOKED") return res.status(403).json({ state: "REVOKED", message: "This licence has been revoked. Please contact support." });
    if (result.state === "EXPIRED") return res.status(403).json({ state: "EXPIRED", message: "This subscription has expired. Go online and renew to continue." });
    if (result.state === "ENTITLEMENT_MISMATCH") return res.status(403).json({ ...result, message: `This licence covers release ${result.releaseVersion}, not this major version.` });
    if (result.state === "ACTIVATION_LIMIT_REACHED") return res.status(409).json({ ...result, message: `This licence is already active on ${result.limit} devices. Deactivate another device to continue.` });
    return res.status(201).json({
      valid: true,
      state: "ACTIVE",
      license_id: `lic_${createHash("sha256").update(licenceKey).digest("hex").slice(0, 20)}`,
      activation_id: result.activationId,
      activation_token: result.activationToken,
      release_version: result.releaseVersion,
      platform: result.platform,
      entitlement_state: "ACTIVE",
      paid_through: paidThroughSeconds(result.paidThrough),
      cancel_at_period_end: result.cancelAtPeriodEnd,
    });
  });

  app.post("/api/license/verify", (req, res) => {
    const activationId = String(req.body?.activation_id || "");
    const result = store.verifyActivation(activationId, bearerToken(req));
    if (result.state === "REVOKED") return res.status(403).json({ valid: false, state: "REVOKED", message: "This licence has been revoked. Please contact support." });
    if (result.state === "EXPIRED") return res.status(403).json({ valid: false, state: "EXPIRED", message: "This subscription has expired. Renew it to continue." });
    if (result.state !== "ACTIVE") return res.status(401).json({ valid: false, state: "INVALID", message: "This activation is not valid." });
    return res.json({ valid: true, state: "ACTIVE", entitlement_state: "ACTIVE", activation_id: result.activationId, release_version: result.releaseVersion, platform: result.platform, paid_through: paidThroughSeconds(result.paidThrough), cancel_at_period_end: result.cancelAtPeriodEnd });
  });

  app.post("/api/license/deactivate", (req, res) => {
    const activationId = String(req.body?.activation_id || "");
    const result = store.deactivateActivation(activationId, bearerToken(req));
    if (result.state !== "NOT_ACTIVATED") {
      logger.warn("licence.deactivation_rejected", { activation: safeReference(activationId) });
      return res.status(401).json({ state: "INVALID", message: "This activation is not valid." });
    }
    logger.info("licence.deactivated", { activation: safeReference(activationId) });
    return res.json({ state: "NOT_ACTIVATED", deactivated: true });
  });

  app.get("/api/config", async (_req, res) => {
    try {
      const price = await stripe.prices.retrieve(config.stripePriceId, { expand: ["product"] });
      if (!isAnnualNoBsPrice(price, config.stripePriceId)) throw new Error("Configured Price is not the NoBS annual subscription.");
      const product = typeof price.product === "object" ? price.product : null;
      res.set("Cache-Control", "public, max-age=300").json({
        productName: product && !product.deleted ? product.name : "NoBS PDF",
        currency: price.currency,
        unitAmount: price.unit_amount,
      });
    } catch {
      res.status(503).json({ error: "Pricing is temporarily unavailable." });
    }
  });

  app.post("/api/checkout", async (_req, res) => {
    try {
      const price = await stripe.prices.retrieve(config.stripePriceId);
      if (!isAnnualNoBsPrice(price, config.stripePriceId)) throw new Error("Configured Price is not the NoBS annual subscription.");
      const session = await stripe.checkout.sessions.create({
        mode: "subscription",
        line_items: [{ price: config.stripePriceId, quantity: 1 }],
        automatic_tax: { enabled: true },
        success_url: `${config.appBaseUrl}/success?session_id={CHECKOUT_SESSION_ID}`,
        cancel_url: `${config.appBaseUrl}/?checkout=cancelled#pricing`,
        metadata: { release_version: config.releaseVersion },
      });
      if (!session.url) throw new Error("Stripe did not return a Checkout URL.");
      logger.info("checkout.created", { session: safeReference(session.id), release: config.releaseVersion });
      res.status(201).json({ url: session.url });
    } catch {
      logger.error("checkout.creation_failed");
      res.status(502).json({ error: "Checkout could not be started. Please try again." });
    }
  });

  app.post("/api/billing/portal", async (req, res) => {
    const sessionId = String(req.body?.session_id || "");
    if (!SESSION_ID.test(sessionId)) return res.status(400).json({ error: "Invalid Checkout Session reference." });
    const purchase = store.findPurchaseBySession(sessionId);
    if (!purchase?.stripe_customer_id) return res.status(404).json({ error: "Billing account not found." });
    try {
      const portal = await stripe.billingPortal.sessions.create({ customer: purchase.stripe_customer_id, return_url: `${config.appBaseUrl}/success?session_id=${encodeURIComponent(sessionId)}` });
      if (!portal.url) throw new Error("Stripe did not return a portal URL.");
      return res.status(201).json({ url: portal.url });
    } catch {
      logger.error("billing_portal.creation_failed", { session: safeReference(sessionId) });
      return res.status(502).json({ error: "Billing management is temporarily unavailable." });
    }
  });

  app.get("/api/purchases/:sessionId", (req, res) => {
    if (!SESSION_ID.test(req.params.sessionId)) return res.status(400).json({ error: "Invalid Checkout Session reference." });
    const purchase = store.findPurchaseBySession(req.params.sessionId);
    res.set("Cache-Control", "no-store");
    if (!purchase) return res.status(202).json({ status: "pending" });
    if (!store.findDownloadEntitlement(req.params.sessionId, config.releaseVersion)) return res.status(403).json({ error: "This subscription is not currently entitled to access." });
    return res.json({ status: "complete", purchase: publicPurchase(purchase) });
  });

  app.get("/api/download/:platform", (req, res) => {
    const sessionId = String(req.query.session_id || "");
    const platform = req.params.platform === "mac" ? "macOS" : req.params.platform === "windows" ? "Windows" : "";
    if (!platforms.has(platform) || !SESSION_ID.test(sessionId)) {
      logger.warn("download.denied", { reason: "malformed", platform: req.params.platform });
      return res.status(400).json({ error: "Invalid download request." });
    }
    if (!store.findDownloadEntitlement(sessionId, config.releaseVersion)) {
      logger.warn("download.denied", { reason: "not_entitled", session: safeReference(sessionId), platform });
      return res.status(403).json({ error: "A valid purchase and licence are required." });
    }
    const target = config.downloads[platform];
    if (!target) {
      logger.error("download.unavailable", { release: config.releaseVersion, platform });
      return res.status(503).json({ error: `${platform} download is temporarily unavailable. Please contact support.` });
    }
    logger.info("download.authorized", { session: safeReference(sessionId), release: config.releaseVersion, platform });
    return res.redirect(303, target);
  });

  app.use("/api", (_req, res) => res.status(404).json({ error: "API endpoint not found." }));

  app.use(express.static("dist", { index: false, maxAge: "1h" }));
  app.get("*splat", (_req, res) => res.sendFile("index.html", { root: "dist" }));
  app.use((error, _req, res, _next) => {
    logger.warn("request.rejected", { reason: error?.type === "entity.parse.failed" ? "malformed_json" : "invalid_request" });
    res.status(error?.type === "entity.parse.failed" ? 400 : 500).json({ error: error?.type === "entity.parse.failed" ? "The request body is malformed." : "The request could not be completed." });
  });
  return app;
}
