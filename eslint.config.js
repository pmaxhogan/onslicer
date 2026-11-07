// stack-template: dotfiles/eslint-config@1 6bf377526f92
import js from "@eslint/js";
import prettier from "eslint-config-prettier";
import globals from "globals";
import tseslint from "typescript-eslint";

/**
 * Flat config. Type-aware linting is deliberately off: `tsc --noEmit` in CI already
 * covers what the typed rules would add, without the slow parser setup.
 *
 * The em dash / en dash ban is enforced by scripts/check-ascii.mjs rather than a lint
 * rule, because it also has to cover Markdown, YAML and shell scripts, which ESLint
 * never parses.
 */
export default tseslint.config(
  {
    ignores: [
      "**/dist/**",
      "**/coverage/**",
      "**/node_modules/**",
      "src-tauri/target/**",
      "src-tauri/gen/**",
      "cache/**",
      "exports/**",
    ],
  },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    languageOptions: {
      ecmaVersion: 2023,
      sourceType: "module",
      globals: { ...globals.node },
    },
    rules: {
      "@typescript-eslint/no-unused-vars": [
        "error",
        {
          argsIgnorePattern: "^_",
          varsIgnorePattern: "^_",
          caughtErrorsIgnorePattern: "^_",
        },
      ],
      eqeqeq: ["error", "smart"],
      "no-implicit-coercion": "error",
      "object-shorthand": "error",
      "prefer-const": "error",
    },
  },
  // onslicer: src/ is the Tauri webview frontend, so it runs with browser globals.
  {
    files: ["src/**/*.js"],
    languageOptions: { globals: { ...globals.browser } },
  },
  // Must stay last: turns off every rule that would fight Prettier.
  prettier,
);
