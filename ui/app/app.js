// Plain JS, no build step -- Tauri serves this folder as-is. `window.__TAURI__`
// is injected because tauri.conf.json sets `app.withGlobalTauri`.
//
// Two screens: `setup` (first launch, no directory chosen yet) and `main`
// (live feed). Which one shows is decided once at load from `get_status`,
// then `main` is the target of every `set_log_directory` success.

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const MAX_FEED_ROWS = 200;

const el = (id) => document.getElementById(id);
const screens = { setup: el('setup'), main: el('main') };

function showScreen(name) {
  for (const [key, node] of Object.entries(screens)) {
    node.classList.toggle('hidden', key !== name);
  }
}

function showError(message) {
  const banner = el('error-banner');
  banner.textContent = message;
  banner.classList.remove('hidden');
}

function clearError() {
  el('error-banner').classList.add('hidden');
}

function escapeHtml(s) {
  const div = document.createElement('div');
  div.textContent = s;
  return div.innerHTML;
}

function renderSnapshot(snap) {
  el('tail-file').textContent = snap.file ?? '—';
  el('tail-char').textContent = snap.character ? `${snap.character} @ ${snap.server ?? '?'}` : '';

  el('stat-total').textContent = snap.total.toLocaleString();
  el('stat-matched').textContent = snap.matched.toLocaleString();
  el('stat-unmatched').textContent = snap.unmatched.toLocaleString();

  const coverable = snap.matched + snap.unmatched;
  const pct = coverable > 0 ? (100 * snap.matched) / coverable : 0;
  el('stat-coverage').textContent = `${pct.toFixed(1)}%`;

  const tbody = document.querySelector('#kind-table tbody');
  tbody.innerHTML = '';
  const entries = Object.entries(snap.by_kind ?? {}).sort((a, b) => b[1] - a[1]);
  for (const [kind, count] of entries) {
    const tr = document.createElement('tr');
    const kindCell = document.createElement('td');
    kindCell.textContent = kind;
    const countCell = document.createElement('td');
    countCell.className = 'num';
    countCell.textContent = count.toLocaleString();
    tr.append(kindCell, countCell);
    tbody.appendChild(tr);
  }

  const status = el('conn-status');
  if (!snap.watching) {
    status.textContent = 'not connected';
    status.className = 'status status-idle';
  } else if (snap.tail_status === 'missing') {
    status.textContent = 'file not found — waiting';
    status.className = 'status status-idle';
  } else {
    status.textContent = 'watching';
    status.className = 'status status-live';
  }
}

function appendFeed(lines) {
  if (!lines || lines.length === 0) return;
  const list = el('feed-list');
  for (const line of lines) {
    const li = document.createElement('li');
    const kindSpan = document.createElement('span');
    kindSpan.className = 'kind';
    kindSpan.textContent = line.kind;
    const textSpan = document.createElement('span');
    textSpan.className = 'text';
    textSpan.textContent = line.text;
    li.append(kindSpan, textSpan);
    list.prepend(li);
  }
  while (list.children.length > MAX_FEED_ROWS) {
    list.removeChild(list.lastChild);
  }
}

async function chooseDirectory() {
  clearError();
  let path;
  try {
    path = await invoke('pick_log_directory');
  } catch (e) {
    showError(String(e));
    return;
  }
  if (!path) return; // user cancelled

  try {
    const status = await invoke('set_log_directory', { path });
    renderSnapshot(status.snapshot);
    showScreen('main');
  } catch (e) {
    showError(String(e));
  }
}

el('choose-dir').addEventListener('click', chooseDirectory);
el('change-dir').addEventListener('click', chooseDirectory);

listen('parse-tick', (event) => {
  renderSnapshot(event.payload.snapshot);
  appendFeed(event.payload.recent);
});

listen('parse-error', (event) => {
  showError(String(event.payload));
});

(async () => {
  const status = await invoke('get_status');
  renderSnapshot(status.snapshot);
  showScreen(status.configured ? 'main' : 'setup');
})();
