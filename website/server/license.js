import { randomBytes } from "node:crypto";

export function generateLicenceKey(random = randomBytes) {
  const value = random(8).toString("hex").toUpperCase();
  return `NOBS-${value.slice(0, 4)}-${value.slice(4, 8)}-${value.slice(8, 12)}-${value.slice(12, 16)}`;
}
