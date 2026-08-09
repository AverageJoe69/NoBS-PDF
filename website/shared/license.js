export const LICENCE_PATTERN = /^NOBS-[A-Z0-9]{4}(?:-[A-Z0-9]{4}){3}$/;

export function normalizeLicenceKey(value) {
  const compact = String(value ?? "").toUpperCase().replace(/[^A-Z0-9]/g, "");
  const body = compact.startsWith("NOBS") ? compact.slice(4) : compact;
  if (body.length !== 16) return "";
  return `NOBS-${body.match(/.{1,4}/g).join("-")}`;
}
