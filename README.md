# onslicer

Browse your [Onshape](https://www.onshape.com/) models and send them straight to
[Bambu Studio](https://bambulab.com/en/download/studio) — no export → save → import dance.

- Document grid with thumbnails, filterable
- Print a whole part studio, a single part, or a composite part
- STL is exported live from Onshape on every print, so geometry is never stale
- Aggressive on-disk caching — a typical session uses 1–2 of the
  [2,500 annual API calls](https://onshape-public.github.io/docs/auth/limits/) on a free Onshape account
- Auto-updates from GitHub releases

## Setup

1. Grab the latest installer from [Releases](https://github.com/pmaxhogan/onslicer/releases/latest).
2. Create an API key in the [Onshape developer portal](https://cad.onshape.com/appstore/dev-portal)
   (read scopes are enough).
3. Paste the access key + secret into the app's settings (⚙). Keys are stored locally
   and only ever sent to onshape.com.

## Development

```sh
npm install
npm run dev      # tauri dev
npm run build    # tauri build
```

Built with [Tauri v2](https://v2.tauri.app/). Every push to `main` builds a release;
installed apps pick it up automatically.

## License

[MIT](LICENSE)
