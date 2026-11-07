// Pure UI helpers (no DOM, no Tauri), unit-tested in ui.test.ts.
import type { AuthInput, AuthMode, AuthSettings, Doc, Element, Slicer, SlicerState } from "./types";

export function fmtDate(iso: string | null | undefined): string {
  return iso
    ? new Date(iso).toLocaleDateString(undefined, {
        year: "numeric",
        month: "short",
        day: "numeric",
      })
    : "";
}

const ESCAPES: Record<string, string> = { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" };

export function esc(s: string | null | undefined): string {
  return (s ?? "").replace(/[&<>"]/g, (c) => ESCAPES[c] ?? c);
}

// "1 doc", "2 docs"
export function plural(n: number, word: string): string {
  return `${n} ${word}${n === 1 ? "" : "s"}`;
}

export function filterDocs(docs: Doc[], query: string): Doc[] {
  const q = query.trim().toLowerCase();
  return q ? docs.filter((d) => d.name.toLowerCase().includes(q)) : docs;
}

// The small type badge in front of an element row.
export function elementTypeLabel(el: Pick<Element, "elementType" | "name">): string {
  if (el.elementType === "PARTSTUDIO") return "part studio";
  if (el.elementType === "BLOB") return /\.\w+$/.exec(el.name)?.[0].slice(1) || "file";
  return el.elementType.toLowerCase();
}

// Toast after a refresh of the document list.
export function refreshSummary(changed: number | undefined, newCalls: number): string {
  const n = changed ?? 0;
  const what = n ? `${plural(n, "new/changed doc")}` : "No changes";
  return `${what} \u00b7 ${plural(newCalls, "API call")}`;
}

// Identifies one Save button's target; matches model.rs export_key.
export function exportKey(docId: string, elementId: string, partId: string | null): string {
  return `${docId}/${elementId}/${partId ?? ""}`;
}

// The last segment of a Windows or POSIX path.
export function fileName(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}

// Whether the selected sign-in mode has what it needs to make API calls.
export function authConfigured(s: AuthSettings): boolean {
  switch (s.mode) {
    case "apiKey":
      return s.hasApiKeys;
    case "password":
      return Boolean(s.username) && s.hasPassword;
    case "session":
      return s.hasSession;
  }
}

export interface AuthForm {
  mode: AuthMode;
  accessKey: string;
  secretKey: string;
  username: string;
  password: string;
  session: string;
}

// A save from the settings dialog sends only what the user filled in, so empty
// secret fields keep the stored values. The username is not secret and is
// prefilled, so it is sent only when it changed.
export function buildAuthInput(form: AuthForm, current: AuthSettings | null): AuthInput {
  const input: AuthInput = { mode: form.mode };
  const put = (key: "accessKey" | "secretKey" | "password" | "session", value: string) => {
    const v = value.trim();
    if (v) input[key] = v;
  };
  if (form.mode === "apiKey") {
    put("accessKey", form.accessKey);
    put("secretKey", form.secretKey);
  } else if (form.mode === "password") {
    const username = form.username.trim();
    if (username !== (current?.username ?? "")) input.username = username;
    put("password", form.password);
  } else {
    put("session", form.session);
  }
  return input;
}

// Placeholder of a secret input: blank when nothing is stored.
export function secretPlaceholder(stored: boolean, fallback: string): string {
  return stored ? "stored - leave blank to keep" : fallback;
}

export function selectedSlicer(state: SlicerState | null): Slicer | null {
  if (!state?.selected) return null;
  return state.slicers.find((s) => s.path === state.selected) ?? null;
}

// The name to show for the slicer that Print opens.
export function slicerName(state: SlicerState | null): string {
  return selectedSlicer(state)?.name ?? "your slicer";
}

export function slicerOptionLabel(s: Slicer): string {
  return s.manual ? `${s.name} (added manually)` : s.name;
}
