import path from "node:path";

const required = ["STRIPE_SECRET_KEY", "STRIPE_WEBHOOK_SECRET", "STRIPE_PRICE_ID", "APP_BASE_URL"];
const RELEASE_VERSION = "1.0.0";

export function loadConfig(env = process.env) {
  const missing = required.filter((name) => !env[name]?.trim());
  if (missing.length) {
    throw new Error(`Missing required environment variables: ${missing.join(", ")}. Copy .env.example to .env and configure Stripe test mode.`);
  }

  let appBaseUrl;
  try {
    appBaseUrl = new URL(env.APP_BASE_URL);
  } catch {
    throw new Error("APP_BASE_URL must be a valid absolute URL.");
  }
  if (!/^https?:$/.test(appBaseUrl.protocol)) throw new Error("APP_BASE_URL must use http or https.");

  const environment = env.NODE_ENV || "development";
  const production = environment === "production";
  const databasePath = path.resolve(env.DATABASE_PATH || "./data/nobs-development.sqlite");
  const releaseVersion = env.NOBS_RELEASE_VERSION || RELEASE_VERSION;
  const activationLimit = Number(env.LICENCE_ACTIVATION_LIMIT || 2);
  const port = Number(env.PORT || 4242);
  const trustProxyMode = env.TRUST_PROXY_MODE || (production ? "cloudflare-railway" : "none");
  const releaseFlag = (name) => {
    const value = env[name]?.trim().toLowerCase();
    if (value === "true") return true;
    if (value === "false" || (!production && value === undefined)) return false;
    throw new Error(`${name} must be explicitly set to true or false.`);
  };
  const releases = {
    macOS: releaseFlag("MACOS_RELEASE_ENABLED"),
    Windows: releaseFlag("WINDOWS_RELEASE_ENABLED"),
  };
  if (!env.STRIPE_WEBHOOK_SECRET.startsWith("whsec_")) throw new Error("STRIPE_WEBHOOK_SECRET must be a Stripe webhook signing secret.");
  if (!env.STRIPE_PRICE_ID.startsWith("price_")) throw new Error("STRIPE_PRICE_ID must be a Stripe Price ID.");
  if (!Number.isInteger(port) || port < 1 || port > 65535) throw new Error("PORT must be a valid TCP port.");
  if (!Number.isInteger(activationLimit) || activationLimit < 1 || activationLimit > 20) throw new Error("LICENCE_ACTIVATION_LIMIT must be an integer between 1 and 20.");
  if (!new Set(["none", "cloudflare-railway"]).has(trustProxyMode)) throw new Error("TRUST_PROXY_MODE must be none or cloudflare-railway.");
  if (!/^\d+\.\d+\.\d+$/.test(releaseVersion)) throw new Error("NOBS_RELEASE_VERSION must be a semantic version such as 1.0.0.");

  if (production) {
    const failures = [];
    if (!env.STRIPE_SECRET_KEY.startsWith("sk_live_")) failures.push("STRIPE_SECRET_KEY must be a live key");
    if (appBaseUrl.protocol !== "https:" || ["localhost", "127.0.0.1"].includes(appBaseUrl.hostname) || appBaseUrl.hostname.endsWith(".test")) failures.push("APP_BASE_URL must be a production HTTPS origin");
    if (!env.DATABASE_PATH || !path.isAbsolute(env.DATABASE_PATH) || databasePath.startsWith("/tmp/")) failures.push("DATABASE_PATH must be an explicit absolute persistent path outside /tmp");
    if (releaseVersion !== RELEASE_VERSION) failures.push(`NOBS_RELEASE_VERSION must be ${RELEASE_VERSION}`);
    if (!releases.macOS && !releases.Windows) failures.push("at least one platform release must be enabled");
    for (const [platform, name, value] of [["macOS", "MACOS_DOWNLOAD_URL", env.MACOS_DOWNLOAD_URL], ["Windows", "WINDOWS_DOWNLOAD_URL", env.WINDOWS_DOWNLOAD_URL]]) {
      if (!releases[platform]) continue;
      try {
        const url = new URL(value);
        if (url.protocol !== "https:" || ["localhost", "127.0.0.1"].includes(url.hostname) || !url.pathname.includes(`/${releaseVersion}/`)) throw new Error();
      } catch { failures.push(`${name} must be an HTTPS URL namespaced under /${releaseVersion}/`); }
    }
    if (failures.length) throw new Error(`Unsafe production configuration: ${failures.join("; ")}.`);
  } else if (!env.STRIPE_SECRET_KEY.startsWith("sk_test_")) {
    throw new Error("Development and test environments must use a Stripe test key.");
  }

  return {
    environment,
    production,
    stripeSecretKey: env.STRIPE_SECRET_KEY,
    stripeWebhookSecret: env.STRIPE_WEBHOOK_SECRET,
    stripePriceId: env.STRIPE_PRICE_ID,
    appBaseUrl: appBaseUrl.origin,
    host: env.HOST || "127.0.0.1",
    port,
    databasePath,
    releaseVersion,
    activationLimit,
    trustProxyMode,
    releases,
    downloads: {
      macOS: releases.macOS ? env.MACOS_DOWNLOAD_URL || "" : "",
      Windows: releases.Windows ? env.WINDOWS_DOWNLOAD_URL || "" : "",
    },
  };
}
