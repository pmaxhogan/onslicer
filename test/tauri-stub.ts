// Minimal stand-in for the Tauri globals (withGlobalTauri) that src/api.js touches
// at import time. Tests of the pure helpers never reach the network or the disk.
const noop = async () => undefined;

const tauri = {
  fs: {
    BaseDirectory: { AppData: 14 },
    exists: async () => true,
    mkdir: noop,
  },
  store: { load: async () => ({ get: noop, set: noop, save: noop }) },
  http: { fetch: noop },
};

(globalThis as unknown as { window: unknown }).window = { __TAURI__: tauri };
