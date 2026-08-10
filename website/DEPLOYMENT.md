# Production deployment

## Architecture

The production application is one long-running Node/Express process. It serves
the Vite output from `dist/`, owns the `/api/*` and `/webhook` routes, and uses a
local SQLite database in WAL mode. Run exactly one application instance and
mount the directory containing the database on durable storage.

The previous `nobspdf.com` deployment was a static Netlify site. Netlify's static
origin cannot run `server/index.js` or provide the persistent local filesystem
required by this SQLite design. Do not translate this server to Netlify
Functions: an ephemeral/serverless filesystem is incompatible with the current
database architecture.

The supplied `Dockerfile` packages the existing architecture without changing
it. Deploy that container to a service which provides:

- a continuously running Node-compatible container;
- one instance only;
- a persistent volume mounted at `/var/lib/nobspdf`;
- HTTPS and custom-domain routing to container port `4242`;
- a health check against `GET /healthz`;
- encrypted environment-variable/secret management.

Because Express serves both the website and API, the complete application can
and should be hosted together at `https://nobs-pdf.com`. After the container is
healthy, replace the current Netlify apex-domain DNS records with the exact DNS
records issued by the selected container host. Do not leave a Netlify SPA
fallback in front of `/api/*`.

## Production environment

Configure these values in the hosting provider, never in a committed `.env`:

```dotenv
NODE_ENV=production
STRIPE_SECRET_KEY=sk_live_...                 # Stripe live secret key
STRIPE_WEBHOOK_SECRET=whsec_...               # secret for the production webhook below
STRIPE_PRICE_ID=price_...                     # live GBP 25/year inclusive-tax recurring Price
APP_BASE_URL=https://nobs-pdf.com
HOST=0.0.0.0
PORT=4242                                     # use a provider-injected PORT if required
DATABASE_PATH=/var/lib/nobspdf/nobs-production.sqlite
NOBS_RELEASE_VERSION=1.0.0
LICENCE_ACTIVATION_LIMIT=2
TRUST_PROXY_MODE=cloudflare-railway           # constrained Railway + Cloudflare proxy trust
MACOS_RELEASE_ENABLED=false
WINDOWS_RELEASE_ENABLED=true
MACOS_DOWNLOAD_URL=
WINDOWS_DOWNLOAD_URL=https://downloads.nobs-pdf.com/1.0.0/ACTUAL_WINDOWS_FILENAME.exe
```

Production startup fails closed for test Stripe keys, non-HTTPS origins,
temporary/relative databases, the wrong release, no enabled platform, or a
missing/unversioned download URL for an enabled platform. Disabled platforms
do not require a URL and cannot expose a download entitlement or redirect.

## Database operations

Mount `/var/lib/nobspdf` as a persistent volume owned by the container's `node`
user. SQLite creates the main database plus `-wal` and `-shm` files in this
directory. Never run multiple application instances against this local volume.

Use SQLite's online backup API/CLI or a volume snapshot that consistently
captures the database and WAL state. Schedule backups, retain them outside the
application volume, and prove restoration to a separate environment before
launch. The server creates and migrates its schema on startup.

## Stripe production setup

1. Create or select the live NoBS PDF Product, set its tax code to
   `txcd_10202001`, then create a GBP 25/year, inclusive-tax recurring Price and set its `price_...`
   value as `STRIPE_PRICE_ID`.
2. Create a live webhook endpoint at `https://nobs-pdf.com/webhook`.
3. Subscribe to the event set documented in `SUBSCRIPTION_LICENSING.md`.
4. Store that endpoint's `whsec_...` value as `STRIPE_WEBHOOK_SECRET`.
5. Confirm signed live/test-mode events never share credentials or objects.
6. Complete a real purchase and retain only redacted Stripe event/session
   references as release evidence.

## Deployment sequence

1. Provision the single-instance container service and persistent volume.
2. Add every environment value above using the provider's secret manager.
3. Build the repository's `website/Dockerfile` with `website/` as build context.
4. Route the provider's HTTPS service to container port `4242` and configure
   `/healthz` as its health check.
5. Verify the provider URL returns JSON `{ "status": "ok" }` from `/healthz`.
6. Attach `nobs-pdf.com`, apply the provider-issued DNS records, and wait for its
   TLS certificate and DNS propagation.
7. Add the Stripe production webhook only after the domain reaches this Node
   service.
8. Verify `content-type: application/json` and the documented status/body for
   every licensing endpoint. An HTML response or redirect is a failed deploy.

## Edge security

The server does not emit CORS allow headers. This is intentional: the browser
site is same-origin and the native desktop HTTP client is not subject to browser
CORS. Keep arbitrary browser origins disallowed.

Activation has an in-memory per-IP limit of ten attempts per minute. It is valid
only while the service remains single-instance and `TRUST_PROXY_MODE` is set to
`cloudflare-railway`. That mode trusts Railway's immediate edge and only
Cloudflare-published proxy networks beyond it; a numeric hop count is unsafe
because the Railway hostname is an alternate, shorter path. Revalidate the
published Cloudflare ranges before release. Add provider edge limits for `/api/checkout`,
`/api/license/*`, `/api/purchases/*`, downloads, and `/webhook`; exempt or size
Stripe webhook limits carefully so legitimate retries are accepted. Do not add
another application instance without first moving rate-limit state to a shared
backend and migrating SQLite to managed PostgreSQL with equivalent constraints.
