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
  partId: string;
  name: string;
  bodyType: string;
  isMesh: boolean;
  thumbHref: string | null;
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

// invoke("get_call_stats")
export interface CallStats {
  session: number;
  total: number;
  since: string | null;
}
