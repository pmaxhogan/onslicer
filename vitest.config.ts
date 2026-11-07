import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    include: ["src/**/*.test.ts"],
    coverage: {
      provider: "v8",
      include: ["src/**/*.ts"],
      // main.ts is DOM wiring for the webview and api.ts thin invoke() wrappers;
      // both need a running Tauri app, not a unit test. The rest is types only.
      exclude: [
        "src/**/*.test.ts",
        "src/main.ts",
        "src/api.ts",
        "src/types.ts",
        "src/contract-types.ts",
      ],
      // json-summary feeds the coverage ratchet job in ci.yml.
      reporter: ["text", "json-summary", "cobertura"],
    },
  },
});
