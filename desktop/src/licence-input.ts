export const LICENCE_PREFIX = "NOBS-";

export function normaliseLicenceBody(value: string): string {
  const withoutWhitespace = value.replace(/\s/g, "");
  const withoutPrefix = withoutWhitespace.replace(/^nobs-/i, "");
  return withoutPrefix.replace(/-/g, "").toUpperCase();
}

export function formatLicenceBody(value: string): string {
  const body = normaliseLicenceBody(value);
  return body.match(/.{1,4}/g)?.join("-") ?? "";
}

export function canonicalLicenceKey(value: string): string | null {
  const body = normaliseLicenceBody(value);
  if (!/^[A-Z0-9]{16}$/.test(body)) return null;
  return `${LICENCE_PREFIX}${body.match(/.{4}/g)!.join("-")}`;
}
