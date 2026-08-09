import "dotenv/config";
import Stripe from "stripe";
import { createApp } from "./app.js";
import { loadConfig } from "./config.js";
import { createStore } from "./store.js";
import { createLogger } from "./logger.js";

try {
  const config = loadConfig();
  const stripe = new Stripe(config.stripeSecretKey);
  const store = createStore(config.databasePath);
  const logger = createLogger();
  const app = createApp({ stripe, store, config, logger });
  const server = app.listen(config.port, config.host, () => {
    logger.info("server.started", { environment: config.environment, host: config.host, port: config.port, release: config.releaseVersion });
  });
  const shutdown = () => server.close(() => { store.close(); process.exit(0); });
  process.on("SIGINT", shutdown);
  process.on("SIGTERM", shutdown);
} catch (error) {
  console.error(error instanceof Error ? error.message : "Unable to start server.");
  process.exit(1);
}
