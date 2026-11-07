# onslicer

Browse your [Onshape](https://www.onshape.com/) models and send them straight to your slicer
([Bambu Studio](https://bambulab.com/en/download/studio) and other common slicers are detected
automatically) - no export -> save -> import dance.

- Document grid with thumbnails, filterable
- Print a whole part studio, a single part, or a composite part
- STL is exported live from Onshape on every print, so geometry is never stale
- Aggressive on-disk caching to minimize latency and avoid waiting on network round-trips
- Auto-updates from GitHub releases

## Setup

1. Grab the latest installer from [Releases](https://github.com/pmaxhogan/onslicer/releases/latest).
2. Create an API key in the [Onshape developer portal](https://cad.onshape.com/appstore/dev-portal)
   (read scopes are enough).
3. Paste the access key + secret into the app's settings (&#x2699;). Secrets are stored in the
   Windows Credential Manager and only ever sent to onshape.com.
4. Pick your slicer in the same dialog if the detected one is not the one you want, or add any
   executable with "Other...".

## TODO: use session from browser?

## Development

```sh
npm install
npm run dev        # tauri dev (starts the Vite dev server on :1420)
npm run build      # tauri build
npm run build:web  # typecheck + Vite build of the frontend into dist/
npm test           # vitest (UI helpers)
cd src-tauri && cargo test   # Rust unit tests (needs dist/, so build:web first)
```

The frontend (`src/`, TypeScript + Vite) is a thin UI over Tauri commands; all Onshape API
calls, the disk caches, exports and the slicer launch live in the Rust backend (`src-tauri/`).

End-to-end check against the real Onshape account (no window, no slicer): run the app with
`ONSLICER_SELFTEST=1`. It prints one JSON line, writes `selftest-result.json` to the app data
dir and exits 0 on success, 1 on failure.

Built with [Tauri v2](https://v2.tauri.app/). Every push to `main` builds a release;
installed apps pick it up automatically.

## License

[MIT](LICENSE)
