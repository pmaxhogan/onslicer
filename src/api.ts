// Typed wrappers around the Tauri commands. All Onshape calls, caches,
// exports and slicer launches happen in the Rust backend.
import { invoke } from "@tauri-apps/api/core";
import type {
  AuthInput,
  AuthSettings,
  CallStats,
  Doc,
  DocsResult,
  Element,
  ElementsResult,
  ExportCheck,
  ExportTarget,
  Part,
  PartsCache,
  PartsResult,
  Refresh,
  SaveResult,
  SlicerState,
} from "./types";

export const getDocs = (refresh: Refresh) => invoke<DocsResult>("get_docs", { refresh });

export const getElements = (doc: Doc, refresh: Refresh) =>
  invoke<ElementsResult>("get_elements", { doc, refresh });

// Disk-cached part list or null; never hits the API.
export const peekParts = (doc: Doc, elementId: string) =>
  invoke<PartsCache | null>("peek_parts", { docId: doc.id, elementId });

export const getParts = (doc: Doc, elementId: string, refresh = false) =>
  invoke<PartsResult>("get_parts", { doc, elementId, refresh });

export const saveExport = (target: ExportTarget) => invoke<SaveResult>("save_export", { target });

export const printExport = (target: ExportTarget) => invoke<SaveResult>("print_export", { target });

export const getCallStats = () => invoke<CallStats>("get_call_stats");

export const checkExports = () => invoke<ExportCheck>("check_exports");

export const getAuthSettings = () => invoke<AuthSettings>("get_auth_settings");

export const saveAuthSettings = (input: AuthInput) =>
  invoke<AuthSettings>("save_auth_settings", { input });

export const listSlicers = () => invoke<SlicerState>("list_slicers");
export const rescanSlicers = () => invoke<SlicerState>("rescan_slicers");
export const pickManualSlicer = () => invoke<SlicerState>("pick_manual_slicer");
export const selectSlicer = (path: string) => invoke<SlicerState>("select_slicer", { path });

// Thumbnails arrive as raw PNG bytes and become object URLs, memoized per
// item until its thumbnail href (and so the image) changes.
const thumbUrls = new Map<string, { href: string; url: string | null }>();

async function memoThumb(
  key: string,
  href: string | null | undefined,
  load: () => Promise<ArrayBuffer>,
): Promise<string | null> {
  if (!href) return null;
  const mem = thumbUrls.get(key);
  if (mem?.href === href) return mem.url;
  const bytes = await load();
  const url = bytes.byteLength
    ? URL.createObjectURL(new Blob([bytes], { type: "image/png" }))
    : null;
  const prev = thumbUrls.get(key);
  if (prev?.url && prev.url !== url) URL.revokeObjectURL(prev.url);
  thumbUrls.set(key, { href, url });
  return url;
}

export const getThumb = (doc: Doc) =>
  memoThumb(`doc:${doc.id}`, doc.thumbHref, () => invoke<ArrayBuffer>("get_thumb", { doc }));

// A tab's thumbnail (part null) or one part's.
export const getItemThumb = (doc: Doc, el: Element, part: Part | null) =>
  memoThumb(
    part ? `part:${doc.id}:${el.id}:${part.partId}` : `el:${doc.id}:${el.id}`,
    part ? part.thumbHref : el.thumbHref,
    () => invoke<ArrayBuffer>("get_item_thumb", { doc, el, part }),
  );
