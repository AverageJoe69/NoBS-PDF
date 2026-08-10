# NoBS PDF licensing and desktop activation

NoBS PDF uses a deliberately small activation system. Stripe remains responsible for subscription/payment state; the NoBS server projects paid-through entitlement, revocation, and activation limits. The desktop app stores an opaque activation credential and paid-through timestamp in the operating system credential store and works offline through the paid period plus a 30-day grace. See `SUBSCRIPTION_LICENSING.md`.

## Flow

```text
Paid Stripe Checkout
→ verified webhook creates purchase/licence
→ desktop submits licence + app-scoped device UUID
→ server validates purchase, status, entitlement, and available slots
→ server stores hashed device ID and hashed activation token
→ desktop stores the returned credential in Keychain/Credential Manager
→ normal app unlocks and remains available offline
```

The desktop never contains Stripe keys, database credentials, webhook secrets, or a private signing key.

## Configuration

Server variables are documented in `.env.example`. Licensing adds:

```dotenv
LICENCE_ACTIVATION_LIMIT=2
```

The default and intended policy is two active devices per licence. The server is authoritative; the desktop does not contain this number.

The desktop API origin is compiled into the Tauri binary. Production defaults to `https://nobs-pdf.com`. Override it for local development before compiling/running Tauri:

```bash
NOBS_LICENSE_API_URL=http://127.0.0.1:4242 npm run tauri:dev
```

Use an HTTPS production origin. Do not compile server secrets into the desktop application.

## Public API

### `POST /api/license/activate`

```json
{
  "license_key": "NOBS-AB12-CD34-EF56-7890",
  "device_identifier": "application-generated UUID",
  "app_version": "1.0.0",
  "platform": "macos"
}
```

The endpoint normalises and validates the key, limits JSON requests to 32 KB, applies an in-memory per-IP activation rate limit, verifies the paid subscription period and licence status, enforces major-release entitlement, and enforces the active-device limit. A successful response contains only licence/activation identifiers, an opaque activation token, entitled release/platform, and the paid-through timestamp. It contains no email or Stripe payment data.

### `POST /api/license/verify`

Uses `Authorization: Bearer <activation token>` with:

```json
{ "activation_id": "act_..." }
```

The server compares the token hash in constant time, verifies the activation and subscription entitlement, and updates `last_seen_at`. It returns `ACTIVE`, `EXPIRED`, `INVALID`, or `REVOKED`.

### `POST /api/license/deactivate`

Uses the same bearer credential and activation ID. Successful deactivation timestamps the device activation, reduces the active count, and immediately frees a slot.

## Database schema

Existing `purchases` rows are migrated in place with:

- `payment_status` — defaults to `paid` because purchases are created only by a verified paid webhook
- `licence_status` — `active` or `revoked`
- `revoked_at`
- `stripe_subscription_id`
- `subscription_status`
- `current_period_end`
- `cancel_at_period_end`

The new `activations` table stores:

- activation ID
- purchase/licence foreign key
- SHA-256 hash of the app-scoped device UUID
- SHA-256 hash of the opaque activation token
- app version and platform
- activation time
- last-seen time
- optional deactivation time

The unique `(purchase_id, device_identifier_hash)` constraint makes repeat activation from the same device idempotent. A repeated activation rotates its credential rather than consuming another slot.

## Desktop credential storage

The Rust `keyring` crate uses native OS facilities:

- macOS: Keychain Services
- Windows: Windows Credential Manager

Two entries under `com.nobspdf.desktop` are stored:

- a randomly generated, application-scoped device UUID
- the licence key, activation ID/token, release entitlement, platform, paid-through timestamp, and local status

The app never reads hardware serial numbers and does not fingerprint the machine. The server stores only a hash of the app-generated UUID.

## Offline and revocation behavior

On startup, the app checks the native credential store. A locally active credential unlocks the normal application immediately. Verification then runs in the background:

- success keeps the activation active and updates `last_seen_at`
- network failure leaves the installed application usable
- an explicit server `REVOKED` or `EXPIRED` response is stored locally and shows the activation screen with an explanation

PDF inspection, estimation, and optimisation commands also check for an active local credential, preventing a bypass of the React screen. They do not perform network requests and no optimisation code was changed.

The desktop performs a cheap local due-check after launch and once per hour while
it remains open. A network verification is attempted only when the last
successful verification is approximately 30 days old (with deterministic
per-activation jitter of up to 24 hours), or when a scheduled retry is due.
Transient failures retry after approximately one day, then three days, then
weekly. During a paid term they never downgrade a locally active subscription
licence. Only the expected authenticated `401 INVALID`, `403 REVOKED`, or `403 EXPIRED`
protocol response changes local activation state. It does not require an
internet connection per PDF or per optimisation.

## Local development and manual test

1. Configure and start the website/backend in Stripe Test mode as described in `STRIPE.md`.
2. Run Stripe CLI forwarding and purchase NoBS PDF with a Stripe test card.
3. Confirm the verified webhook creates the licence and obtain its key from the success page or local test database.
4. In another terminal, start the desktop against the local API:

   ```bash
   cd "/Users/joeconway/NoBS PDF/desktop"
   NOBS_LICENSE_API_URL=http://127.0.0.1:4242 npm run tauri:dev
   ```

5. Enter the generated `NOBS-XXXX-XXXX-XXXX-XXXX` key and activate.
6. Close NoBS PDF.
7. Stop the website/backend or disconnect the network.
8. Reopen NoBS PDF and confirm the normal application remains unlocked.
9. Optimise a test PDF and confirm processing is unchanged.
10. Reconnect/start the backend and reopen the app to verify activation status.
11. Open **Licence** in the app header and deactivate this device.
12. Confirm the server activation count decreases and the activation screen returns.
13. Activate a second device identifier, then a replacement after deactivation.
14. Attempt three simultaneously active device identifiers and confirm the third receives `ACTIVATION_LIMIT_REACHED`.

Automated verification:

```bash
cd "/Users/joeconway/NoBS PDF/website"
npm test

cd "/Users/joeconway/NoBS PDF/desktop/src-tauri"
cargo test

cd "/Users/joeconway/NoBS PDF"
cargo test
```

## Production requirements

- Deploy the API and persistent database behind HTTPS.
- Compile release desktop builds with the correct `NOBS_LICENSE_API_URL`.
- Replace the in-memory rate limiter with a shared reverse-proxy or Redis-backed limiter before running multiple server instances.
- Add an administrative, authenticated revocation/support tool; there is intentionally no public revoke endpoint.
- Define support procedures for lost devices and activation resets.
- Back up the purchases and activations database.
- Code-sign and notarize macOS releases and sign Windows installers.
- Test Keychain and Credential Manager behavior in signed release builds and OS upgrade/reinstall scenarios.
