import fs from "node:fs";
import path from "node:path";
import Database from "better-sqlite3";
import { createHash, randomBytes, randomUUID, timingSafeEqual } from "node:crypto";
import { generateLicenceKey } from "./license.js";
import { normalizeLicenceKey } from "../shared/license.js";

const nowIso = () => new Date().toISOString();

function entitlement(row, now = Date.now()) {
  if (!row) return "INVALID";
  if (row.licence_status === "revoked") return "REVOKED";
  const paidThrough = Date.parse(row.current_period_end || "");
  return Number.isFinite(paidThrough) && paidThrough > now ? "ACTIVE" : "EXPIRED";
}

export function createStore(filename) {
  if (filename !== ":memory:") fs.mkdirSync(path.dirname(filename), { recursive: true, mode: 0o700 });
  const db = new Database(filename);
  if (filename !== ":memory:") fs.chmodSync(filename, 0o600);
  db.pragma("journal_mode = WAL");
  db.pragma("synchronous = FULL");
  db.pragma("busy_timeout = 5000");
  db.pragma("foreign_keys = ON");
  db.exec(`
    CREATE TABLE IF NOT EXISTS processed_events (
      stripe_event_id TEXT PRIMARY KEY, event_type TEXT NOT NULL, processed_at TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS purchases (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      licence_key TEXT NOT NULL UNIQUE,
      customer_email TEXT NOT NULL,
      stripe_customer_id TEXT,
      stripe_checkout_session_id TEXT NOT NULL UNIQUE,
      stripe_payment_intent_id TEXT,
      stripe_product_id TEXT NOT NULL,
      stripe_price_id TEXT NOT NULL,
      purchase_timestamp TEXT NOT NULL,
      release_version TEXT NOT NULL,
      activation_status TEXT NOT NULL DEFAULT 'inactive',
      activation_count INTEGER NOT NULL DEFAULT 0,
      created_at TEXT NOT NULL,
      updated_at TEXT NOT NULL
    );
    CREATE INDEX IF NOT EXISTS purchases_email_idx ON purchases(customer_email);
  `);
  const columns = new Set(db.prepare("PRAGMA table_info(purchases)").all().map(column => column.name));
  const additions = [
    ["payment_status", "TEXT NOT NULL DEFAULT 'paid'"],
    ["licence_status", "TEXT NOT NULL DEFAULT 'active'"],
    ["revoked_at", "TEXT"],
    ["stripe_subscription_id", "TEXT"],
    ["subscription_status", "TEXT NOT NULL DEFAULT 'incomplete'"],
    ["current_period_end", "TEXT"],
    ["cancel_at_period_end", "INTEGER NOT NULL DEFAULT 0"],
  ];
  for (const [name, type] of additions) if (!columns.has(name)) db.exec(`ALTER TABLE purchases ADD COLUMN ${name} ${type}`);
  db.exec(`
    CREATE UNIQUE INDEX IF NOT EXISTS purchases_subscription_idx ON purchases(stripe_subscription_id) WHERE stripe_subscription_id IS NOT NULL;
    CREATE TABLE IF NOT EXISTS activations (
      activation_id TEXT PRIMARY KEY,
      purchase_id INTEGER NOT NULL REFERENCES purchases(id) ON DELETE CASCADE,
      device_identifier_hash TEXT NOT NULL,
      activation_token_hash TEXT NOT NULL,
      app_version TEXT NOT NULL,
      platform TEXT NOT NULL,
      activated_at TEXT NOT NULL,
      last_seen_at TEXT NOT NULL,
      deactivated_at TEXT,
      UNIQUE(purchase_id, device_identifier_hash)
    );
    CREATE INDEX IF NOT EXISTS activations_purchase_idx ON activations(purchase_id, deactivated_at);
  `);

  const findBySession = db.prepare("SELECT * FROM purchases WHERE stripe_checkout_session_id = ?");
  const findBySubscription = db.prepare("SELECT * FROM purchases WHERE stripe_subscription_id = ?");
  const eventSeen = db.prepare("SELECT 1 FROM processed_events WHERE stripe_event_id = ?");
  const insertEvent = db.prepare("INSERT INTO processed_events VALUES (?, ?, ?)");
  const recordSubscription = db.transaction((event, subscription) => {
    if (eventSeen.get(event.id)) return { duplicate: true, purchase: findBySubscription.get(subscription.subscriptionId) ?? null };
    const existing = findBySubscription.get(subscription.subscriptionId) || findBySession.get(subscription.checkoutSessionId);
    insertEvent.run(event.id, event.type, nowIso());
    if (existing) return { duplicate: true, purchase: existing };
    const now = nowIso();
    for (let attempt = 0; attempt < 5; attempt += 1) {
      try {
        db.prepare(`INSERT INTO purchases (
          licence_key, customer_email, stripe_customer_id, stripe_checkout_session_id,
          stripe_payment_intent_id, stripe_product_id, stripe_price_id, purchase_timestamp,
          release_version, activation_status, activation_count, payment_status, licence_status,
          stripe_subscription_id, subscription_status, current_period_end, cancel_at_period_end,
          created_at, updated_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'inactive', 0, 'paid', 'active', ?, ?, ?, ?, ?, ?)`)
          .run(generateLicenceKey(), subscription.customerEmail, subscription.stripeCustomerId,
            subscription.checkoutSessionId, subscription.paymentIntentId, subscription.productId,
            subscription.priceId, subscription.purchaseTimestamp, subscription.releaseVersion,
            subscription.subscriptionId, subscription.subscriptionStatus, subscription.currentPeriodEnd,
            subscription.cancelAtPeriodEnd ? 1 : 0, now, now);
        return { duplicate: false, purchase: findBySubscription.get(subscription.subscriptionId) };
      } catch (error) {
        if (!String(error.message).includes("licence_key")) throw error;
      }
    }
    throw new Error("Unable to allocate a unique licence key.");
  });

  const updateSubscription = db.transaction((event, subscription) => {
    if (eventSeen.get(event.id)) return { duplicate: true, updated: false };
    insertEvent.run(event.id, event.type, nowIso());
    const result = db.prepare(`UPDATE purchases SET subscription_status = ?, current_period_end = CASE WHEN ? = 'failed' THEN current_period_end ELSE ? END,
      cancel_at_period_end = ?, payment_status = ?, licence_status = CASE WHEN ? = 'refunded' THEN 'revoked' ELSE licence_status END,
      revoked_at = CASE WHEN ? = 'refunded' THEN ? ELSE revoked_at END, updated_at = ? WHERE stripe_subscription_id = ?`)
      .run(subscription.subscriptionStatus, subscription.paymentStatus || "paid", subscription.currentPeriodEnd,
        subscription.cancelAtPeriodEnd ? 1 : 0, subscription.paymentStatus || "paid",
        subscription.paymentStatus || "paid", subscription.paymentStatus || "paid", nowIso(), nowIso(), subscription.subscriptionId);
    return { duplicate: false, updated: result.changes > 0 };
  });

  const hash = value => createHash("sha256").update(value).digest("hex");
  const activeCount = db.prepare("SELECT COUNT(*) count FROM activations WHERE purchase_id = ? AND deactivated_at IS NULL");
  const findLicence = db.prepare("SELECT * FROM purchases WHERE licence_key = ?");
  const findActivation = db.prepare(`SELECT a.*, p.licence_key, p.licence_status, p.release_version,
    p.subscription_status, p.current_period_end, p.cancel_at_period_end
    FROM activations a JOIN purchases p ON p.id = a.purchase_id WHERE a.activation_id = ?`);

  const activate = db.transaction(({ licenceKey, deviceIdentifier, appVersion, platform, limit, now = Date.now() }) => {
    const purchase = findLicence.get(normalizeLicenceKey(licenceKey));
    const state = entitlement(purchase, now);
    if (state !== "ACTIVE") return { state };
    if (String(purchase.release_version).split(".")[0] !== String(appVersion).split(".")[0]) return { state: "ENTITLEMENT_MISMATCH", releaseVersion: purchase.release_version };
    const deviceHash = hash(deviceIdentifier);
    const existing = db.prepare("SELECT * FROM activations WHERE purchase_id = ? AND device_identifier_hash = ?").get(purchase.id, deviceHash);
    const count = activeCount.get(purchase.id).count;
    if ((!existing || existing.deactivated_at) && count >= limit) return { state: "ACTIVATION_LIMIT_REACHED", activeDevices: count, limit };
    const token = randomBytes(32).toString("base64url");
    const tokenHash = hash(token);
    const timestamp = nowIso();
    const activationId = existing?.activation_id || `act_${randomUUID()}`;
    if (existing) db.prepare(`UPDATE activations SET activation_token_hash=?, app_version=?, platform=?, activated_at=?, last_seen_at=?, deactivated_at=NULL WHERE activation_id=?`).run(tokenHash, appVersion, platform, timestamp, timestamp, activationId);
    else db.prepare(`INSERT INTO activations (activation_id,purchase_id,device_identifier_hash,activation_token_hash,app_version,platform,activated_at,last_seen_at) VALUES (?,?,?,?,?,?,?,?)`).run(activationId, purchase.id, deviceHash, tokenHash, appVersion, platform, timestamp, timestamp);
    const nextCount = activeCount.get(purchase.id).count;
    db.prepare("UPDATE purchases SET activation_status=?, activation_count=?, updated_at=? WHERE id=?").run(nextCount ? "active" : "inactive", nextCount, timestamp, purchase.id);
    return { state: "ACTIVE", activationId, activationToken: token, releaseVersion: purchase.release_version, platform, activeDevices: nextCount, limit, paidThrough: purchase.current_period_end, cancelAtPeriodEnd: Boolean(purchase.cancel_at_period_end) };
  });

  function authenticatedActivation(activationId, token) {
    const activation = findActivation.get(activationId);
    if (!activation || !token) return null;
    const actual = Buffer.from(hash(token), "hex");
    const expected = Buffer.from(activation.activation_token_hash, "hex");
    return actual.length === expected.length && timingSafeEqual(actual, expected) ? activation : null;
  }

  return {
    healthCheck() { db.prepare("SELECT 1").get(); },
    recordSubscription,
    updateSubscription,
    findPurchaseBySession(sessionId) { return findBySession.get(sessionId) ?? null; },
    findPurchaseBySubscription(subscriptionId) { return findBySubscription.get(subscriptionId) ?? null; },
    findDownloadEntitlement(sessionId, releaseVersion) {
      const row = db.prepare("SELECT * FROM purchases WHERE stripe_checkout_session_id=? AND release_version=?").get(sessionId, releaseVersion);
      return entitlement(row) === "ACTIVE" ? row : null;
    },
    activateLicence(input) { return activate(input); },
    verifyActivation(activationId, token, now = Date.now()) {
      const activation = authenticatedActivation(activationId, token);
      if (!activation || activation.deactivated_at) return { state: "INVALID" };
      const state = entitlement(activation, now);
      if (state !== "ACTIVE") return { state };
      db.prepare("UPDATE activations SET last_seen_at=? WHERE activation_id=?").run(nowIso(), activationId);
      return { state: "ACTIVE", activationId, releaseVersion: activation.release_version, platform: activation.platform, paidThrough: activation.current_period_end, cancelAtPeriodEnd: Boolean(activation.cancel_at_period_end) };
    },
    deactivateActivation(activationId, token) {
      const activation = authenticatedActivation(activationId, token);
      if (!activation || activation.deactivated_at) return { state: "INVALID" };
      const now = nowIso();
      db.prepare("UPDATE activations SET deactivated_at=?, last_seen_at=? WHERE activation_id=?").run(now, now, activationId);
      const count = activeCount.get(activation.purchase_id).count;
      db.prepare("UPDATE purchases SET activation_status=?, activation_count=?, updated_at=? WHERE id=?").run(count ? "active" : "inactive", count, now, activation.purchase_id);
      return { state: "NOT_ACTIVATED" };
    },
    setLicenceState(licenceKey, { licenceStatus = "active", currentPeriodEnd, subscriptionStatus = "active" } = {}) {
      db.prepare("UPDATE purchases SET licence_status=?, revoked_at=?, current_period_end=COALESCE(?,current_period_end), subscription_status=? WHERE licence_key=?")
        .run(licenceStatus, licenceStatus === "revoked" ? nowIso() : null, currentPeriodEnd ?? null, subscriptionStatus, normalizeLicenceKey(licenceKey));
    },
    activeActivationCount(licenceKey) { const row = findLicence.get(normalizeLicenceKey(licenceKey)); return row ? activeCount.get(row.id).count : 0; },
    processedEventCount(eventId) { return db.prepare("SELECT COUNT(*) count FROM processed_events WHERE stripe_event_id=?").get(eventId).count; },
    close() { db.close(); },
  };
}
