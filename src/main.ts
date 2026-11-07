import { getVersion } from "@tauri-apps/api/app";
import { relaunch } from "@tauri-apps/plugin-process";
import { check } from "@tauri-apps/plugin-updater";
import * as api from "./api";
import type {
  AuthMode,
  AuthSettings,
  Doc,
  Element,
  Part,
  Refresh,
  SaveResult,
  SlicerState,
} from "./types";
import {
  authConfigured,
  buildAuthInput,
  elementTypeLabel,
  esc,
  fileName,
  filterDocs,
  fmtDate,
  plural,
  refreshSummary,
  secretPlaceholder,
  selectedSlicer,
  slicerName,
  slicerOptionLabel,
} from "./ui";

function $<T extends HTMLElement = HTMLElement>(selector: string): T {
  const el = document.querySelector<T>(selector);
  if (!el) throw new Error(`missing element ${selector}`);
  return el;
}

// Tauri rejects invoke() with the command's error string.
function errMsg(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

let docs: Doc[] = [];
let currentDoc: Doc | null = null;
let appVersion = "";
getVersion()
  .then((v) => (appVersion = v))
  .catch(() => {});

let toastTimer: ReturnType<typeof setTimeout> | undefined;
function toast(msg: string, isErr = false, ms = 3500): void {
  const t = $("#toast");
  t.textContent = msg;
  t.className = "show" + (isErr ? " err" : "");
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (t.className = ""), ms);
}

function render(): void {
  const shown = filterDocs(docs, $<HTMLInputElement>("#search").value);
  const grid = $("#grid");
  grid.innerHTML = "";
  $("#empty").hidden = shown.length > 0;
  for (const d of shown) {
    const card = document.createElement("div");
    card.className = "card";
    card.innerHTML = `
      <div class="noimg">\u2026</div>
      <div class="info"><div class="name">${esc(d.name)}</div><div class="date">${fmtDate(d.modifiedAt)}</div></div>`;
    const placeholder = card.querySelector<HTMLElement>(".noimg");
    const noPreview = () => {
      if (placeholder) placeholder.textContent = "no preview";
    };
    api
      .getThumb(d)
      .then((url) => {
        if (!url) return noPreview();
        const img = document.createElement("img");
        img.src = url;
        placeholder?.replaceWith(img);
      })
      .catch(noPreview);
    card.onclick = () => void openDoc(d);
    grid.appendChild(card);
  }
}

let docsMeta = "";

async function updateMeta(): Promise<void> {
  const c = await api.getCallStats();
  const since = c.since ? ` since ${fmtDate(c.since)}` : "";
  const meta = $("#meta");
  meta.textContent = `${docsMeta} \u00b7 API calls: ${c.session} this session, ${c.total} total`;
  meta.title =
    `${c.total} API calls counted by onslicer${since}. ` +
    "Authoritative count: cad.onshape.com/user/developer";
}

async function loadDocs(refresh: Refresh = "none"): Promise<void> {
  const btn = $<HTMLButtonElement>("#refresh");
  btn.disabled = true;
  try {
    const data = await api.getDocs(refresh);
    docs = data.items;
    const v = appVersion ? `v${appVersion} \u00b7 ` : "";
    docsMeta = `${v}${plural(docs.length, "doc")} \u00b7 ${data.fromCache ? "cached" : "fresh"} ${fmtDate(data.fetchedAt)}`;
    render();
    if (refresh !== "none") toast(refreshSummary(data.changed, data.newCalls));
  } catch (e) {
    toast("Failed to load documents: " + errMsg(e), true, 6000);
  } finally {
    btn.disabled = false;
    updateMeta().catch(() => {});
  }
}

// ---------- slicer state ----------

// null until the first list_slicers answer; Print stays disabled until then.
let slicerState: SlicerState | null = null;

function canPrint(): boolean {
  return Boolean(selectedSlicer(slicerState));
}

function printTitle(): string {
  return canPrint()
    ? `Export and open in ${slicerName(slicerState)}`
    : "No slicer selected - open Settings";
}

function saveTitle(): string {
  return `Re-export and overwrite the file in place (then use "Reload from disk" in ${slicerName(slicerState)})`;
}

// Banner, Print buttons and the settings dropdown all follow the state.
function applySlicerState(state: SlicerState): void {
  slicerState = state;
  $("#slicer-banner").hidden = canPrint();
  for (const btn of document.querySelectorAll<HTMLButtonElement>("button.print")) {
    if (btn.dataset.busy) continue;
    btn.disabled = !canPrint();
    btn.title = printTitle();
  }
  for (const btn of document.querySelectorAll<HTMLButtonElement>("button.save")) {
    btn.title = saveTitle();
  }
  renderSlicerSelect();
}

async function refreshSlicers(fn: () => Promise<SlicerState> = api.listSlicers): Promise<void> {
  const errBox = $("#slicer-error");
  try {
    applySlicerState(await fn());
    errBox.hidden = true;
  } catch (e) {
    errBox.textContent = errMsg(e);
    errBox.hidden = false;
    // Re-render so the dropdown snaps back to the real selection.
    if (slicerState) renderSlicerSelect();
  }
}

// Option values for the non-slicer entries; never an absolute path.
const OTHER = ":other";
const NONE = ":none";

function renderSlicerSelect(): void {
  const select = $<HTMLSelectElement>("#set-slicer");
  select.innerHTML = "";
  const add = (value: string, label: string, title = "") => {
    const opt = document.createElement("option");
    opt.value = value;
    opt.textContent = label;
    opt.title = title;
    select.appendChild(opt);
    return opt;
  };
  const state = slicerState;
  const selected = selectedSlicer(state);
  if (!selected) {
    const none = add(NONE, state?.slicers.length ? "Select a slicer" : "No slicer detected");
    none.disabled = true;
  }
  for (const s of state?.slicers ?? []) add(s.path, slicerOptionLabel(s), s.path);
  add(OTHER, "Other...", "Pick a slicer executable");
  select.value = selected?.path ?? NONE;
  $("#slicer-path").textContent = selected?.path ?? "";
}

async function onSlicerChange(): Promise<void> {
  const value = $<HTMLSelectElement>("#set-slicer").value;
  if (value === NONE) return;
  // Cancelling the picker returns the unchanged state, which re-renders the
  // dropdown back to the previous selection.
  await refreshSlicers(value === OTHER ? api.pickManualSlicer : () => api.selectSlicer(value));
}

async function rescanSlicers(): Promise<void> {
  const btn = $<HTMLButtonElement>("#slicer-rescan");
  btn.disabled = true;
  try {
    await refreshSlicers(api.rescanSlicers);
  } finally {
    btn.disabled = false;
  }
}

// ---------- document dialog ----------

// One entry per element row. Part studios also carry their parts box; `parts`
// is null until the list is loaded (from disk cache or an explicit fetch).
// `expanded` is the user's toggle; `autoExpanded` is set by the filter when a
// part name matches, and is cleared by the user's next toggle or filter edit.
interface Entry {
  el: Element;
  row: HTMLElement;
  box: HTMLElement | null;
  expandBtn: HTMLButtonElement | null;
  parts: Part[] | null;
  partRows: { p: Part; row: HTMLElement }[];
  expanded: boolean;
  autoExpanded: boolean;
}

let dlgEntries: Entry[] = [];
let dlgNote: HTMLElement | null = null;
let dlgGen = 0;

async function openDoc(d: Doc, refresh: Refresh = "none"): Promise<void> {
  const gen = ++dlgGen;
  currentDoc = d;
  dlgEntries = [];
  dlgNote = null;
  $("#dlg-title").textContent = d.name;
  $("#dlg-body").innerHTML =
    '<div style="padding:20px;text-align:center"><span class="spin"></span></div>';
  const dlg = $<HTMLDialogElement>("#dlg");
  if (!dlg.open) {
    $<HTMLInputElement>("#dlg-filter").value = "";
    dlg.showModal();
  }
  try {
    const data = await api.getElements(d, refresh);
    if (gen !== dlgGen) return;
    if (data.staleError)
      toast(`Showing cached tabs (doc changed, refetch failed): ${data.staleError}`, true, 6000);
    const body = $("#dlg-body");
    body.innerHTML = "";
    for (const el of data.items) {
      const row = document.createElement("div");
      row.className = "el" + (el.printable ? "" : " dim");
      row.innerHTML = `<span class="type">${esc(elementTypeLabel(el))}</span><span class="ename">${esc(el.name)}</span>`;
      const entry: Entry = {
        el,
        row,
        box: null,
        expandBtn: null,
        parts: null,
        partRows: [],
        expanded: false,
        autoExpanded: false,
      };
      if (el.elementType === "PARTSTUDIO") {
        const expand = document.createElement("button");
        expand.className = "expand";
        expand.textContent = "\u25b8 parts";
        expand.title = "List individual parts (1 API call, then cached)";
        expand.onclick = () => void togglePartList(entry);
        row.appendChild(expand);
        entry.expandBtn = expand;
        entry.box = document.createElement("div");
        entry.box.className = "parts";
        entry.box.hidden = true;
      }
      if (el.printable)
        addActions(row, el, null, el.elementType === "PARTSTUDIO" ? "Print all" : "Print");
      body.appendChild(row);
      if (entry.box) body.appendChild(entry.box);
      dlgEntries.push(entry);
    }
    if (!data.items.length) {
      body.innerHTML = '<div style="padding:20px;color:var(--muted)">Empty document</div>';
      return;
    }
    dlgNote = document.createElement("div");
    dlgNote.className = "dlg-note";
    dlgNote.hidden = true;
    body.appendChild(dlgNote);
    // Part lists already on disk are free to read and make part names filterable.
    await Promise.all(
      dlgEntries
        .filter((e) => e.box)
        .map(async (e) => {
          const cached = await api.peekParts(d, e.el.id);
          if (cached && gen === dlgGen) renderParts(e, cached.items);
        }),
    );
    if (gen !== dlgGen) return;
    applyDlgFilter(true);
  } catch (e) {
    if (gen !== dlgGen) return;
    const msg = errMsg(e);
    // Incremental doc refreshes can't see deletions, so a doc that vanished
    // from Onshape still has a card until the next full re-list.
    const gone = /^elements (403|404)\b/.test(msg)
      ? "<br><br>If this document was deleted, Shift+click the main refresh button for a full re-list."
      : "";
    $("#dlg-body").innerHTML = `<div style="padding:20px;color:#e74c3c">${esc(msg)}${gone}</div>`;
  } finally {
    if (gen === dlgGen) updateMeta().catch(() => {});
  }
}

function renderParts(entry: Entry, items: Part[]): void {
  const box = entry.box;
  if (!box) return;
  entry.parts = items;
  entry.partRows = [];
  box.innerHTML = "";
  for (const p of items) {
    const row = document.createElement("div");
    row.className = "el";
    row.innerHTML = `<span class="type">${esc(p.bodyType)}</span><span class="ename">${esc(p.name)}</span>`;
    addActions(row, entry.el, p, "Print");
    box.appendChild(row);
    entry.partRows.push({ p, row });
  }
  if (!items.length)
    box.innerHTML = '<div style="padding:8px 12px;color:var(--muted)">No parts</div>';
}

async function loadParts(entry: Entry): Promise<boolean> {
  const gen = dlgGen;
  const box = entry.box;
  if (!box || !currentDoc) return false;
  box.innerHTML = '<div style="padding:8px 12px"><span class="spin"></span></div>';
  try {
    const data = await api.getParts(currentDoc, entry.el.id);
    if (gen !== dlgGen) return false;
    renderParts(entry, data.items);
    return true;
  } catch (e) {
    if (gen !== dlgGen) return false;
    box.innerHTML = `<div style="padding:8px 12px;color:#e74c3c">${esc(errMsg(e))}</div>`;
    return false;
  }
}

async function togglePartList(entry: Entry): Promise<void> {
  const opening = Boolean(entry.box?.hidden);
  entry.expanded = opening;
  entry.autoExpanded = false;
  applyDlgFilter();
  if (opening && !entry.parts) {
    await loadParts(entry);
    applyDlgFilter();
  }
}

// Show/hide rows to match the filter box. Rebuilding the DOM would lose expand
// state, so this only toggles `hidden`. `fromInput` is true when the filter
// text itself changed, which is the only time auto-expansion is recomputed.
function applyDlgFilter(fromInput = false): void {
  const q = $<HTMLInputElement>("#dlg-filter").value.trim().toLowerCase();
  const hit = (s: string | null | undefined) => (s ?? "").toLowerCase().includes(q);
  let shown = 0;
  const unsearched: Entry[] = [];
  for (const e of dlgEntries) {
    const nameHit = !q || hit(e.el.name);
    // Only match part names when the query misses the studio name; a studio
    // that matches by name shows all its parts.
    const partOnly = Boolean(q) && !nameHit && Boolean(e.parts?.some((p) => hit(p.name)));
    e.row.hidden = !(nameHit || partOnly);
    if (!e.row.hidden) shown++;
    if (!e.box || !e.expandBtn) continue;
    if (q && !nameHit && !e.parts) unsearched.push(e);
    if (fromInput) e.autoExpanded = partOnly;
    const open = !e.row.hidden && (e.expanded || e.autoExpanded);
    e.box.hidden = !open;
    e.expandBtn.textContent = open ? "\u25be parts" : "\u25b8 parts";
    for (const { p, row } of e.partRows) row.hidden = partOnly && !hit(p.name);
  }
  const note = dlgNote;
  if (!note) return;
  note.innerHTML = "";
  const say = (text: string) => {
    const span = document.createElement("span");
    span.textContent = text;
    note.appendChild(span);
  };
  if (q && !shown) say("No matches.");
  if (unsearched.length) {
    const n = unsearched.length;
    say(`Part names not searched in ${plural(n, "part studio")} (parts not listed yet).`);
    const btn = document.createElement("button");
    btn.textContent = `List parts (${plural(n, "API call")})`;
    btn.title =
      "Fetch the part lists of the hidden part studios so their part names can be searched";
    btn.onclick = async () => {
      btn.disabled = true;
      btn.innerHTML = '<span class="spin"></span>';
      const ok = await Promise.all(unsearched.map(loadParts));
      if (ok.some((x) => !x)) toast("Some part lists failed to load", true);
      applyDlgFilter(true);
    };
    note.appendChild(btn);
  }
  note.hidden = !note.childNodes.length;
}

const SAVE_ICON =
  '<svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"><path fill="currentColor" d="M2 1h9.6L15 4.4V14a1 1 0 0 1-1 1H2a1 1 0 0 1-1-1V2a1 1 0 0 1 1-1zm2 1.5V6h7V2.5zM8 9a2 2 0 1 0 0 4 2 2 0 0 0 0-4z"/></svg>';

// Save (re-export in place, no slicer launch) + Print buttons for one target.
function addActions(row: HTMLElement, el: Element, part: Part | null, label: string): void {
  const doc = currentDoc;
  if (!doc) return;
  const target = { doc, el, part };
  const save = document.createElement("button");
  save.className = "save";
  save.title = saveTitle();
  save.innerHTML = SAVE_ICON;
  save.onclick = () =>
    void runAction(
      save,
      () => api.saveExport(target),
      (r) => `Saved ${fileName(r.file)} \u2713`,
      "Save failed",
      SAVE_ICON,
    );
  const print = document.createElement("button");
  print.className = "primary print";
  print.textContent = label;
  print.disabled = !canPrint();
  print.title = printTitle();
  print.onclick = () => {
    const name = slicerName(slicerState);
    void runAction(
      print,
      () => api.printExport(target),
      () => `Sent to ${name} \u2713`,
      "Print failed",
      esc(label),
    ).then(() => refreshSlicers());
  };
  row.append(save, print);
}

async function runAction(
  btn: HTMLButtonElement,
  fn: () => Promise<SaveResult>,
  okMsg: (r: SaveResult) => string,
  errPrefix: string,
  restoreHtml: string,
): Promise<void> {
  btn.disabled = true;
  btn.dataset.busy = "1";
  btn.innerHTML = '<span class="spin"></span>';
  try {
    toast(okMsg(await fn()));
  } catch (e) {
    toast(`${errPrefix}: ${errMsg(e)}`, true, 6000);
  } finally {
    delete btn.dataset.busy;
    btn.disabled = btn.classList.contains("print") && !canPrint();
    btn.innerHTML = restoreHtml;
    updateMeta().catch(() => {});
  }
}

// ---------- settings ----------

let authSettings: AuthSettings | null = null;

function showModeFields(): void {
  const mode = $<HTMLSelectElement>("#set-mode").value;
  for (const box of document.querySelectorAll<HTMLElement>("#settings .mode-fields")) {
    box.hidden = box.dataset.mode !== mode;
  }
}

function fillAuthForm(s: AuthSettings): void {
  authSettings = s;
  $<HTMLSelectElement>("#set-mode").value = s.mode;
  $<HTMLInputElement>("#set-username").value = s.username;
  const secrets: [string, boolean, string][] = [
    ["#set-access", s.hasApiKeys, "on_\u2026"],
    ["#set-secret", s.hasApiKeys, "secret"],
    ["#set-password", s.hasPassword, "password"],
    ["#set-session", s.hasSession, "cookie"],
  ];
  for (const [sel, stored, fallback] of secrets) {
    const input = $<HTMLInputElement | HTMLTextAreaElement>(sel);
    input.value = "";
    input.placeholder = secretPlaceholder(stored, fallback);
  }
  showModeFields();
}

async function openSettings(): Promise<void> {
  $("#set-error").hidden = true;
  const dlg = $<HTMLDialogElement>("#settings");
  try {
    fillAuthForm(await api.getAuthSettings());
  } catch (e) {
    $("#set-error").textContent = errMsg(e);
    $("#set-error").hidden = false;
  }
  if (!dlg.open) dlg.showModal();
  await refreshSlicers();
}

async function saveSettings(): Promise<void> {
  const btn = $<HTMLButtonElement>("#settings-save");
  const errBox = $("#set-error");
  const input = buildAuthInput(
    {
      mode: $<HTMLSelectElement>("#set-mode").value as AuthMode,
      accessKey: $<HTMLInputElement>("#set-access").value,
      secretKey: $<HTMLInputElement>("#set-secret").value,
      username: $<HTMLInputElement>("#set-username").value,
      password: $<HTMLInputElement>("#set-password").value,
      session: $<HTMLTextAreaElement>("#set-session").value,
    },
    authSettings,
  );
  btn.disabled = true;
  errBox.hidden = true;
  try {
    fillAuthForm(await api.saveAuthSettings(input));
  } catch (e) {
    errBox.textContent = errMsg(e);
    errBox.hidden = false;
    return;
  } finally {
    btn.disabled = false;
  }
  $<HTMLDialogElement>("#settings").close();
  toast("Settings saved");
  void loadDocs(docs.length === 0 ? "normal" : "none");
}

// ---------- auto-update ----------

async function checkForUpdate(): Promise<void> {
  // A debug build is always "older" than the latest release (its version is
  // the 0.1.0 placeholder), and installing would replace the user's installed
  // app, so only release builds update themselves.
  if (import.meta.env.TAURI_ENV_DEBUG === "true") return;
  try {
    const update = await check();
    if (update) {
      toast(`Downloading update v${update.version}\u2026`, false, 5000);
      await update.downloadAndInstall();
      $("#update-text").textContent = `Update v${update.version} installed.`;
      $("#update-banner").style.display = "block";
    }
  } catch {
    /* no releases yet / offline - never block the app on updates */
  }
}

// ---------- init ----------

// Click = incremental refresh (cheap); Shift+click = full re-list.
$("#refresh").onclick = (e) => void loadDocs(e.shiftKey ? "full" : "normal");
$("#dlg-refresh").onclick = (e) => {
  if (currentDoc) void openDoc(currentDoc, e.shiftKey ? "full" : "normal");
};
$("#dlg-close").onclick = () => $<HTMLDialogElement>("#dlg").close();
$("#dlg-filter").oninput = () => applyDlgFilter(true);
$("#search").oninput = render;
$("#open-settings").onclick = () => void openSettings();
$("#slicer-banner-settings").onclick = () => void openSettings();
$("#settings-close").onclick = () => $<HTMLDialogElement>("#settings").close();
$("#settings-save").onclick = () => void saveSettings();
$("#set-mode").onchange = showModeFields;
$("#set-slicer").onchange = () => void onSlicerChange();
$("#slicer-rescan").onclick = () => void rescanSlicers();
$("#update-restart").onclick = () => void relaunch();

void (async () => {
  void checkForUpdate();
  void refreshSlicers();
  let auth: AuthSettings;
  try {
    auth = await api.getAuthSettings();
  } catch (e) {
    toast("Cannot read the sign-in settings: " + errMsg(e), true, 6000);
    return;
  }
  if (!authConfigured(auth)) {
    await openSettings();
    return;
  }
  await loadDocs();
})();
