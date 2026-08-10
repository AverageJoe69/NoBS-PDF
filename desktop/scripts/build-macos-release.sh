#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "macOS release bundles must be built on macOS." >&2
  exit 1
fi

if [[ -z "${NOBS_LICENSE_API_URL:-}" || "${NOBS_LICENSE_API_URL}" != https://* ]]; then
  echo "Set NOBS_LICENSE_API_URL to the deployed production HTTPS origin." >&2
  exit 1
fi

if [[ -z "${APPLE_SIGNING_IDENTITY:-}" || "${APPLE_SIGNING_IDENTITY}" == "-" ]]; then
  echo "Set APPLE_SIGNING_IDENTITY to a valid Developer ID Application identity; ad-hoc signing is not allowed." >&2
  exit 1
fi

if ! security find-identity -v -p codesigning | grep -Fq "\"${APPLE_SIGNING_IDENTITY}\""; then
  echo "APPLE_SIGNING_IDENTITY is not a valid identity in the active keychain." >&2
  exit 1
fi

has_api_credentials=false
if [[ -n "${APPLE_API_ISSUER:-}" && -n "${APPLE_API_KEY:-}" && -n "${APPLE_API_KEY_PATH:-}" ]]; then
  has_api_credentials=true
fi

has_apple_id_credentials=false
if [[ -n "${APPLE_ID:-}" && -n "${APPLE_PASSWORD:-}" && -n "${APPLE_TEAM_ID:-}" ]]; then
  has_apple_id_credentials=true
fi

if [[ "${has_api_credentials}" != true && "${has_apple_id_credentials}" != true ]]; then
  echo "Provide either APPLE_API_ISSUER/APPLE_API_KEY/APPLE_API_KEY_PATH or APPLE_ID/APPLE_PASSWORD/APPLE_TEAM_ID for notarisation." >&2
  exit 1
fi

if [[ ! -f ../vendor/pdfium/lib/libpdfium.dylib ]]; then
  echo "Missing Apple Silicon PDFium library at vendor/pdfium/lib/libpdfium.dylib." >&2
  exit 1
fi

npm run tauri:build -- --bundles app,dmg
