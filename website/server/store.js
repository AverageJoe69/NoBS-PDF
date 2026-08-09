import fs from "node:fs";
import path from "node:path";
import Database from "better-sqlite3";
import { createHash, randomBytes, randomUUID, timingSafeEqual } from "node:crypto";
import { generateLicenceKey } from "./license.js";
import { normalizeLicenceKey } from "../shared/license.js";

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
      stripe_event_id TEXT PRIMARY KEY,
      event_type TEXT NOT NULL,
      processed_at TEXT NOT NULL
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
  const purchaseColumns = new Set(db.prepare("PRAGMA table_info(purchases)").all().map(column => column.name));
  if (!purchaseColumns.has("payment_status")) db.exec("ALTER TABLE purchases ADD COLUMN payment_status TEXT NOT NULL DEFAULT 'paid'");
  if (!purchaseColumns.has("licence_status")) db.exec("ALTER TABLE purchases ADD COLUMN licence_status TEXT NOT NULL DEFAULT 'active'");
  if (!purchaseColumns.has("revoked_at")) db.exec("ALTER TABLE purchases ADD COLUMN revoked_at TEXT");
  db.exec(`
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
  const insertEvent = db.prepare("INSERT INTO processed_events (stripe_event_id, event_type, processed_at) VALUES (?, ?, ?)");
  const insertPurchase = db.prepare(`
    INSERT INTO purchases (
      licence_key, customer_email, stripe_customer_id, stripe_checkout_session_id,
      stripe_payment_intent_id, stripe_product_id, stripe_price_id, purchase_timestamp,
      release_version, activation_status, activation_count, created_at, updated_at
    ) VALUES (@licenceKey, @customerEmail, @stripeCustomerId, @checkoutSessionId,
      @paymentIntentId, @productId, @priceId, @purchaseTimestamp,
      @releaseVersion, 'inactive', 0, @createdAt, @updatedAt)
  `);

  const record = db.transaction((event, purchase) => {
    const existing = findBySession.get(purchase.checkoutSessionId);
    const processed = db.prepare("SELECT 1 FROM processed_events WHERE stripe_event_id = ?").get(event.id);
    if (processed || existing) return { duplicate: true, purchase: existing ?? null };
    const now = new Date().toISOString();
    insertEvent.run(event.id, event.type, now);
    for (let attempt = 0; attempt < 5; attempt += 1) {
      const licenceKey = generateLicenceKey();
      try {
        insertPurchase.run({ ...purchase, licenceKey, createdAt: now, updatedAt: now });
        return { duplicate: false, purchase: findBySession.get(purchase.checkoutSessionId) };
      } catch (error) {
        if (!String(error.message).includes("licence_key")) throw error;
      }
    }
    throw new Error("Unable to allocate a unique licence key.");
  });

  const hash = value => createHash("sha256").update(value).digest("hex");
  const activeCount = db.prepare("SELECT COUNT(*) count FROM activations WHERE purchase_id = ? AND deactivated_at IS NULL");
  const findLicence = db.prepare("SELECT * FROM purchases WHERE licence_key = ?");
  const findActivation = db.prepare(`SELECT a.*, p.licence_key, p.licence_status, p.payment_status, p.release_version
    FROM activations a JOIN purchases p ON p.id = a.purchase_id WHERE a.activation_id = ?`);

  const activate = db.transaction(({ licenceKey, deviceIdentifier, appVersion, platform, limit }) => {
    const purchase = findLicence.get(normalizeLicenceKey(licenceKey));
    if (!purchase || purchase.payment_status !== "paid") return { state: "INVALID" };
    if (purchase.licence_status === "revoked") return { state: "REVOKED" };
    if (purchase.licence_status !== "active") return { state: "INVALID" };
    if (String(purchase.release_version).split(".")[0] !== String(appVersion).split(".")[0]) {
      return { state: "ENTITLEMENT_MISMATCH", releaseVersion: purchase.release_version };
    }
    const deviceHash = hash(deviceIdentifier);
    const existing = db.prepare("SELECT * FROM activations WHERE purchase_id = ? AND device_identifier_hash = ?").get(purchase.id, deviceHash);
    const count = activeCount.get(purchase.id).count;
    if ((!existing || existing.deactivated_at) && count >= limit) return { state: "ACTIVATION_LIMIT_REACHED", activeDevices: count, limit };
    const token = randomBytes(32).toString("base64url");
    const tokenHash = hash(token);
    const now = new Date().toISOString();
    let activationId;
    if (existing) {
      activationId = existing.activation_id;
      db.prepare(`UPDATE activations SET activation_token_hash = ?, app_version = ?, platform = ?,
        activated_at = ?, last_seen_at = ?, deactivated_at = NULL WHERE activation_id = ?`)
        .run(tokenHash, appVersion, platform, now, now, activationId);
    } else {
      activationId = `act_${randomUUID()}`;
      db.prepare(`INSERT INTO activations (activation_id, purchase_id, device_identifier_hash,
        activation_token_hash, app_version, platform, activated_at, last_seen_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?)`)
        .run(activationId, purchase.id, deviceHash, tokenHash, appVersion, platform, now, now);
    }
    const nextCount = activeCount.get(purchase.id).count;
    db.prepare("UPDATE purchases SET activation_status = ?, activation_count = ?, updated_at = ? WHERE id = ?")
      .run(nextCount ? "active" : "inactive", nextCount, now, purchase.id);
    return { state: "ACTIVE", activationId, activationToken: token, releaseVersion: purchase.release_version, platform, activeDevices: nextCount, limit };
  });

  function authenticatedActivation(activationId, token) {
    const activation = findActivation.get(activationId);
    if (!activation || !token) return null;
    const actual = Buffer.from(hash(token), "hex");
    const expected = Buffer.from(activation.activation_token_hash, "hex");
    return actual.length === expected.length && timingSafeEqual(actual, expected) ? activation : null;
  }

  return {
    recordPurchase: record,
    findPurchaseBySession(sessionId) { return findBySession.get(sessionId) ?? null; },
    findDownloadEntitlement(sessionId, releaseVersion) {
      return db.prepare(`SELECT id, release_version, payment_status, licence_status FROM purchases
        WHERE stripe_checkout_session_id = ? AND release_version = ? AND payment_status = 'paid' AND licence_status = 'active'`)
        .get(sessionId, releaseVersion) ?? null;
    },
    activateLicence(input) { return activate(input); },
    verifyActivation(activationId, token) {
      const activation = authenticatedActivation(activationId, token);
      if (!activation || activation.deactivated_at || activation.payment_status !== "paid") return { state: "INVALID" };
      if (activation.licence_status === "revoked") return { state: "REVOKED" };
      const now = new Date().toISOString();
      db.prepare("UPDATE activations SET last_seen_at = ? WHERE activation_id = ?").run(now, activationId);
      return { state: "ACTIVE", activationId, releaseVersion: activation.release_version, platform: activation.platform };
    },
    deactivateActivation(activationId, token) {
      const activation = authenticatedActivation(activationId, token);
      if (!activation || activation.deactivated_at) return { state: "INVALID" };
      const now = new Date().toISOString();
      db.prepare("UPDATE activations SET deactivated_at = ?, last_seen_at = ? WHERE activation_id = ?").run(now, now, activationId);
      const count = activeCount.get(activation.purchase_id).count;
      db.prepare("UPDATE purchases SET activation_status = ?, activation_count = ?, updated_at = ? WHERE id = ?")
        .run(count ? "active" : "inactive", count, now, activation.purchase_id);
      return { state: "NOT_ACTIVATED" };
    },
    setLicenceState(licenceKey, { paymentStatus = "paid", licenceStatus = "active" } = {}) {
      db.prepare("UPDATE purchases SET payment_status = ?, licence_status = ?, revoked_at = ? WHERE licence_key = ?")
        .run(paymentStatus, licenceStatus, licenceStatus === "revoked" ? new Date().toISOString() : null, normalizeLicenceKey(licenceKey));
    },
    activeActivationCount(licenceKey) {
      const purchase = findLicence.get(normalizeLicenceKey(licenceKey));
      return purchase ? activeCount.get(purchase.id).count : 0;
    },
    processedEventCount(eventId) { return db.prepare("SELECT COUNT(*) count FROM processed_events WHERE stripe_event_id = ?").get(eventId).count; },
    close() { db.close(); },
  };
}
