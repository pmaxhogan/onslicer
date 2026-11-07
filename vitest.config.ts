import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    include: ["src/**/*.test.ts"],
    // src/api.js reads window.__TAURI__ at import time; stub it before any test file loads.
    setupFiles: ["test/tauri-stub.ts"],
    coverage: {
      provider: "v8",
      include: ["src/**/*.js"],
      // json-summary feeds the coverage ratchet job in ci.yml.
      reporter: ["text", "json-summary", "cobertura"],
    },
  },
});
