// onslicer data layer - Onshape REST + disk caches + slicer launch, all via
// Tauri plugins (withGlobalTauri, so everything hangs off window.__TAURI__).

const T = window.__TAURI__;
const fs = T.fs;
const BaseDirectory = fs.BaseDirectory;
const AppData = { baseDir: BaseDirectory.AppData };

const BASE = "https://cad.onshape.com";
// TODO: support other slicers or install directories?
const DEFAULT_BAMBU = "C:\\Program Files\\Bambu Studio\\bambu-studio.exe";

export let apiCallsThisSession = 0;

// ---------- settings ----------

let storePromise = null;
function store() {
  storePromise ??= T.store.load("settings.json", { autoSave: true });
  return storePromise;
}

export async function getSettings() {
  const s = await store();
  return {
    accessKey: (await s.get("accessKey")) ?? "",
    secretKey: (await s.get("secretKey")) ?? "",
    bambuPath: (await s.get("bambuPath")) ?? DEFAULT_BAMBU,
    selftest: (await s.get("selftest")) ?? false,
  };
}

export async function saveSettings({ accessKey, secretKey, bambuPath }) {
  const s = await store();
  await s.set("accessKey", accessKey.trim());
  await s.set("secretKey", secretKey.trim());
  await s.set("bambuPath", (bambuPath || DEFAULT_BAMBU).trim());
  await s.save();
}

export async function hasKeys() {
  const s = await getSettings();
  return Boolean(s.accessKey && s.secretKey);
}

// ---------- call accounting ----------

// our estimates can show roughly where the yearly allocation stands. Onshape's own count at
// https://cad.onshape.com/user/developer is authoritative.
export async function getCallStats() {
  await countChain;
  const s = await store();
  return {
    session: apiCallsThisSession,
    total: (await s.get("apiCallsTotal")) ?? 0,
    since: (await s.get("apiCallsSince")) ?? null,
  };
}

// Serialized so parallel fetches (thumbnails) don't lose increments.
let countChain = Promise.resolve();
function countCall() {
  apiCallsThisSession++;
  countChain = countChain
    .then(async () => {
      const s = await store();
      await s.set("apiCallsTotal", ((await s.get("apiCallsTotal")) ?? 0) + 1);
      if (!(await s.get("apiCallsSince"))) await s.set("apiCallsSince", new Date().toISOString());
    })
    .catch(() => {});
  return countChain;
}

// ---------- onshape fetch ----------

// Onshape redirects some endpoints (e.g. STL export) to a sibling
// *.onshape.com host; auto-follow would drop the Authorization header on the
// cross-origin hop, so follow redirects by hand and re-attach auth while the
// URL stays on onshape.com.
async function onshape(pathOrUrl, accept = "application/json;charset=UTF-8;qs=0.09") {
  const s = await getSettings();
  if (!s.accessKey || !s.secretKey)
    throw new Error("Onshape API keys not set \u2014 open Settings");
  const auth = "Basic " + btoa(`${s.accessKey}:${s.secretKey}`);
  let url = new URL(pathOrUrl, BASE);
  for (let hops = 0; hops < 5; hops++) {
    const headers = { Accept: accept };
    if (url.hostname.endsWith(".onshape.com")) headers.Authorization = auth;
    const res = await T.http.fetch(url.toString(), { method: "GET", headers, maxRedirections: 0 });
    if (headers.Authorization && res.status < 400) await countCall();
    if (res.status >= 300 && res.status < 400) {
      const loc = res.headers.get("location");
      if (!loc) throw new Error(`redirect without location (${res.status})`);
      url = new URL(loc, url);
      continue;
    }
    return res;
  }
  throw new Error("too many redirects: " + pathOrUrl);
}

// ---------- cache helpers ----------

async function initDirs() {
  for (const dir of ["cache", "cache/thumbs", "cache/elements", "cache/parts", "exports"]) {
    if (!(await fs.exists(dir, AppData))) await fs.mkdir(dir, { ...AppData, recursive: true });
  }
}
const ready = initDirs();

async function readJsonCache(rel) {
  try {
    return JSON.parse(await fs.readTextFile(rel, AppData));
  } catch {
    return null;
  }
}

async function writeJsonCache(rel, obj) {
  await fs.writeTextFile(rel, JSON.stringify(obj, null, 2), AppData);
}

async function removeMatching(dir, predicate) {
  try {
    for (const entry of await fs.readDir(dir, AppData)) {
      if (entry.isFile && predicate(entry.name)) {
        await fs.remove(`${dir}/${entry.name}`, AppData);
      }
    }
  } catch {
    /* dir may not exist yet. TODO: check the error rather than blindly trusting */
  }
}

export function sanitize(name) {
  // Windows rejects path segments ending in '.' or ' ', so trim those too.
  return (
    (name || "model")
      .replace(/[^\w.\- ]+/g, "_")
      .replace(/\s+/g, " ")
      .trim()
      .slice(0, 80)
      .replace(/[. ]+$/, "") || "model"
  );
}

// ---------- documents ----------

const MAX_DOCS = 200;

function docItem(d) {
  return {
    id: d.id,
    name: d.name,
    modifiedAt: d.modifiedAt,
    workspaceId: d.defaultWorkspace?.id,
    thumbHref:
      d.thumbnail?.sizes?.find((x) => x.size === "300x170")?.href ??
      d.thumbnail?.sizes?.[0]?.href ??
      null,
  };
}

// refresh: 'incremental' (default for the refresh button) walks the
// modifiedAt-desc listing only until it reaches a doc that is unchanged in the
// cache - everything after that is unchanged too - so a typical refresh is one
// call. It cannot see deletions; 'full' re-lists everything (1 call/20 docs).
export async function getDocs(refresh = false) {
  await ready;
  const cached = await readJsonCache("cache/docs.json");
  if (!refresh && cached) return { ...cached, fromCache: true, newCalls: 0 };

  const incremental = refresh !== "full" && Boolean(cached);
  const known = new Map((incremental ? cached.items : []).map((d) => [d.id, d.modifiedAt]));
  const fresh = [];
  let reachedUnchanged = false;
  let calls = 0;
  let next = "/api/v6/documents?filter=0&limit=20&sortColumn=modifiedAt&sortOrder=desc";
  while (next && fresh.length < MAX_DOCS && !reachedUnchanged) {
    const res = await onshape(next);
    calls++;
    if (!res.ok) throw new Error(`documents ${res.status}: ${(await res.text()).slice(0, 200)}`);
    const page = await res.json();
    for (const d of page.items ?? []) {
      if (known.get(d.id) === d.modifiedAt) {
        reachedUnchanged = true;
        break;
      }
      fresh.push(docItem(d));
    }
    next = page.next || null;
  }

  let items = fresh;
  if (incremental) {
    const freshIds = new Set(fresh.map((d) => d.id));
    items = [...fresh, ...cached.items.filter((d) => !freshIds.has(d.id))]
      .sort((a, b) => (b.modifiedAt ?? "").localeCompare(a.modifiedAt ?? ""))
      .slice(0, MAX_DOCS);
  }
  const data = { fetchedAt: new Date().toISOString(), items };
  await writeJsonCache("cache/docs.json", data);
  return { ...data, fromCache: false, newCalls: calls, changed: fresh.length };
}

// ---------- elements ----------

// 3MF blob tabs are frozen snapshots of an export - they silently go stale
// when the source part studio changes, so they are hidden entirely.
export function isStale3mfBlob(e) {
  return (
    e.elementType === "BLOB" && (e.dataType === "application/3mf" || /\.3mf$/i.test(e.name || ""))
  );
}

const PRINTABLE_BLOBS = { "model/stl": ".stl", "application/sla": ".stl" };

// refresh: true refetches the element list (1 call) and drops only the part
// lists of elements whose microversionId changed (or is unknown); 'full' drops
// every part list of this doc, as an escape hatch.
// Without an explicit refresh, a cached list is still refetched when the doc
// list says the doc changed since that list was cached, so after a top-level
// refresh, opening an edited doc shows new tabs without a doc-level refresh.
export function elementsStale(old, doc) {
  if (!doc.modifiedAt) return false;
  if (old.docModifiedAt) return old.docModifiedAt !== doc.modifiedAt;
  return Date.parse(doc.modifiedAt) > Date.parse(old.fetchedAt); // pre-upgrade cache
}

export async function getElements(doc, refresh = false) {
  await ready;
  const rel = `cache/elements/${doc.id}.json`;
  const old = await readJsonCache(rel);
  const fromOld = (extra) => ({
    ...old,
    items: old.items.filter((e) => !isStale3mfBlob(e)),
    fromCache: true,
    ...extra,
  });
  if (!refresh && old && !elementsStale(old, doc)) return fromOld();
  let els;
  try {
    const res = await onshape(`/api/v6/documents/d/${doc.id}/w/${doc.workspaceId}/elements`);
    if (!res.ok) throw new Error(`elements ${res.status}: ${(await res.text()).slice(0, 200)}`);
    els = await res.json();
  } catch (err) {
    // An automatic (staleness-driven) refetch must never hide a usable cache,
    // e.g. offline or with the API allocation exhausted.
    if (!refresh && old) return fromOld({ staleError: String(err?.message || err) });
    throw err;
  }
  const data = {
    fetchedAt: new Date().toISOString(),
    docModifiedAt: doc.modifiedAt ?? null,
    items: els.map((e) => ({
      id: e.id,
      name: e.name,
      elementType: e.elementType,
      dataType: e.dataType,
      microversionId: e.microversionId ?? null,
      printable:
        e.elementType === "PARTSTUDIO" ||
        (e.elementType === "BLOB" && (e.dataType in PRINTABLE_BLOBS || /\.stl$/i.test(e.name))),
    })),
  };
  const oldMv = new Map((old?.items ?? []).map((e) => [e.id, e.microversionId]));
  const newMv = new Map(data.items.map((e) => [e.id, e.microversionId]));
  await removeMatching("cache/parts", (name) => {
    if (!name.startsWith(`${doc.id}-`)) return false;
    if (refresh === "full") return true;
    const eid = name.slice(doc.id.length + 1).replace(/\.json$/, "");
    const mv = newMv.get(eid);
    return !mv || mv !== oldMv.get(eid);
  });
  await writeJsonCache(rel, data);
  return { ...data, items: data.items.filter((e) => !isStale3mfBlob(e)), fromCache: false };
}

// ---------- parts ----------

// Disk-cached part list or null. Never hits the API - the dialog filter uses
// this so typing can match part names without a network round-trip
export async function peekParts(doc, eid) {
  await ready;
  return readJsonCache(`cache/parts/${doc.id}-${eid}.json`);
}

export async function getParts(doc, eid, refresh = false) {
  await ready;
  const rel = `cache/parts/${doc.id}-${eid}.json`;
  let data = refresh ? null : await peekParts(doc, eid);
  const fromCache = Boolean(data);
  if (!data) {
    const res = await onshape(`/api/v6/parts/d/${doc.id}/w/${doc.workspaceId}/e/${eid}`);
    if (!res.ok) throw new Error(`parts ${res.status}: ${(await res.text()).slice(0, 200)}`);
    const parts = await res.json();
    data = {
      fetchedAt: new Date().toISOString(),
      items: parts.map((p) => ({
        partId: p.partId,
        name: p.name,
        bodyType: p.bodyType,
        isMesh: p.isMesh,
      })),
    };
    await writeJsonCache(rel, data);
  }
  return { ...data, fromCache };
}

// ---------- thumbnails ----------

// Thumbnails are cached on disk to avoid network round trip latency, as
// <docId>-<stamp>.png, where <stamp> is the thumbHref's ?t= value - Onshape
// bumps it when the thumbnail changes. A refresh therefore only downloads
// thumbnails of new or changed docs.
const thumbUrls = new Map(); // docId -> { key, url }

export function thumbKey(doc) {
  try {
    const t = new URL(doc.thumbHref).searchParams.get("t");
    if (t) return `${doc.id}-${t.replace(/\W/g, "")}`;
  } catch {
    /* fall through */
  }
  return `${doc.id}-0`;
}

export async function getThumb(doc) {
  await ready;
  if (!doc.thumbHref) return null;
  const key = thumbKey(doc);
  const mem = thumbUrls.get(doc.id);
  if (mem?.key === key) return mem.url;
  const rel = `cache/thumbs/${key}.png`;
  const legacy = `cache/thumbs/${doc.id}.png`; // pre-stamp cache layout
  let bytes;
  if (await fs.exists(rel, AppData)) {
    bytes = await fs.readFile(rel, AppData);
  } else if (await fs.exists(legacy, AppData)) {
    // Legacy files were written for the hrefs in the current docs.json, so
    // adopt them under the stamped name instead of re-downloading.
    await fs.rename(legacy, rel, {
      oldPathBaseDir: BaseDirectory.AppData,
      newPathBaseDir: BaseDirectory.AppData,
    });
    bytes = await fs.readFile(rel, AppData);
  } else {
    const res = await onshape(doc.thumbHref, "image/png");
    if (!res.ok) return null;
    bytes = new Uint8Array(await res.arrayBuffer());
    await removeMatching("cache/thumbs", (name) => name.startsWith(`${doc.id}-`));
    await fs.writeFile(rel, bytes, AppData);
  }
  const url = URL.createObjectURL(new Blob([bytes], { type: "image/png" }));
  if (mem) URL.revokeObjectURL(mem.url);
  thumbUrls.set(doc.id, { key, url });
  return url;
}

// ---------- print ----------

// Stable export path per (doc, element, part), named after the thing itself so
// slicer shows a meaningful object name. Because the path never changes,
// save() can overwrite it and "Reload from disk" picks up new geometry:
//   exports/<doc>/<tab>.stl            whole part studio or blob
//   exports/<doc>/<tab>/<part>.stl     single part
export function exportRel(doc, el, part, ext) {
  const dir = part
    ? `exports/${sanitize(doc.name)}/${sanitize(el.name)}`
    : `exports/${sanitize(doc.name)}`;
  const base = sanitize(part ? part.name : el.name).replace(/\.(stl|3mf)$/i, "");
  return { dir, rel: `${dir}/${base}${ext}` };
}

// Export from Onshape and write to the stable path. Does not launch anything.
export async function save({ doc, el, part = null }) {
  await ready;
  let bytes, ext;
  if (el.elementType === "PARTSTUDIO") {
    const partFilter = part ? `&partIds=${encodeURIComponent(part.partId)}` : "";
    const res = await onshape(
      `/api/v6/partstudios/d/${doc.id}/w/${doc.workspaceId}/e/${el.id}/stl?units=millimeter&mode=binary&grouping=true${partFilter}`,
      "*/*",
    );
    if (!res.ok)
      throw new Error(`STL export failed (${res.status}): ${(await res.text()).slice(0, 200)}`);
    bytes = new Uint8Array(await res.arrayBuffer());
    ext = ".stl";
  } else if (el.elementType === "BLOB") {
    const res = await onshape(
      `/api/v6/blobelements/d/${doc.id}/w/${doc.workspaceId}/e/${el.id}`,
      "*/*",
    );
    if (!res.ok) throw new Error(`blob download failed (${res.status})`);
    bytes = new Uint8Array(await res.arrayBuffer());
    ext =
      PRINTABLE_BLOBS[el.dataType] ??
      (/\.(stl|3mf)$/i.exec(el.name || "")?.[0].toLowerCase() || ".stl");
  } else {
    throw new Error(`unsupported element type ${el.elementType}`);
  }

  const { dir, rel } = exportRel(doc, el, part, ext);
  if (!(await fs.exists(dir, AppData))) await fs.mkdir(dir, { ...AppData, recursive: true });
  await fs.writeFile(rel, bytes, AppData);
  const file = await T.path.join(await T.path.appDataDir(), ...rel.split("/"));
  return { file, bytes: bytes.length };
}

// Export, then open the file in the slicer.
export async function print(target) {
  const res = await save(target);
  const s = await getSettings();
  await T.core.invoke("launch_slicer", { exe: s.bambuPath, file: res.file });
  return res;
}

// ---------- self-test (dev verification hook) ----------

export async function runSelfTest() {
  const out = { startedAt: new Date().toISOString(), steps: [] };
  const step = (name, ok, detail) => out.steps.push({ name, ok, detail });
  try {
    const docs = await getDocs(true);
    step("docs", docs.items.length > 0, `${docs.items.length} docs, fromCache=${docs.fromCache}`);
    const doc = docs.items.find((d) => d.name === "button holders") ?? docs.items[0];
    const thumb = await getThumb(doc);
    step("thumb", Boolean(thumb), thumb ? "blob url ok" : "no thumbnail");
    const els = await getElements(doc, true);
    step(
      "elements",
      els.items.length > 0,
      els.items.map((e) => `${e.name}:${e.elementType}`).join(", "),
    );
    step("no-3mf-blobs", !els.items.some((e) => /\.3mf$/i.test(e.name)), "stale 3mf blobs hidden");
    const ps = els.items.find((e) => e.elementType === "PARTSTUDIO");
    const parts = await getParts(doc, ps.id, true);
    step(
      "parts",
      parts.items.length > 0,
      parts.items.map((p) => `${p.name}(${p.bodyType})`).join(", "),
    );
    const solid = parts.items.find((p) => p.bodyType === "solid") ?? parts.items[0];
    const res = await save({ doc, el: ps, part: solid });
    step("export", res.bytes > 100, `${res.bytes} bytes -> ${res.file}`);

    // Cache invalidation: a plain elements refresh keeps the parts cache of an
    // unchanged part studio (same microversionId); a 'full' refresh drops it.
    const partsCache = `cache/parts/${doc.id}-${ps.id}.json`;
    const existedBefore = await fs.exists(partsCache, AppData);
    const refreshed = await getElements(doc, true);
    const keptAfterRefresh = await fs.exists(partsCache, AppData);
    const hasMv = Boolean(refreshed.items.find((e) => e.id === ps.id)?.microversionId);
    await getElements(doc, "full");
    const keptAfterFull = await fs.exists(partsCache, AppData);
    step(
      "cache-invalidation",
      existedBefore && hasMv && keptAfterRefresh && !keptAfterFull,
      `parts cache before=${existedBefore} mv=${hasMv} after-refresh=${keptAfterRefresh} after-full=${keptAfterFull}`,
    );

    // Blob print path (different endpoint + ext inference). 3MF blobs are
    // hidden from the UI, so reach one via the unfiltered raw element list.
    const rawEls = JSON.parse(
      await fs.readTextFile(`cache/elements/${doc.id}.json`, AppData),
    ).items;
    const blob = rawEls.find((e) => e.elementType === "BLOB" && /\.(stl|3mf)$/i.test(e.name || ""));
    if (blob) {
      const bres = await save({ doc, el: blob });
      step(
        "blob-export",
        bres.bytes > 100 && /\.(stl|3mf)$/.test(bres.file),
        `${bres.bytes} bytes -> ${bres.file}`,
      );
    } else {
      step("blob-export", true, "no blob element available to test (skipped)");
    }
    out.ok = out.steps.every((s) => s.ok);
  } catch (err) {
    out.ok = false;
    out.error = String(err?.message || err);
  }
  out.apiCalls = apiCallsThisSession;
  out.finishedAt = new Date().toISOString();
  await fs.writeTextFile("selftest-result.json", JSON.stringify(out, null, 2), AppData);
  return out;
}
