# onslicer

Tauri 2 desktop app (Windows, NSIS): browse Onshape documents, export STL, open it in a slicer.

## Commands

- `npm run dev` - tauri dev (Vite dev server on :1420)
- `npm run build:web` - typecheck + Vite build into `dist/` (Rust builds need `dist/` to exist)
- `npm test` / `npm run lint` / `npm run format:check` / `npm run check:ascii`
- `cd src-tauri && ~/.cargo/bin/cargo fmt|clippy|test` - use the rustup toolchain in
  `~/.cargo/bin`; the Chocolatey cargo on PATH has no rustfmt/clippy and mixing the two
  toolchains in one target dir needs a `cargo clean`
- Live end-to-end check: run the app with `ONSLICER_SELFTEST=1` (headless, prints one JSON
  line, exits 0/1, writes `selftest-result.json` to the app data dir)

## Layout

- `src/` - TypeScript UI (`main.ts` wiring, `ui.ts` pure helpers with tests, `api.ts` typed
  `invoke` wrappers, `types.ts` / `contract-types.ts` command JSON shapes)
- `src-tauri/src/` - Rust backend: `onshape.rs` HTTP client (manual redirects, auth only to
  https `*.onshape.com`, call counter), `auth.rs` sign-in modes + Credential Manager,
  `data.rs` disk caches and exports, `slicers.rs` detection/selection/launch,
  `commands.rs` Tauri commands, `selftest.rs`, `settings.rs`
- `.github/workflows/` - `ci.yml` (js, rust, coverage jobs on PRs and main), `build.yml`
  (release after green ci on main), Dependabot auto-merge

## Gotchas

- Every push to `main` releases after ci is green, and installed copies auto-update.
- Dev builds and the installed app share one app data dir (`%APPDATA%com.pmaxhogan.onslicer`:
  `settings.json`, `cache/`, `exports/`) and the same Credential Manager entries.
- The Credential Manager is not reachable from SSH / session-0 shells (error 1312). Run the app
  or the selftest from the desktop session.
- No settings or cache migration code: change the stored shape and move existing data by hand.
- Secrets: never in `settings.json`; keyring service `com.pmaxhogan.onslicer`, long values
  chunked (see `auth.rs`).

<!-- stack:begin (managed by the stack plugin; edit via /stack:remember or by hand, keep the markers) -->

## Stack

- mode: deps.dependabot-mechanism=workflow-run
- mode: test.coverage-ratchet=compare-job
- exception: lint.eslint-prettier - eslint.config.js adds browser globals for src/ and ignores Tauri/cache dirs. Reason: src/ is the Tauri webview frontend. (2026-11-07)
- exception: style.ascii-only - check-ascii.mjs also skips .icns. Reason: Tauri macOS icon binary. (2026-11-07)
- exception: ci.deploy-on-main - ci.yml is ci/node-ts plus a Windows rust job (fmt, clippy, cargo test) and the coverage compare job inline; build.yml releases via workflow_run after ci is green on main. Reason: Tauri app with a Rust shell; workflow-run auto-merge keys on the single ci workflow. (2026-11-07)
- exception: deps.dependabot-policy - dependabot.yml is dependabot/multi-ecosystem (npm + cargo in /src-tauri + actions, one weekly PR). Reason: Tauri app with a Rust manifest. (2026-11-07)

<!-- stack:end -->
