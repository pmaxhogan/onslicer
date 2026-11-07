<!-- stack:begin (managed by the stack plugin; edit via /stack:remember or by hand, keep the markers) -->

## Stack

- mode: deps.dependabot-mechanism=workflow-run
- mode: test.coverage-ratchet=compare-job
- exception: ci.deploy-on-main - ci.yml is ci/node-ts plus a Windows rust job (fmt, clippy, cargo test) and the coverage compare job inline; build.yml releases on every push to main without waiting for ci. Reason: Tauri app with a Rust shell; workflow-run auto-merge keys on the single ci workflow. (2026-11-07)
- exception: lint.eslint-prettier - eslint.config.js adds browser globals for src/ and ignores Tauri/cache dirs. Reason: src/ is the Tauri webview frontend. (2026-11-07)
- exception: style.ascii-only - check-ascii.mjs also skips .icns. Reason: Tauri macOS icon binary. (2026-11-07)

<!-- stack:end -->
