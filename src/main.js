import * as api from './api.js';

const $ = s => document.querySelector(s);
let docs = [];
let currentDoc = null;
let appVersion = '';
window.__TAURI__.app.getVersion().then(v => appVersion = v).catch(() => {});

function toast(msg, isErr = false, ms = 3500) {
  const t = $('#toast');
  t.textContent = msg;
  t.className = 'show' + (isErr ? ' err' : '');
  clearTimeout(t._h);
  t._h = setTimeout(() => t.className = '', ms);
}

function fmtDate(iso) {
  return iso ? new Date(iso).toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' }) : '';
}

function esc(s) { return (s ?? '').replace(/[&<>"]/g, c => ({ '&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;' }[c])); }

function render() {
  const filter = $('#search').value.trim().toLowerCase();
  const shown = docs.filter(d => !filter || d.name.toLowerCase().includes(filter));
  $('#grid').innerHTML = '';
  $('#empty').hidden = shown.length > 0;
  for (const d of shown) {
    const card = document.createElement('div');
    card.className = 'card';
    card.innerHTML = `
      <div class="noimg">…</div>
      <div class="info"><div class="name">${esc(d.name)}</div><div class="date">${fmtDate(d.modifiedAt)}</div></div>`;
    api.getThumb(d).then(url => {
      if (!url) { card.querySelector('.noimg').textContent = 'no preview'; return; }
      const img = document.createElement('img');
      img.src = url;
      card.querySelector('.noimg').replaceWith(img);
    }).catch(() => card.querySelector('.noimg').textContent = 'no preview');
    card.onclick = () => openDoc(d);
    $('#grid').appendChild(card);
  }
}

async function loadDocs(refresh = false) {
  $('#refresh').disabled = true;
  try {
    if (refresh) api.clearThumbUrls();
    const data = await api.getDocs(refresh);
    docs = data.items;
    const v = appVersion ? `v${appVersion} · ` : '';
    $('#meta').textContent = `${v}${docs.length} docs · ${data.fromCache ? 'cached' : 'fresh'} ${fmtDate(data.fetchedAt)}`;
    render();
  } catch (e) {
    toast('Failed to load documents: ' + e.message, true, 6000);
  } finally {
    $('#refresh').disabled = false;
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
  $('#dlg-title').textContent = d.name;
  $('#dlg-body').innerHTML = '<div style="padding:20px;text-align:center"><span class="spin"></span></div>';
  if (!$('#dlg').open) {
    $('#dlg-filter').value = '';
    $('#dlg').showModal();
  }
  try {
    const data = await api.getElements(d, refresh);
    if (gen !== dlgGen) return;
    const body = $('#dlg-body');
    body.innerHTML = '';
    for (const el of data.items) {
      const row = document.createElement('div');
      row.className = 'el' + (el.printable ? '' : ' dim');
      const typeLabel = el.elementType === 'PARTSTUDIO' ? 'part studio'
        : el.elementType === 'BLOB' ? (el.name.match(/\.\w+$/)?.[0].slice(1) || 'file')
        : el.elementType.toLowerCase();
      row.innerHTML = `<span class="type">${esc(typeLabel)}</span><span class="ename">${esc(el.name)}</span>`;
      const entry = { el, row, box: null, expandBtn: null, parts: null, partRows: [],
        expanded: false, autoExpanded: false };
      if (el.elementType === 'PARTSTUDIO') {
        const expand = document.createElement('button');
        expand.className = 'expand';
        expand.textContent = '▸ parts';
        expand.title = 'List individual parts (1 API call, then cached)';
        expand.onclick = () => togglePartList(entry);
        row.appendChild(expand);
        entry.expandBtn = expand;
        entry.box = document.createElement('div');
        entry.box.className = 'parts';
        entry.box.hidden = true;
      }
      if (el.printable) {
        const btn = document.createElement('button');
        btn.className = 'primary';
        btn.textContent = el.elementType === 'PARTSTUDIO' ? 'Print all' : 'Print';
        btn.onclick = () => sendToPrint(el, btn);
        row.appendChild(btn);
      }
      body.appendChild(row);
      if (entry.box) body.appendChild(entry.box);
      dlgEntries.push(entry);
    }
    if (!data.items.length) {
      body.innerHTML = '<div style="padding:20px;color:var(--muted)">Empty document</div>';
      return;
    }
    dlgNote = document.createElement('div');
    dlgNote.className = 'dlg-note';
    dlgNote.hidden = true;
    body.appendChild(dlgNote);
    // Part lists already on disk are free to read and make part names filterable.
    await Promise.all(dlgEntries.filter(e => e.box).map(async e => {
      const cached = await api.peekParts(d, e.el.id);
      if (cached && gen === dlgGen) renderParts(e, cached.items);
    }));
    if (gen !== dlgGen) return;
    applyDlgFilter(true);
  } catch (e) {
    if (gen !== dlgGen) return;
    $('#dlg-body').innerHTML = `<div style="padding:20px;color:#e74c3c">${esc(e.message)}</div>`;
  }
}

function renderParts(entry, items) {
  entry.parts = items;
  entry.partRows = [];
  entry.box.innerHTML = '';
  for (const p of items) {
    const row = document.createElement('div');
    row.className = 'el';
    row.innerHTML = `<span class="type">${esc(p.bodyType)}</span><span class="ename">${esc(p.name)}</span>`;
    const btn = document.createElement('button');
    btn.className = 'primary';
    btn.textContent = 'Print';
    btn.onclick = () => sendToPrint(entry.el, btn, p);
    row.appendChild(btn);
    entry.box.appendChild(row);
    entry.partRows.push({ p, row });
  }
  if (!items.length) entry.box.innerHTML = '<div style="padding:8px 12px;color:var(--muted)">No parts</div>';
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
  const q = $('#dlg-filter').value.trim().toLowerCase();
  const hit = s => (s ?? '').toLowerCase().includes(q);
  let shown = 0;
  const unsearched = [];
  for (const e of dlgEntries) {
    const nameHit = !q || hit(e.el.name);
    // Only match part names when the query misses the studio name; a studio
    // that matches by name shows all its parts.
    const partOnly = !!q && !nameHit && !!e.parts && e.parts.some(p => hit(p.name));
    e.row.hidden = !(nameHit || partOnly);
    if (!e.row.hidden) shown++;
    if (!e.box) continue;
    if (q && !nameHit && !e.parts) unsearched.push(e);
    if (fromInput) e.autoExpanded = partOnly;
    const open = !e.row.hidden && (e.expanded || e.autoExpanded);
    e.box.hidden = !open;
    e.expandBtn.textContent = open ? '▾ parts' : '▸ parts';
    for (const { p, row } of e.partRows) row.hidden = partOnly && !hit(p.name);
  }
  if (!dlgNote) return;
  dlgNote.innerHTML = '';
  const say = text => { const span = document.createElement('span'); span.textContent = text; dlgNote.appendChild(span); };
  if (q && !shown) say('No matches.');
  if (unsearched.length) {
    const n = unsearched.length;
    const s = n === 1 ? '' : 's';
    say(`Part names not searched in ${n} part studio${s} (parts not listed yet).`);
    const btn = document.createElement('button');
    btn.textContent = `List parts (${n} API call${s})`;
    btn.title = 'Fetch the part lists of the hidden part studios so their part names can be searched';
    btn.onclick = async () => {
      btn.disabled = true;
      btn.innerHTML = '<span class="spin"></span>';
      const ok = await Promise.all(unsearched.map(loadParts));
      if (ok.some(x => !x)) toast('Some part lists failed to load', true);
      applyDlgFilter(true);
    };
    dlgNote.appendChild(btn);
  }
  dlgNote.hidden = !dlgNote.childNodes.length;
}

async function sendToPrint(el, btn, part = null) {
  btn.disabled = true;
  btn.innerHTML = '<span class="spin"></span>';
  try {
    await api.print({ doc: currentDoc, el, part });
    toast('Sent to Bambu Studio ✓');
  } catch (e) {
    toast('Print failed: ' + e.message, true, 6000);
  } finally {
    btn.disabled = false;
    btn.textContent = part || el.elementType !== 'PARTSTUDIO' ? 'Print' : 'Print all';
  }
}

// ---------- settings ----------

async function openSettings() {
  const s = await api.getSettings();
  $('#set-access').value = s.accessKey;
  $('#set-secret').value = s.secretKey;
  $('#set-bambu').value = s.bambuPath;
  $('#settings').showModal();
}

async function saveSettings() {
  await api.saveSettings({
    accessKey: $('#set-access').value,
    secretKey: $('#set-secret').value,
    bambuPath: $('#set-bambu').value,
  });
  $('#settings').close();
  toast('Settings saved');
  loadDocs(docs.length === 0);
}

// ---------- auto-update ----------

async function checkForUpdate() {
  try {
    const update = await window.__TAURI__.updater.check();
    if (update) {
      toast(`Downloading update v${update.version}…`, false, 5000);
      await update.downloadAndInstall();
      $('#update-text').textContent = `Update v${update.version} installed.`;
      $('#update-banner').style.display = 'block';
    }
  } catch {
    /* no releases yet / offline — never block the app on updates */
  }
}

// ---------- init ----------

$('#refresh').onclick = () => loadDocs(true);
$('#dlg-refresh').onclick = () => openDoc(currentDoc, true);
$('#dlg-close').onclick = () => $('#dlg').close();
$('#dlg-filter').oninput = () => applyDlgFilter(true);
$('#search').oninput = render;
$('#open-settings').onclick = openSettings;
$('#settings-close').onclick = () => $('#settings').close();
$('#settings-save').onclick = saveSettings;
$('#update-restart').onclick = () => window.__TAURI__.process.relaunch();

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
    console.log('selftest', result);
    document.title = result.ok ? 'onslicer [selftest OK]' : 'onslicer [selftest FAILED]';
  }
})();
