import { createHash } from "node:crypto";

export function safeReference(value) {
  return value ? createHash("sha256").update(String(value)).digest("hex").slice(0, 12) : undefined;
}

export function createLogger(output = console) {
  function write(level, event, fields = {}) {
    output[level === "error" ? "error" : "log"](JSON.stringify({ timestamp: new Date().toISOString(), level, event, ...fields }));
  }
  return {
    info(event, fields) { write("info", event, fields); },
    warn(event, fields) { write("warn", event, fields); },
    error(event, fields) { write("error", event, fields); },
  };
}

export const noOpLogger = { info() {}, warn() {}, error() {} };
