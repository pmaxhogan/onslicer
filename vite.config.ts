import { defineConfig } from "vite";

// Tauri's standard Vite setup, with the page and its sources in src/. Tauri dev
// loads http://localhost:1420 (tauri.conf.json devUrl), so the port is fixed.
const host = process.env.TAURI_DEV_HOST;
// Set by the Tauri CLI for `tauri dev` and `tauri build --debug`.
const debug = process.env.TAURI_ENV_DEBUG === "true";

export default defineConfig({
  root: "src",
  clearScreen: false,
  envPrefix: ["VITE_", "TAURI_ENV_"],
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  build: {
    outDir: "../dist",
    emptyOutDir: true,
    // WebView2 on Windows is Chromium; 105 is Tauri's documented floor.
    target: "chrome105",
    minify: !debug,
    sourcemap: debug,
  },
});
