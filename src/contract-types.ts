// CONTRACT FILE: JSON shapes of the auth and slicer Tauri commands, matching
// src-tauri/src/auth.rs and src-tauri/src/slicers.rs. Keep the two in sync.

export type AuthMode = "apiKey" | "password" | "session";

// invoke("get_auth_settings") and invoke("save_auth_settings", { input })
export interface AuthSettings {
  mode: AuthMode;
  hasApiKeys: boolean;
  username: string;
  hasPassword: boolean;
  hasSession: boolean;
}

// For each optional field: omitted/null keeps the stored value, "" deletes it.
export interface AuthInput {
  mode: AuthMode;
  accessKey?: string | null;
  secretKey?: string | null;
  username?: string | null;
  password?: string | null;
  session?: string | null;
}

export interface Slicer {
  name: string;
  path: string;
  manual: boolean;
}

// invoke("list_slicers"), invoke("rescan_slicers"), invoke("pick_manual_slicer"),
// invoke("select_slicer", { path })
export interface SlicerState {
  slicers: Slicer[];
  selected: string | null;
}
