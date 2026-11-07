/// <reference types="vite/client" />

interface ImportMetaEnv {
  // "true" when the Tauri CLI builds a debug app (tauri dev, tauri build --debug).
  readonly TAURI_ENV_DEBUG?: string;
}
