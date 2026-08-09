# NoBS PDF 1.0.0 release

Current recommendation: **NOT READY FOR RELEASE**. The source is release-hardened, but external production dependencies, legal documents, signed platform artifacts, and a real Stripe end-to-end test are not available in this workspace.

## Release checklist

- [ ] Stripe live product configured
- [ ] Stripe live price configured
- [ ] Stripe live secret configured
- [ ] Stripe production webhook configured
- [ ] Production HTTPS configured
- [ ] Production database configured
- [ ] Database backups configured
- [ ] Licensing API deployed
- [ ] Mac download uploaded
- [ ] Windows download uploaded
- [ ] macOS signing configured
- [ ] macOS notarisation completed
- [ ] Windows signing configured
- [ ] Production API URL configured
- [ ] Production Stripe values configured
- [ ] Terms published
- [ ] Privacy policy published
- [ ] Refund policy published
- [ ] Support email configured
- [x] Final version set to 1.0.0
- [ ] End-to-end purchase tested
- [ ] End-to-end activation tested
- [ ] Offline activation tested
- [ ] Deactivation tested
- [x] Production build tested
- [x] Final benchmark regression passed

Checked items indicate facts verifiable without external credentials. Do not mark manual items complete from automated tests alone.

## Audit summary

Audited the frozen Rust PDF engine, Tauri commands/configuration/capabilities, native credential storage, React desktop gate, web frontend, Express routes, Stripe session/webhook flow, SQLite schema and transactions, activation/deactivation, protected downloads, environment handling, package metadata, native resources, security headers, error messages, logging, tests, and distribution configuration.

Release-hardening changes include:

- consistent application release version `1.0.0`
- fail-closed production server configuration
- fail-closed release desktop API origin
- production HTTPS and versioned-download enforcement
- revoked/unpaid download denial
- SQLite `FULL` synchronization, WAL, busy timeout, directory creation, and restrictive file permissions
- structured operational logs containing only hashed Stripe/licence/activation references
- Tauri CSP and frozen JavaScript prototype
- hardened macOS runtime configuration
- platform-specific PDFium resource names
- checkout cancellation and service-unavailable customer messages
- explicit refund-policy link location

The PDF inspection, planning, optimisation, raster merge, flattening, export, validation, image handling, geometry, and benchmark implementation was not modified.

## Rust dependency security status

| Dependency | Advisory | Previous version | Patched version | Action and benchmark impact | Release decision |
| --- | --- | --- | --- | --- | --- |
| `time` | `RUSTSEC-2026-0009` | `0.3.36` | `0.3.47` | Updated the direct exact pin to the minimum patched release. All 35 normal Rust tests and all 9 golden checks pass. The benchmark is byte-for-byte unchanged at `61,002,045` → `11,835,505` bytes, with render error `5.817019354423868` and validation PASS. | Resolved. |
| `quick-xml` | `RUSTSEC-2026-0194` | `0.38.4` | `0.41.0` | Updated its parent `plist` from `1.8.0` to `1.10.0` through Tauri's existing `plist = "1"` constraint. No override or Tauri update was used; desktop licensing tests pass. This dependency is outside the PDF engine, so the golden output is unaffected. | Resolved. |
| `quick-xml` | `RUSTSEC-2026-0195` | `0.38.4` | `0.41.0` | Resolved by the same deliberate `plist 1.10.0` parent update. `plist` uses the plain XML reader rather than `NsReader`, but the vulnerable version was removed rather than suppressed. | Resolved. |
| `lopdf` | `RUSTSEC-2026-0187` | `0.36.0` | `>=0.42.0` | Not changed. `lopdf` is the frozen PDF engine's direct parser/rewriter dependency. The patched release adds a nesting limit, but the intervening releases also change parser, reader, writer, object/document, stream/object-stream, encryption, CMap/font/text, and save/load code. A safe upgrade therefore requires a separate compatibility branch and a broader malformed-PDF corpus in addition to the golden benchmark. | Unresolved HIGH release blocker. |

### lopdf compatibility and exposure decision

The vulnerable path is reachable: every locally selected PDF is untrusted input to `lopdf::Document::load` in NoBS parsing, rewriting, flattening, and validation paths. A maliciously deeply nested PDF can exhaust the application's process stack before optimisation completes. NoBS does not ingest remote PDFs automatically, so exploitation requires a user to select or open a crafted local file; that reduces exposure but does not remove it. Current validation occurs after parsing and therefore is not a mitigation for this parser vulnerability.

No superficial advisory suppression or pre-parse check has been added. Until the engine compatibility branch is complete, operational mitigation is to process only PDFs from trusted sources and restart the app after a parser crash. A real resolution requires upgrading to at least `lopdf 0.42.0`, compiling any API adaptations, testing malformed nesting at and around the new limit, comparing serialization/object ordering and streams, and passing the full normal, golden, geometry, annotation, text, vector, metadata, image, and render regression suite.

## Production blockers, in priority order

1. **Rust dependency audit still fails on the frozen engine.** The `time` and `quick-xml` advisories are resolved, but `lopdf 0.36.0` remains affected by high-severity `RUSTSEC-2026-0187` (deeply nested PDF object stack overflow; fixed in 0.42.0). It is reachable through local PDF processing and cannot be upgraded safely without a separate engine compatibility branch and full malformed-input and golden regression review.
2. **Windows PDFium binary is missing.** Supply the matching production `pdfium.dll` at `vendor/pdfium/bin/pdfium.dll` and validate it against the pinned PDFium version before a Windows build can work.
3. **Stripe live configuration is missing.** Live secret, live one-time Price ID, and production webhook signing secret are required.
4. **No deployed HTTPS API or durable production database is configured.** The service must run as one application instance with a persistent volume and backups.
5. **No production download artifacts or HTTPS destinations are configured.** Both URLs must be namespaced under `/1.0.0/`.
6. **macOS Developer ID/notarization credentials are missing.** The locally produced `.app` is ad-hoc/linker signed, has no Team ID, and fails strict resource signature verification; it must not be distributed.
7. **Windows Authenticode credentials are missing.** No signed Windows artifact has been produced.
8. **Terms, Privacy, and Refund pages are links only.** Actual reviewed policies must be published.
9. **Support address ownership is unverified.** Confirm that `support@nobspdf.com` is live and monitored.
10. **A real Stripe Test-mode BUY → DOWNLOAD → ACTIVATE → USE test has not been possible without account credentials and webhook forwarding.**
11. **Transactional purchase/licence email delivery is not configured.** The success page works, but customer access instructions also need a delivery provider before launch.

## What you need to provide

### Stripe and deployment

- `STRIPE_SECRET_KEY` (`sk_live_...` in production)
- `STRIPE_WEBHOOK_SECRET` for the production `/webhook` destination
- `STRIPE_PRICE_ID` for the live one-time NoBS PDF Price
- final public `APP_BASE_URL`
- absolute persistent `DATABASE_PATH` and backup destination/schedule
- production host/container configuration and HTTPS reverse proxy
- macOS and Windows HTTPS download URLs under `/1.0.0/`
- confirmation that `support@nobspdf.com` works
- email provider credentials and approved purchase/access email copy

### macOS signing and notarisation

- Apple Developer membership
- Developer ID Application certificate in Keychain, or `APPLE_CERTIFICATE` plus `APPLE_CERTIFICATE_PASSWORD` in CI
- `APPLE_SIGNING_IDENTITY` if it cannot be inferred
- either App Store Connect values `APPLE_API_ISSUER`, `APPLE_API_KEY`, and `APPLE_API_KEY_PATH`, or Apple ID notarisation values `APPLE_ID`, app-specific `APPLE_PASSWORD`, and `APPLE_TEAM_ID`

Tauri documents these official signing and notarisation variables in its [macOS signing guide](https://v2.tauri.app/distribute/sign/macos/).

### Windows signing

- Windows signing certificate/provider
- certificate thumbprint and trusted timestamp URL, or an approved custom/Azure signing command
- Windows build machine with the Windows SDK signing tools
- matching `pdfium.dll`

Tauri's official Windows guide uses `certificateThumbprint`, SHA-256 `digestAlgorithm`, and `timestampUrl` in the Windows bundle configuration. Add the real values only when provided: <https://v2.tauri.app/distribute/sign/windows/>.

### Legal

- reviewed Terms of Sale/Licence
- Privacy Policy
- Refund Policy
- support and company/contact details required in your jurisdiction

## Environment and deployment

Use `website/.env.example` as the variable inventory. Store production values in the deployment secret manager, never in `.env` committed to source.

Production startup intentionally rejects test keys, HTTP/localhost origins, relative or temporary database paths, missing/unversioned downloads, and a release other than `1.0.0`.

Run the Node server as a **single instance** while it uses SQLite and in-memory rate limiting. Mount the directory containing `DATABASE_PATH` on durable storage. Back up the SQLite database using a SQLite-aware online backup or a filesystem snapshot that captures the database together with WAL state; periodically restore a backup in a separate environment to prove recovery.

## Build commands

### Website/API

```bash
cd "/Users/joeconway/NoBS PDF/website"
npm ci
npm test
npm run build
NODE_ENV=production npm start
```

The final command requires every production variable from `.env.example` to be supplied by the runtime environment.

### macOS release

Run on macOS after the production API and Apple credentials exist:

```bash
cd "/Users/joeconway/NoBS PDF/desktop"
npm ci
NOBS_LICENSE_API_URL=https://YOUR_PRODUCTION_DOMAIN npm run tauri:build -- --bundles app,dmg
```

Verify:

```bash
codesign --verify --deep --strict --verbose=2 "src-tauri/target/release/bundle/macos/NoBS PDF.app"
spctl --assess --type execute --verbose=4 "src-tauri/target/release/bundle/macos/NoBS PDF.app"
xcrun stapler validate "src-tauri/target/release/bundle/dmg/NoBS PDF_1.0.0_*.dmg"
```

Do not distribute an ad-hoc-signed artifact. Tauri enables hardened runtime in `tauri.macos.conf.json`; actual signing and notarisation remain credential-dependent.

### Windows release

Run on Windows after placing the matching DLL at `vendor/pdfium/bin/pdfium.dll` and adding real certificate configuration to `tauri.windows.conf.json`:

```powershell
cd "C:\path\to\NoBS PDF\desktop"
npm ci
$env:NOBS_LICENSE_API_URL = "https://YOUR_PRODUCTION_DOMAIN"
npm run tauri:build -- --bundles nsis,msi
```

Verify signatures using the Windows SDK:

```powershell
signtool verify /pa /all /v "src-tauri\target\release\bundle\nsis\NoBS PDF_1.0.0_x64-setup.exe"
signtool verify /pa /all /v "src-tauri\target\release\bundle\msi\NoBS PDF_1.0.0_x64_en-US.msi"
```

Cross-compiling a Windows installer from macOS is not accepted as release verification.

## Stripe Test-mode end-to-end test

The automated suite verifies session parameters, signature rejection/acceptance, idempotency, purchase/licence creation, success retrieval, downloads, activation limits, revocation, deactivation, and malformed input. A real account-backed test still requires your test credentials.

Follow `website/STRIPE.md` and `website/LICENSE.md`, then record evidence for each step:

1. Start the website/API using Stripe test values and persistent test DB.
2. Forward `checkout.session.completed` and `checkout.session.async_payment_succeeded` using Stripe CLI.
3. Buy with Stripe's documented test card.
4. Confirm a verified webhook creates exactly one purchase/licence.
5. Confirm `/success?session_id=...` displays the licence and both downloads authorize.
6. Build/run the desktop with the test licensing API origin and activate that key.
7. Optimise the golden/test PDF.
8. Stop the API, reopen the desktop, and confirm offline use.
9. Restart the API, deactivate, and confirm the slot is free.
10. Reactivate and resend the webhook to confirm no duplicate licence.

Retain redacted logs and Stripe Dashboard event/Session IDs as release evidence. Never retain or paste secrets, activation tokens, or full licence keys into logs.

## Final verification

Before release, all commands below must pass from clean dependency installs:

```bash
cd "/Users/joeconway/NoBS PDF"
cargo test
cargo test --release --test benchmark -- --ignored --test-threads=1

cd desktop
npm ci
npm run build
cd src-tauri
cargo test

cd ../../website
npm ci
npm test
npm run build
npm audit
```

The frozen golden result must remain:

- Input: `61,002,045` bytes
- Output: `11,835,505` bytes
- Reduction: `80.6%`

Rust dependency audit is mandatory. It currently reports only the unresolved `lopdf` vulnerability described in blocker 1 (plus non-vulnerability maintenance warnings in target-specific desktop dependencies):

```bash
cd "/Users/joeconway/NoBS PDF"
cargo audit
cd desktop/src-tauri
cargo audit
```
