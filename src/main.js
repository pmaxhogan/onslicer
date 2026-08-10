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

async function openDoc(d, refresh = false) {
  currentDoc = d;
  $('#dlg-title').textContent = d.name;
  $('#dlg-body').innerHTML = '<div style="padding:20px;text-align:center"><span class="spin"></span></div>';
  if (!$('#dlg').open) $('#dlg').showModal();
  try {
    const data = await api.getElements(d, refresh);
    const body = $('#dlg-body');
    body.innerHTML = '';
    for (const el of data.items) {
      const row = document.createElement('div');
      row.className = 'el' + (el.printable ? '' : ' dim');
      const typeLabel = el.elementType === 'PARTSTUDIO' ? 'part studio'
        : el.elementType === 'BLOB' ? (el.name.match(/\.\w+$/)?.[0].slice(1) || 'file')
        : el.elementType.toLowerCase();
      row.innerHTML = `<span class="type">${esc(typeLabel)}</span><span class="ename">${esc(el.name)}</span>`;
      if (el.elementType === 'PARTSTUDIO') {
        const expand = document.createElement('button');
        expand.className = 'expand';
        expand.textContent = '▸ parts';
        expand.title = 'List individual parts (1 API call, then cached)';
        expand.onclick = () => togglePartList(el, expand, partsBox);
        row.appendChild(expand);
      }
      if (el.printable) {
        const btn = document.createElement('button');
        btn.className = 'primary';
        btn.textContent = el.elementType === 'PARTSTUDIO' ? 'Print all' : 'Print';
        btn.onclick = () => sendToPrint(el, btn);
        row.appendChild(btn);
      }
      body.appendChild(row);
      const partsBox = document.createElement('div');
      partsBox.className = 'parts';
      partsBox.hidden = true;
      body.appendChild(partsBox);
    }
    if (!data.items.length) body.innerHTML = '<div style="padding:20px;color:var(--muted)">Empty document</div>';
  } catch (e) {
    $('#dlg-body').innerHTML = `<div style="padding:20px;color:#e74c3c">${esc(e.message)}</div>`;
  }
}

async function togglePartList(el, expandBtn, box) {
  if (!box.hidden) { box.hidden = true; expandBtn.textContent = '▸ parts'; return; }
  box.hidden = false;
  expandBtn.textContent = '▾ parts';
  if (box.dataset.loaded) return;
  box.innerHTML = '<div style="padding:8px 12px"><span class="spin"></span></div>';
  try {
    const data = await api.getParts(currentDoc, el.id);
    box.innerHTML = '';
    box.dataset.loaded = '1';
    for (const p of data.items) {
      const row = document.createElement('div');
      row.className = 'el';
      row.innerHTML = `<span class="type">${esc(p.bodyType)}</span><span class="ename">${esc(p.name)}</span>`;
      const btn = document.createElement('button');
      btn.className = 'primary';
      btn.textContent = 'Print';
      btn.onclick = () => sendToPrint(el, btn, p);
      row.appendChild(btn);
      box.appendChild(row);
    }
    if (!data.items.length) box.innerHTML = '<div style="padding:8px 12px;color:var(--muted)">No parts</div>';
  } catch (e) {
    box.innerHTML = `<div style="padding:8px 12px;color:#e74c3c">${esc(e.message)}</div>`;
    delete box.dataset.loaded;
  }
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
