# Production deployment

## Architecture

The production application is one long-running Node/Express process. It serves
the Vite output from `dist/`, owns the `/api/*` and `/webhook` routes, and uses a
local SQLite database in WAL mode. Run exactly one application instance and
mount the directory containing the database on durable storage.

The current `nobspdf.com` deployment is a static Netlify site. Netlify's static
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
and should be hosted together at `https://nobspdf.com`. After the container is
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
APP_BASE_URL=https://nobspdf.com
HOST=0.0.0.0
PORT=4242                                     # use a provider-injected PORT if required
DATABASE_PATH=/var/lib/nobspdf/nobs-production.sqlite
NOBS_RELEASE_VERSION=1.0.0
LICENCE_ACTIVATION_LIMIT=2
TRUST_PROXY_HOPS=1                            # change only to the documented proxy-hop count
MACOS_DOWNLOAD_URL=https://downloads.nobspdf.com/1.0.0/ACTUAL_MACOS_FILENAME.dmg
WINDOWS_DOWNLOAD_URL=https://downloads.nobspdf.com/1.0.0/ACTUAL_WINDOWS_FILENAME.exe
```

Production startup fails closed for test Stripe keys, non-HTTPS origins,
temporary/relative databases, the wrong release, or unversioned download URLs.

## Database operations

Mount `/var/lib/nobspdf` as a persistent volume owned by the container's `node`
user. SQLite creates the main database plus `-wal` and `-shm` files in this
directory. Never run multiple application instances against this local volume.

Use SQLite's online backup API/CLI or a volume snapshot that consistently
captures the database and WAL state. Schedule backups, retain them outside the
application volume, and prove restoration to a separate environment before
launch. The server creates and migrates its schema on startup.

## Stripe production setup

1. Create or select the live GBP 25/year, inclusive-tax recurring NoBS PDF Price and set its `price_...`
   value as `STRIPE_PRICE_ID`.
2. Create a live webhook endpoint at `https://nobspdf.com/webhook`.
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
6. Attach `nobspdf.com`, apply the provider-issued DNS records, and wait for its
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
only while the service remains single-instance and `TRUST_PROXY_HOPS` matches
the real proxy topology. Add provider edge limits for `/api/checkout`,
`/api/license/*`, `/api/purchases/*`, downloads, and `/webhook`; exempt or size
Stripe webhook limits carefully so legitimate retries are accepted. Do not add
another application instance without first moving rate-limit state to a shared
backend and migrating SQLite to managed PostgreSQL with equivalent constraints.
