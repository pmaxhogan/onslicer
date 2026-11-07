import * as api from "./api.js";

const $ = (s) => document.querySelector(s);
let docs = [];
let currentDoc = null;
let appVersion = "";
window.__TAURI__.app
  .getVersion()
  .then((v) => (appVersion = v))
  .catch(() => {});

function toast(msg, isErr = false, ms = 3500) {
  const t = $("#toast");
  t.textContent = msg;
  t.className = "show" + (isErr ? " err" : "");
  clearTimeout(t._h);
  t._h = setTimeout(() => (t.className = ""), ms);
}

function fmtDate(iso) {
  return iso
    ? new Date(iso).toLocaleDateString(undefined, {
        year: "numeric",
        month: "short",
        day: "numeric",
      })
    : "";
}

function esc(s) {
  return (s ?? "").replace(
    /[&<>"]/g,
    (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c],
  );
}

function render() {
  const filter = $("#search").value.trim().toLowerCase();
  const shown = docs.filter((d) => !filter || d.name.toLowerCase().includes(filter));
  $("#grid").innerHTML = "";
  $("#empty").hidden = shown.length > 0;
  for (const d of shown) {
    const card = document.createElement("div");
    card.className = "card";
    card.innerHTML = `
      <div class="noimg">\u2026</div>
      <div class="info"><div class="name">${esc(d.name)}</div><div class="date">${fmtDate(d.modifiedAt)}</div></div>`;
    api
      .getThumb(d)
      .then((url) => {
        if (!url) {
          card.querySelector(".noimg").textContent = "no preview";
          return;
        }
        const img = document.createElement("img");
        img.src = url;
        card.querySelector(".noimg").replaceWith(img);
      })
      .catch(() => (card.querySelector(".noimg").textContent = "no preview"));
    card.onclick = () => openDoc(d);
    $("#grid").appendChild(card);
  }
}

let docsMeta = "";

async function updateMeta() {
  const c = await api.getCallStats();
  const since = c.since ? ` since ${fmtDate(c.since)}` : "";
  $("#meta").textContent =
    `${docsMeta} \u00b7 API calls: ${c.session} this session, ${c.total} total`;
  $("#meta").title =
    `${c.total} API calls counted by onslicer${since}. ` +
    "Authoritative count: cad.onshape.com/user/developer";
}

// refresh: false (cache), true (incremental: only new/changed docs), 'full'.
async function loadDocs(refresh = false) {
  $("#refresh").disabled = true;
  try {
    const data = await api.getDocs(refresh);
    docs = data.items;
    const v = appVersion ? `v${appVersion} \u00b7 ` : "";
    docsMeta = `${v}${docs.length} docs \u00b7 ${data.fromCache ? "cached" : "fresh"} ${fmtDate(data.fetchedAt)}`;
    render();
    if (refresh) {
      const n = data.changed;
      toast(
        `${n ? `${n} new/changed doc${n === 1 ? "" : "s"}` : "No changes"} \u00b7 ${data.newCalls} API call${data.newCalls === 1 ? "" : "s"}`,
      );
    }
  } catch (e) {
    toast("Failed to load documents: " + e.message, true, 6000);
  } finally {
    $("#refresh").disabled = false;
    updateMeta().catch(() => {});
  }
}

// ---------- document dialog ----------

// One entry per element row. Part studios also carry their parts box; `parts`
// is null until the list is loaded (from disk cache or an explicit fetch).
// `expanded` is the user's toggle; `autoExpanded` is set by the filter when a
// part name matches, and is cleared by the user's next toggle or filter edit.
let dlgEntries = [];
let dlgNote = null;
let dlgGen = 0;

async function openDoc(d, refresh = false) {
  const gen = ++dlgGen;
  currentDoc = d;
  dlgEntries = [];
  dlgNote = null;
  $("#dlg-title").textContent = d.name;
  $("#dlg-body").innerHTML =
    '<div style="padding:20px;text-align:center"><span class="spin"></span></div>';
  if (!$("#dlg").open) {
    $("#dlg-filter").value = "";
    $("#dlg").showModal();
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
      const typeLabel =
        el.elementType === "PARTSTUDIO"
          ? "part studio"
          : el.elementType === "BLOB"
            ? el.name.match(/\.\w+$/)?.[0].slice(1) || "file"
            : el.elementType.toLowerCase();
      row.innerHTML = `<span class="type">${esc(typeLabel)}</span><span class="ename">${esc(el.name)}</span>`;
      const entry = {
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
        expand.onclick = () => togglePartList(entry);
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
    // Incremental doc refreshes can't see deletions, so a doc that vanished
    // from Onshape still has a card until the next full re-list.
    const gone = /^elements (403|404)\b/.test(e.message)
      ? "<br><br>If this document was deleted, Shift+click the main refresh button for a full re-list."
      : "";
    $("#dlg-body").innerHTML =
      `<div style="padding:20px;color:#e74c3c">${esc(e.message)}${gone}</div>`;
  } finally {
    if (gen === dlgGen) updateMeta().catch(() => {});
  }
}

function renderParts(entry, items) {
  entry.parts = items;
  entry.partRows = [];
  entry.box.innerHTML = "";
  for (const p of items) {
    const row = document.createElement("div");
    row.className = "el";
    row.innerHTML = `<span class="type">${esc(p.bodyType)}</span><span class="ename">${esc(p.name)}</span>`;
    addActions(row, entry.el, p, "Print");
    entry.box.appendChild(row);
    entry.partRows.push({ p, row });
  }
  if (!items.length)
    entry.box.innerHTML = '<div style="padding:8px 12px;color:var(--muted)">No parts</div>';
}

async function loadParts(entry) {
  const gen = dlgGen;
  entry.box.innerHTML = '<div style="padding:8px 12px"><span class="spin"></span></div>';
  try {
    const data = await api.getParts(currentDoc, entry.el.id);
    if (gen !== dlgGen) return false;
    renderParts(entry, data.items);
    return true;
  } catch (e) {
    if (gen !== dlgGen) return false;
    entry.box.innerHTML = `<div style="padding:8px 12px;color:#e74c3c">${esc(e.message)}</div>`;
    return false;
  }
}

async function togglePartList(entry) {
  const opening = entry.box.hidden;
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
function applyDlgFilter(fromInput = false) {
  const q = $("#dlg-filter").value.trim().toLowerCase();
  const hit = (s) => (s ?? "").toLowerCase().includes(q);
  let shown = 0;
  const unsearched = [];
  for (const e of dlgEntries) {
    const nameHit = !q || hit(e.el.name);
    // Only match part names when the query misses the studio name; a studio
    // that matches by name shows all its parts.
    const partOnly = Boolean(q) && !nameHit && Boolean(e.parts) && e.parts.some((p) => hit(p.name));
    e.row.hidden = !(nameHit || partOnly);
    if (!e.row.hidden) shown++;
    if (!e.box) continue;
    if (q && !nameHit && !e.parts) unsearched.push(e);
    if (fromInput) e.autoExpanded = partOnly;
    const open = !e.row.hidden && (e.expanded || e.autoExpanded);
    e.box.hidden = !open;
    e.expandBtn.textContent = open ? "\u25be parts" : "\u25b8 parts";
    for (const { p, row } of e.partRows) row.hidden = partOnly && !hit(p.name);
  }
  if (!dlgNote) return;
  dlgNote.innerHTML = "";
  const say = (text) => {
    const span = document.createElement("span");
    span.textContent = text;
    dlgNote.appendChild(span);
  };
  if (q && !shown) say("No matches.");
  if (unsearched.length) {
    const n = unsearched.length;
    const s = n === 1 ? "" : "s";
    say(`Part names not searched in ${n} part studio${s} (parts not listed yet).`);
    const btn = document.createElement("button");
    btn.textContent = `List parts (${n} API call${s})`;
    btn.title =
      "Fetch the part lists of the hidden part studios so their part names can be searched";
    btn.onclick = async () => {
      btn.disabled = true;
      btn.innerHTML = '<span class="spin"></span>';
      const ok = await Promise.all(unsearched.map(loadParts));
      if (ok.some((x) => !x)) toast("Some part lists failed to load", true);
      applyDlgFilter(true);
    };
    dlgNote.appendChild(btn);
  }
  dlgNote.hidden = !dlgNote.childNodes.length;
}

const SAVE_ICON =
  '<svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"><path fill="currentColor" d="M2 1h9.6L15 4.4V14a1 1 0 0 1-1 1H2a1 1 0 0 1-1-1V2a1 1 0 0 1 1-1zm2 1.5V6h7V2.5zM8 9a2 2 0 1 0 0 4 2 2 0 0 0 0-4z"/></svg>';

// Save (re-export in place, no slicer launch) + Print buttons for one target.
function addActions(row, el, part, label) {
  const save = document.createElement("button");
  save.className = "save";
  save.title =
    'Re-export and overwrite the file in place (then use "Reload from disk" in Bambu Studio)';
  save.innerHTML = SAVE_ICON;
  save.onclick = () =>
    runAction(
      save,
      () => api.save({ doc: currentDoc, el, part }),
      (r) => `Saved ${r.file.split(/[\\/]/).pop()} \u2713`,
      "Save failed",
      SAVE_ICON,
    );
  const print = document.createElement("button");
  print.className = "primary";
  print.textContent = label;
  print.onclick = () =>
    runAction(
      print,
      () => api.print({ doc: currentDoc, el, part }),
      () => "Sent to Bambu Studio \u2713",
      "Print failed",
      esc(label),
    );
  row.append(save, print);
}

async function runAction(btn, fn, okMsg, errPrefix, restoreHtml) {
  btn.disabled = true;
  btn.innerHTML = '<span class="spin"></span>';
  try {
    toast(okMsg(await fn()));
  } catch (e) {
    toast(`${errPrefix}: ${e.message}`, true, 6000);
  } finally {
    btn.disabled = false;
    btn.innerHTML = restoreHtml;
    updateMeta().catch(() => {});
  }
}

// ---------- settings ----------

async function openSettings() {
  const s = await api.getSettings();
  $("#set-access").value = s.accessKey;
  $("#set-secret").value = s.secretKey;
  $("#set-bambu").value = s.bambuPath;
  $("#settings").showModal();
}

async function saveSettings() {
  await api.saveSettings({
    accessKey: $("#set-access").value,
    secretKey: $("#set-secret").value,
    bambuPath: $("#set-bambu").value,
  });
  $("#settings").close();
  toast("Settings saved");
  loadDocs(docs.length === 0);
}

// ---------- auto-update ----------

async function checkForUpdate() {
  try {
    const update = await window.__TAURI__.updater.check();
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
$("#refresh").onclick = (e) => loadDocs(e.shiftKey ? "full" : true);
$("#dlg-refresh").onclick = (e) => openDoc(currentDoc, e.shiftKey ? "full" : true);
$("#dlg-close").onclick = () => $("#dlg").close();
$("#dlg-filter").oninput = () => applyDlgFilter(true);
$("#search").oninput = render;
$("#open-settings").onclick = openSettings;
$("#settings-close").onclick = () => $("#settings").close();
$("#settings-save").onclick = saveSettings;
$("#update-restart").onclick = () => window.__TAURI__.process.relaunch();

(async () => {
  checkForUpdate();
  const s = await api.getSettings();
  if (!s.accessKey || !s.secretKey) {
    openSettings();
    return;
  }
  await loadDocs();
  if (s.selftest) {
    const result = await api.runSelfTest();
    console.log("selftest", result);
    document.title = result.ok ? "onslicer [selftest OK]" : "onslicer [selftest FAILED]";
  }
})();
