# NoBS PDF desktop

The desktop app is a Tauri shell around the existing Rust optimisation engine. PDF processing remains local and the existing `pdfdoctor` CLI is unchanged.

## Run in development

```bash
cd desktop
npm install
NOBS_LICENSE_API_URL=http://127.0.0.1:4242 npm run tauri:dev
```

Drop `../tests/MT_AngelRaise_01.pdf` onto the window (or click the drop area), review the real 1080p estimate, click **Optimise PDF**, and choose a new output filename. The app will only show success after the rebuilt PDF passes the engine's validation checks.

## Build the macOS app

```bash
cd desktop
NOBS_LICENSE_API_URL=https://YOUR_PRODUCTION_DOMAIN npm run tauri:build
```

The application and installer are written beneath `src-tauri/target/release/bundle/`.

The 1080p profile is enabled. Other resolution profiles are deliberately shown as coming soon until corresponding structure-preserving engine policies are implemented and validated.
