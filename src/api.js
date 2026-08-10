// onslicer data layer — Onshape REST + disk caches + slicer launch, all via
// Tauri plugins (withGlobalTauri, so everything hangs off window.__TAURI__).
//
// Free Onshape accounts get 2,500 API calls/year, so every listing is cached
// in the app data dir and only refetched on an explicit refresh.

const T = window.__TAURI__;
const fs = T.fs;
const BaseDirectory = fs.BaseDirectory;
const AppData = { baseDir: BaseDirectory.AppData };

const BASE = 'https://cad.onshape.com';
const DEFAULT_BAMBU = 'C:\\Program Files\\Bambu Studio\\bambu-studio.exe';

export let apiCallsThisSession = 0;

// ---------- settings ----------

let storePromise = null;
function store() {
  storePromise ??= T.store.load('settings.json', { autoSave: true });
  return storePromise;
}

export async function getSettings() {
  const s = await store();
  return {
    accessKey: (await s.get('accessKey')) ?? '',
    secretKey: (await s.get('secretKey')) ?? '',
    bambuPath: (await s.get('bambuPath')) ?? DEFAULT_BAMBU,
    selftest: (await s.get('selftest')) ?? false,
  };
}

export async function saveSettings({ accessKey, secretKey, bambuPath }) {
  const s = await store();
  await s.set('accessKey', accessKey.trim());
  await s.set('secretKey', secretKey.trim());
  await s.set('bambuPath', (bambuPath || DEFAULT_BAMBU).trim());
  await s.save();
}

export async function hasKeys() {
  const s = await getSettings();
  return !!(s.accessKey && s.secretKey);
}

// ---------- onshape fetch ----------

// Onshape redirects some endpoints (e.g. STL export) to a sibling
// *.onshape.com host; auto-follow would drop the Authorization header on the
// cross-origin hop, so follow redirects by hand and re-attach auth while the
// URL stays on onshape.com.
async function onshape(pathOrUrl, accept = 'application/json;charset=UTF-8;qs=0.09') {
  const s = await getSettings();
  if (!s.accessKey || !s.secretKey) throw new Error('Onshape API keys not set — open Settings');
  const auth = 'Basic ' + btoa(`${s.accessKey}:${s.secretKey}`);
  let url = new URL(pathOrUrl, BASE);
  for (let hops = 0; hops < 5; hops++) {
    const headers = { Accept: accept };
    if (url.hostname.endsWith('.onshape.com')) headers.Authorization = auth;
    const res = await T.http.fetch(url.toString(), { method: 'GET', headers, maxRedirections: 0 });
    apiCallsThisSession++;
    if (res.status >= 300 && res.status < 400) {
      const loc = res.headers.get('location');
      if (!loc) throw new Error(`redirect without location (${res.status})`);
      url = new URL(loc, url);
      continue;
    }
    return res;
  }
  throw new Error('too many redirects: ' + pathOrUrl);
}

// ---------- cache helpers ----------

async function initDirs() {
  for (const dir of ['cache', 'cache/thumbs', 'cache/elements', 'cache/parts', 'exports']) {
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
    /* dir may not exist yet */
  }
}

function sanitize(name) {
  return (name || 'model').replace(/[^\w.\- ]+/g, '_').replace(/\s+/g, ' ').trim().slice(0, 80);
}

// ---------- documents ----------

export async function getDocs(refresh = false) {
  await ready;
  if (refresh) await removeMatching('cache/thumbs', () => true);
  let data = refresh ? null : await readJsonCache('cache/docs.json');
  const fromCache = !!data;
  if (!data) {
    const items = [];
    let next = '/api/v6/documents?filter=0&limit=20&sortColumn=modifiedAt&sortOrder=desc';
    while (next && items.length < 200) {
      const res = await onshape(next);
      if (!res.ok) throw new Error(`documents ${res.status}: ${(await res.text()).slice(0, 200)}`);
      const page = await res.json();
      for (const d of page.items ?? []) {
        items.push({
          id: d.id,
          name: d.name,
          modifiedAt: d.modifiedAt,
          workspaceId: d.defaultWorkspace?.id,
          thumbHref: d.thumbnail?.sizes?.find(x => x.size === '300x170')?.href
            ?? d.thumbnail?.sizes?.[0]?.href ?? null,
        });
      }
      next = page.next || null;
    }
    data = { fetchedAt: new Date().toISOString(), items };
    await writeJsonCache('cache/docs.json', data);
  }
  return { ...data, fromCache };
}

// ---------- elements ----------

// 3MF blob tabs are frozen snapshots of an export — they silently go stale
// when the source part studio changes, so they are hidden entirely.
function isStale3mfBlob(e) {
  return e.elementType === 'BLOB' && (e.dataType === 'application/3mf' || /\.3mf$/i.test(e.name || ''));
}

const PRINTABLE_BLOBS = { 'model/stl': '.stl', 'application/sla': '.stl' };

export async function getElements(doc, refresh = false) {
  await ready;
  const rel = `cache/elements/${doc.id}.json`;
  // An element-list refresh also invalidates this doc's cached part lists.
  if (refresh) await removeMatching('cache/parts', name => name.startsWith(`${doc.id}-`));
  let data = refresh ? null : await readJsonCache(rel);
  const fromCache = !!data;
  if (!data) {
    const res = await onshape(`/api/v6/documents/d/${doc.id}/w/${doc.workspaceId}/elements`);
    if (!res.ok) throw new Error(`elements ${res.status}: ${(await res.text()).slice(0, 200)}`);
    const els = await res.json();
    data = {
      fetchedAt: new Date().toISOString(),
      items: els.map(e => ({
        id: e.id,
        name: e.name,
        elementType: e.elementType,
        dataType: e.dataType,
        printable: e.elementType === 'PARTSTUDIO'
          || (e.elementType === 'BLOB' && (e.dataType in PRINTABLE_BLOBS || /\.stl$/i.test(e.name))),
      })),
    };
    await writeJsonCache(rel, data);
  }
  return { ...data, items: data.items.filter(e => !isStale3mfBlob(e)), fromCache };
}

// ---------- parts ----------

export async function getParts(doc, eid, refresh = false) {
  await ready;
  const rel = `cache/parts/${doc.id}-${eid}.json`;
  let data = refresh ? null : await readJsonCache(rel);
  const fromCache = !!data;
  if (!data) {
    const res = await onshape(`/api/v6/parts/d/${doc.id}/w/${doc.workspaceId}/e/${eid}`);
    if (!res.ok) throw new Error(`parts ${res.status}: ${(await res.text()).slice(0, 200)}`);
    const parts = await res.json();
    data = {
      fetchedAt: new Date().toISOString(),
      items: parts.map(p => ({ partId: p.partId, name: p.name, bodyType: p.bodyType, isMesh: p.isMesh })),
    };
    await writeJsonCache(rel, data);
  }
  return { ...data, fromCache };
}

// ---------- thumbnails ----------

const thumbUrls = new Map();

export async function getThumb(doc) {
  await ready;
  if (thumbUrls.has(doc.id)) return thumbUrls.get(doc.id);
  const rel = `cache/thumbs/${doc.id}.png`;
  let bytes;
  if (await fs.exists(rel, AppData)) {
    bytes = await fs.readFile(rel, AppData);
  } else {
    if (!doc.thumbHref) return null;
    const res = await onshape(doc.thumbHref, 'image/png');
    if (!res.ok) return null;
    bytes = new Uint8Array(await res.arrayBuffer());
    await fs.writeFile(rel, bytes, AppData);
  }
  const url = URL.createObjectURL(new Blob([bytes], { type: 'image/png' }));
  thumbUrls.set(doc.id, url);
  return url;
}

export function clearThumbUrls() {
  for (const url of thumbUrls.values()) URL.revokeObjectURL(url);
  thumbUrls.clear();
}

// ---------- print ----------

export async function print({ doc, el, part = null }) {
  await ready;
  const name = part ? `${el.name} - ${part.name}` : el.name;
  let bytes, ext;
  if (el.elementType === 'PARTSTUDIO') {
    const partFilter = part ? `&partIds=${encodeURIComponent(part.partId)}` : '';
    const res = await onshape(
      `/api/v6/partstudios/d/${doc.id}/w/${doc.workspaceId}/e/${el.id}/stl?units=millimeter&mode=binary&grouping=true${partFilter}`,
      '*/*');
    if (!res.ok) throw new Error(`STL export failed (${res.status}): ${(await res.text()).slice(0, 200)}`);
    bytes = new Uint8Array(await res.arrayBuffer());
    ext = '.stl';
  } else if (el.elementType === 'BLOB') {
    const res = await onshape(`/api/v6/blobelements/d/${doc.id}/w/${doc.workspaceId}/e/${el.id}`, '*/*');
    if (!res.ok) throw new Error(`blob download failed (${res.status})`);
    bytes = new Uint8Array(await res.arrayBuffer());
    ext = PRINTABLE_BLOBS[el.dataType] ?? '.stl';
  } else {
    throw new Error(`unsupported element type ${el.elementType}`);
  }

  const rel = `exports/${sanitize(name).replace(/\.stl$/i, '')}${ext}`;
  await fs.writeFile(rel, bytes, AppData);

  const s = await getSettings();
  const file = await T.path.join(await T.path.appDataDir(), ...rel.split('/'));
  await T.core.invoke('launch_slicer', { exe: s.bambuPath, file });
  return { file, bytes: bytes.length };
}

// ---------- self-test (dev verification hook) ----------

export async function runSelfTest() {
  const out = { startedAt: new Date().toISOString(), steps: [] };
  const step = (name, ok, detail) => out.steps.push({ name, ok, detail });
  try {
    const docs = await getDocs(true);
    step('docs', docs.items.length > 0, `${docs.items.length} docs, fromCache=${docs.fromCache}`);
    const doc = docs.items.find(d => d.name === 'button holders') ?? docs.items[0];
    const thumb = await getThumb(doc);
    step('thumb', !!thumb, thumb ? 'blob url ok' : 'no thumbnail');
    const els = await getElements(doc, true);
    step('elements', els.items.length > 0,
      els.items.map(e => `${e.name}:${e.elementType}`).join(', '));
    step('no-3mf-blobs', !els.items.some(e => /\.3mf$/i.test(e.name)), 'stale 3mf blobs hidden');
    const ps = els.items.find(e => e.elementType === 'PARTSTUDIO');
    const parts = await getParts(doc, ps.id, true);
    step('parts', parts.items.length > 0, parts.items.map(p => `${p.name}(${p.bodyType})`).join(', '));
    const solid = parts.items.find(p => p.bodyType === 'solid') ?? parts.items[0];
    const res = await print({ doc, el: ps, part: solid });
    step('print', res.bytes > 100, `${res.bytes} bytes → ${res.file}`);
    out.ok = out.steps.every(s => s.ok);
  } catch (err) {
    out.ok = false;
    out.error = String(err?.message || err);
  }
  out.apiCalls = apiCallsThisSession;
  out.finishedAt = new Date().toISOString();
  await fs.writeTextFile('selftest-result.json', JSON.stringify(out, null, 2), AppData);
  return out;
}
