#!/usr/bin/env bash
set -euo pipefail

PDFIUM_SHA256='fbdec47c3f2eaa80705ed25cf8bed5ac420998ba0f3e786d4d297b6238749064'
TARGET='aarch64-apple-darwin'
DMG_NAME='NoBS-PDF-1.0.0-macOS-arm64.dmg'

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DESKTOP_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
REPO_DIR="$(cd "${DESKTOP_DIR}/.." && pwd)"
TARGET_DIR="${DESKTOP_DIR}/src-tauri/target/${TARGET}/release"
APP_PATH="${TARGET_DIR}/bundle/macos/NoBS PDF.app"
PDFIUM_PATH="${APP_PATH}/Contents/Resources/libpdfium.dylib"
BUILD_DMG="${TARGET_DIR}/bundle/dmg/${DMG_NAME}"
ARTIFACT_DIR="${REPO_DIR}/release-artifacts/macos"
PERSISTED_DMG="${ARTIFACT_DIR}/${DMG_NAME}"

fail() { echo "macOS release error: $*" >&2; exit 1; }

[[ "$(uname -s)" == Darwin ]] || fail "This pipeline must run on macOS."
[[ "$(uname -m)" == arm64 ]] || fail "This pipeline requires Apple Silicon."
[[ -z "$(git -C "${REPO_DIR}" status --porcelain)" ]] || fail "The git working tree must be clean."
BUILD_COMMIT="$(git -C "${REPO_DIR}" rev-parse HEAD)"
[[ "${NOBS_LICENSE_API_URL:-}" == https://* ]] || fail "Set NOBS_LICENSE_API_URL to the production HTTPS origin."
[[ "${APPLE_SIGNING_IDENTITY:-}" == 'Developer ID Application:'* ]] || fail "Set a Developer ID Application identity; ad-hoc signing is forbidden."
security find-identity -v -p codesigning | grep -Fq "\"${APPLE_SIGNING_IDENTITY}\"" || fail "The requested signing identity is not valid."
[[ -n "${APPLE_API_ISSUER:-}" && -n "${APPLE_API_KEY:-}" ]] || fail "Set App Store Connect API issuer and key IDs."
[[ -f "${APPLE_API_KEY_PATH:-}" ]] || fail "APPLE_API_KEY_PATH must reference the local .p8 file."

SOURCE_PDFIUM="${REPO_DIR}/vendor/pdfium/lib/libpdfium.dylib"
[[ -f "${SOURCE_PDFIUM}" ]] || fail "Missing pinned PDFium."
[[ "$(file -b "${SOURCE_PDFIUM}")" == *arm64* ]] || fail "Pinned PDFium is not arm64."
[[ "$(shasum -a 256 "${SOURCE_PDFIUM}" | awk '{print $1}')" == "${PDFIUM_SHA256}" ]] || fail "Pinned PDFium checksum mismatch."

cd "${DESKTOP_DIR}"
# Tauri must not notarise its intermediate app before nested PDFium is re-signed.
env -u APPLE_API_ISSUER -u APPLE_API_KEY -u APPLE_API_KEY_PATH \
  npm run tauri -- build --bundles app --config src-tauri/tauri.macos.conf.json --target "${TARGET}"

[[ -f "${PDFIUM_PATH}" ]] || fail "Final app is missing PDFium."
[[ "$(file -b "${PDFIUM_PATH}")" == *arm64* ]] || fail "Bundled PDFium is not arm64."
[[ "$(shasum -a 256 "${PDFIUM_PATH}" | awk '{print $1}')" == "${PDFIUM_SHA256}" ]] || fail "Bundled PDFium differs before signing."

codesign --force --options runtime --timestamp --sign "${APPLE_SIGNING_IDENTITY}" "${PDFIUM_PATH}"
codesign --verify --strict --verbose=4 "${PDFIUM_PATH}"
PDFIUM_SIGNATURE="$(codesign -dvvv --verbose=4 "${PDFIUM_PATH}" 2>&1)"
grep -Fq "Authority=${APPLE_SIGNING_IDENTITY}" <<<"${PDFIUM_SIGNATURE}" || fail "PDFium has the wrong authority."
grep -Fq 'Timestamp=' <<<"${PDFIUM_SIGNATURE}" || fail "PDFium has no secure timestamp."
grep -Fq 'flags=0x10000(runtime)' <<<"${PDFIUM_SIGNATURE}" || fail "PDFium lacks hardened runtime."
! grep -Fq 'Signature=adhoc' <<<"${PDFIUM_SIGNATURE}" || fail "PDFium remains ad-hoc signed."

codesign --force --options runtime --timestamp --sign "${APPLE_SIGNING_IDENTITY}" "${APP_PATH}"
codesign --verify --deep --strict --verbose=4 "${APP_PATH}"
codesign --verify --strict --verbose=4 "${PDFIUM_PATH}"
APP_SIGNATURE="$(codesign -dvvv --verbose=4 "${APP_PATH}" 2>&1)"
grep -Fq "Authority=${APPLE_SIGNING_IDENTITY}" <<<"${APP_SIGNATURE}" || fail "App has the wrong authority."
grep -Fq 'Timestamp=' <<<"${APP_SIGNATURE}" || fail "App has no secure timestamp."
grep -Fq 'flags=0x10000(runtime)' <<<"${APP_SIGNATURE}" || fail "App lacks hardened runtime."

mkdir -p "$(dirname "${BUILD_DMG}")" "${ARTIFACT_DIR}"
STAGING_DIR="$(mktemp -d /tmp/nobs-macos-dmg.XXXXXX)"
ditto "${APP_PATH}" "${STAGING_DIR}/NoBS PDF.app"
ln -s /Applications "${STAGING_DIR}/Applications"
hdiutil create -ov -format UDZO -volname 'NoBS PDF' -srcfolder "${STAGING_DIR}" "${BUILD_DMG}"
find "${STAGING_DIR}" -depth -delete
codesign --force --timestamp --sign "${APPLE_SIGNING_IDENTITY}" "${BUILD_DMG}"
codesign --verify --strict --verbose=4 "${BUILD_DMG}"

# Persist first. This exact file—not the temporary build output—is submitted,
# stapled, tested, and eventually published.
ditto "${BUILD_DMG}" "${PERSISTED_DMG}"
PRE_SHA256="$(shasum -a 256 "${PERSISTED_DMG}" | awk '{print $1}')"
DMG_SIZE="$(stat -f '%z' "${PERSISTED_DMG}")"
printf '%s  %s\n' "${PRE_SHA256}" "${DMG_NAME}" > "${PERSISTED_DMG}.pre-notarisation.sha256"
printf '%s\n' "${BUILD_COMMIT}" > "${PERSISTED_DMG}.commit"

NOTARY_JSON="$(xcrun notarytool submit "${PERSISTED_DMG}" --key "${APPLE_API_KEY_PATH}" --key-id "${APPLE_API_KEY}" --issuer "${APPLE_API_ISSUER}" --wait --output-format json)"
echo "${NOTARY_JSON}"
NOTARY_STATUS="$(node -e 'const v=JSON.parse(process.argv[1]);process.stdout.write(v.status||"")' "${NOTARY_JSON}")"
NOTARY_ID="$(node -e 'const v=JSON.parse(process.argv[1]);process.stdout.write(v.id||"")' "${NOTARY_JSON}")"
printf '%s\n' "${NOTARY_ID}" > "${PERSISTED_DMG}.notarisation-id"
if [[ "${NOTARY_STATUS}" != Accepted ]]; then
  xcrun notarytool log "${NOTARY_ID}" --key "${APPLE_API_KEY_PATH}" --key-id "${APPLE_API_KEY}" --issuer "${APPLE_API_ISSUER}" "${PERSISTED_DMG}.notarisation-log.json" || true
  fail "Apple notarisation returned ${NOTARY_STATUS:-unknown}."
fi

[[ "$(shasum -a 256 "${PERSISTED_DMG}" | awk '{print $1}')" == "${PRE_SHA256}" ]] || fail "Persisted DMG changed after submission."
xcrun stapler staple "${PERSISTED_DMG}"
xcrun stapler validate "${PERSISTED_DMG}"
POST_SHA256="$(shasum -a 256 "${PERSISTED_DMG}" | awk '{print $1}')"
printf '%s  %s\n' "${POST_SHA256}" "${DMG_NAME}" > "${PERSISTED_DMG}.sha256"

echo "Build commit: ${BUILD_COMMIT}"
echo "Persisted DMG: ${PERSISTED_DMG}"
echo "DMG bytes: ${DMG_SIZE}"
echo "Pre-notarisation SHA-256: ${PRE_SHA256}"
echo "Notarisation submission: ${NOTARY_ID} (${NOTARY_STATUS})"
echo "Post-stapling SHA-256: ${POST_SHA256}"
