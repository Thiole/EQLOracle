// Plain JS, no build step -- Tauri serves this folder as-is. `window.__TAURI__`
// is injected because tauri.conf.json sets `app.withGlobalTauri`.
//
// Two screens: `setup` (first launch, no directory chosen yet) and `main`.
// Inside `main`, a left-nav sidebar switches between modules ("Overview",
// "Combat") without touching the toolbar or which screen is showing --
// module state is a smaller, separate concern from setup/connected state.

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const MAX_FEED_ROWS = 200;
const COMBAT_REFRESH_MS = 3000;

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

function fmtDuration(ms) {
  const total = Math.max(0, Math.round((ms ?? 0) / 1000));
  const m = Math.floor(total / 60);
  const s = total % 60;
  return `${m}:${String(s).padStart(2, '0')}`;
}

// ---------------------------------------------------------------- toolbar / overview

function renderStatus(status, counts) {
  el('tail-file').textContent = status.file ?? '—';
  el('tail-char').textContent = status.character ? `${status.character} @ ${status.server ?? '?'}` : '';
  el('backfill-note').classList.toggle('hidden', !status.backfilling);

  el('stat-total').textContent = counts.total.toLocaleString();
  el('stat-matched').textContent = counts.matched.toLocaleString();
  el('stat-unmatched').textContent = counts.unmatched.toLocaleString();

  const coverable = counts.matched + counts.unmatched;
  const pct = coverable > 0 ? (100 * counts.matched) / coverable : 0;
  el('stat-coverage').textContent = `${pct.toFixed(1)}%`;

  const tbody = document.querySelector('#kind-table tbody');
  tbody.innerHTML = '';
  const entries = Object.entries(counts.by_kind ?? {}).sort((a, b) => b[1] - a[1]);
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

  const conn = el('conn-status');
  if (!status.watching) {
    conn.textContent = 'not connected';
    conn.className = 'status status-idle';
  } else if (status.tail_status === 'missing') {
    conn.textContent = 'file not found — waiting';
    conn.className = 'status status-idle';
  } else if (status.backfilling) {
    conn.textContent = 'replaying history';
    conn.className = 'status status-live';
  } else {
    conn.textContent = 'watching';
    conn.className = 'status status-live';
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

// ---------------------------------------------------------------- module nav

let activeModule = 'overview';

function showModule(name) {
  activeModule = name;
  for (const btn of document.querySelectorAll('#module-nav .nav-item')) {
    btn.classList.toggle('active', btn.dataset.module === name);
  }
  for (const section of document.querySelectorAll('.module')) {
    section.classList.toggle('hidden', section.id !== `module-${name}`);
  }
  if (name === 'combat') refreshCombat();
}

for (const btn of document.querySelectorAll('#module-nav .nav-item')) {
  btn.addEventListener('click', () => showModule(btn.dataset.module));
}

// ---------------------------------------------------------------- combat module

async function refreshCombat() {
  if (activeModule !== 'combat') return;

  const zoneSelect = el('zone-select');
  const encSelect = el('encounter-select');
  const prevZone = zoneSelect.value;
  const prevEnc = encSelect.value;

  const visits = await invoke('list_zone_visits');
  zoneSelect.innerHTML = '';
  const allOpt = document.createElement('option');
  allOpt.value = '';
  allOpt.textContent = `All zones (${visits.reduce((n, v) => n + v.fight_count, 0)} fights)`;
  zoneSelect.appendChild(allOpt);
  for (const v of visits) {
    const opt = document.createElement('option');
    // -1 is the "Unknown" bucket's wire value (see combat::matches_visit)
    // -- distinct from '', which means "no filter" here.
    opt.value = v.index === null ? '-1' : String(v.index);
    opt.textContent = `${v.current ? '● ' : ''}${v.label} (${v.fight_count})`;
    zoneSelect.appendChild(opt);
  }
  if ([...zoneSelect.options].some((o) => o.value === prevZone)) {
    zoneSelect.value = prevZone;
  }

  const zoneVisit = zoneSelect.value === '' ? null : Number(zoneSelect.value);
  const encounters = await invoke('list_encounters', { zoneVisit });
  encSelect.innerHTML = '';
  const aggOpt = document.createElement('option');
  aggOpt.value = '';
  aggOpt.textContent = `Aggregate (${encounters.length} fight${encounters.length === 1 ? '' : 's'})`;
  encSelect.appendChild(aggOpt);
  for (const e of encounters) {
    const opt = document.createElement('option');
    opt.value = String(e.id);
    const tag = e.open ? 'ongoing' : e.slain ? 'kill' : 'reset';
    opt.textContent = `${e.target} — ${fmtDuration(e.duration_ms)} — ${e.total_damage.toLocaleString()} dmg (${tag})`;
    encSelect.appendChild(opt);
  }
  if ([...encSelect.options].some((o) => o.value === prevEnc)) {
    encSelect.value = prevEnc;
  }

  const encounterId = encSelect.value === '' ? null : Number(encSelect.value);
  const summary = await invoke('get_combat_summary', { zoneVisit, encounterId });
  renderCombatSummary(summary);
}

function renderCombatSummary(summary) {
  el('combat-fights').textContent = summary.fight_count.toLocaleString();
  el('combat-damage').textContent = summary.total_damage.toLocaleString();
  el('combat-duration').textContent = fmtDuration(summary.duration_ms);
  el('combat-dps').textContent = summary.dps.toFixed(1);

  const tbody = document.querySelector('#ability-table tbody');
  tbody.innerHTML = '';
  for (const row of summary.abilities) {
    const tr = document.createElement('tr');
    tr.innerHTML = `
      <td>${row.ability}</td>
      <td class="tags">${row.tags.join(' ')}</td>
      <td class="num">${row.hits.toLocaleString()}</td>
      <td class="num">${row.total.toLocaleString()}</td>
      <td class="num">${row.pct.toFixed(1)}</td>
      <td class="num">${row.dps.toFixed(1)}</td>
      <td class="num">${row.hits > 0 ? Math.round(row.total / row.hits).toLocaleString() : 0}</td>
      <td class="num">${row.hits > 0 ? ((100 * row.crits) / row.hits).toFixed(0) : 0}</td>
    `;
    tbody.appendChild(tr);
  }
  el('combat-empty').classList.toggle('hidden', summary.abilities.length > 0);
}

el('zone-select').addEventListener('change', () => {
  el('encounter-select').value = ''; // a new zone invalidates the old fight pick
  refreshCombat();
});
el('encounter-select').addEventListener('change', refreshCombat);

setInterval(refreshCombat, COMBAT_REFRESH_MS);

// ---------------------------------------------------------------- setup / directory

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
    const result = await invoke('set_log_directory', { path });
    renderStatus(result.status, result.counts);
    showScreen('main');
  } catch (e) {
    showError(String(e));
  }
}

el('choose-dir').addEventListener('click', chooseDirectory);
el('change-dir').addEventListener('click', chooseDirectory);

listen('parse-tick', (event) => {
  renderStatus(event.payload.status, event.payload.counts);
  appendFeed(event.payload.recent);
});

listen('parse-error', (event) => {
  showError(String(event.payload));
});

(async () => {
  const result = await invoke('get_status');
  renderStatus(result.status, result.counts);
  showScreen(result.configured ? 'main' : 'setup');
  showModule('overview');
})();
