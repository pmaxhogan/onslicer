// JSON shapes of the document-browser Tauri commands, matching
// src-tauri/src/model.rs. The auth and slicer shapes live in contract-types.ts.

export type { AuthInput, AuthMode, AuthSettings, Slicer, SlicerState } from "./contract-types";

// "none": use the cache when usable; "normal": refetch, keeping what provably
// did not change; "full": refetch everything and drop dependent caches.
export type Refresh = "none" | "normal" | "full";

export interface Doc {
  id: string;
  name: string;
  modifiedAt: string | null;
  workspaceId: string | null;
  thumbHref: string | null;
}

export interface Element {
  id: string;
  name: string;
  elementType: string;
  dataType: string | null;
  microversionId: string | null;
  printable: boolean;
  thumbHref: string | null;
}

export interface Part {
  // For an assembly instance: the instance's id (see `source`).
  partId: string;
  name: string;
  bodyType: string;
  isMesh: boolean;
  thumbHref: string | null;
  // Set on the instances listed under an assembly.
  source?: ItemSource;
}

// The part an assembly instance shows, in the configuration it shows it in.
export interface ItemSource {
  documentId: string;
  wvm: "w" | "v" | "m";
  wvmId: string;
  elementId: string;
  partId: string;
  configuration: string;
  // The configuration values that tell this instance apart from the other
  // instances of the same part, e.g. "motor_depth 60 mm"; often "".
  configLabel: string;
}

// invoke("get_docs", { refresh })
export interface DocsResult {
  fetchedAt: string;
  items: Doc[];
  fromCache: boolean;
  newCalls: number;
  changed?: number;
}

// invoke("get_elements", { doc, refresh })
export interface ElementsResult {
  fetchedAt: string;
  docModifiedAt: string | null;
  withThumbs: boolean;
  items: Element[];
  fromCache: boolean;
  staleError?: string;
}

// invoke("peek_parts", { docId, elementId }) returns PartsCache | null
export interface PartsCache {
  fetchedAt: string;
  withThumbs: boolean;
  items: Part[];
}

// invoke("get_parts", { doc, elementId, refresh })
export interface PartsResult extends PartsCache {
  fromCache: boolean;
}

// invoke("save_export" | "print_export", { target })
export interface ExportTarget {
  doc: Doc;
  el: Element;
  part: Part | null;
}

export interface SaveResult {
  file: string;
  bytes: number;
}

// invoke("check_exports"): keys (see ui.ts exportKey) of exported targets
// whose tab changed in Onshape since their last save; `docs` is the document
// list fetched to find out, absent when nothing has been exported.
export interface ExportCheck {
  stale: string[];
  docs?: DocsResult;
}

// invoke("get_call_stats")
export interface CallStats {
  session: number;
  total: number;
  since: string | null;
}
