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

function escapeHtml(s) {
  const div = document.createElement('div');
  div.textContent = s;
  return div.innerHTML;
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
    const others = e.entities.length > 1 ? ` +${e.entities.length - 1} other${e.entities.length > 2 ? 's' : ''}` : '';
    opt.textContent = `${e.target}${others} — ${fmtDuration(e.duration_ms)} — ${e.total_damage.toLocaleString()} dmg (${tag})`;
    encSelect.appendChild(opt);
  }
  if ([...encSelect.options].some((o) => o.value === prevEnc)) {
    encSelect.value = prevEnc;
  }

  const encounterId = encSelect.value === '' ? null : Number(encSelect.value);
  currentZoneVisit = zoneVisit;
  currentEncounterId = encounterId;

  const summary = await invoke('get_combat_summary', { zoneVisit, encounterId });
  renderCombatStats(summary);

  const allies = await invoke('list_allies', { zoneVisit, encounterId });
  renderAllies(allies); // also keeps the expanded ally's detail panel open, if any

  if (encounterId !== null) {
    if (encounterId !== currentTimelineEncounterId) {
      highlightedEntity = null;
      selectedBucketMs = null;
      el('timeline-state').classList.add('hidden');
    }
    await loadTimeline(encounterId);
  } else {
    el('timeline-pane').classList.add('hidden');
    currentTimelineEncounterId = null;
  }
}

function renderCombatStats(summary) {
  el('combat-fights').textContent = summary.fight_count.toLocaleString();
  el('combat-damage').textContent = summary.total_damage.toLocaleString();
  el('combat-duration').textContent = fmtDuration(summary.duration_ms);
  el('combat-dps').textContent = summary.dps.toFixed(1);
}

function abilitySubtableHtml(abilities) {
  if (abilities.length === 0) {
    return '<p class="muted">No abilities recorded for this selection.</p>';
  }
  const rows = abilities
    .map(
      (row) => `
      <tr>
        <td>${escapeHtml(row.ability)}</td>
        <td class="tags">${row.tags.join(' ')}</td>
        <td class="num">${row.hits.toLocaleString()}</td>
        <td class="num">${row.total.toLocaleString()}</td>
        <td class="num">${row.pct.toFixed(1)}</td>
        <td class="num">${row.dps.toFixed(1)}</td>
        <td class="num">${row.hits > 0 ? Math.round(row.total / row.hits).toLocaleString() : 0}</td>
        <td class="num">${row.hits > 0 ? ((100 * row.crits) / row.hits).toFixed(0) : 0}</td>
      </tr>`
    )
    .join('');
  return `
    <table class="ability-subtable">
      <thead>
        <tr>
          <th>ability</th><th>tags</th><th class="num">hits</th><th class="num">total</th>
          <th class="num">%</th><th class="num">dps</th><th class="num">avg</th><th class="num">crit%</th>
        </tr>
      </thead>
      <tbody>${rows}</tbody>
    </table>`;
}

// ---------------------------------------------------------------- allies

let currentZoneVisit = null;
let currentEncounterId = null;
let expandedAlly = null;

// `renderAllies` runs every 3s. It used to rebuild the whole <tbody> from
// scratch every time -- destroying and recreating even an already-open
// detail panel, flashing it back to "Loading..." before repopulating. That
// read as the panel closing and reopening on every tick. Fixed by treating
// this as a live view over changing data rather than a fresh render each
// time: existing row/detail elements are found by name and patched in
// place (`updateAllyRowValues`, `refreshAllyDetail`), and a DOM node is
// only created or removed when an ally actually enters or leaves the
// selection -- never just because a refresh happened.
function renderAllies(allies) {
  const tbody = document.querySelector('#ally-table tbody');
  el('combat-empty').classList.toggle('hidden', allies.length > 0);

  if (expandedAlly !== null && !allies.some((a) => a.name === expandedAlly)) {
    collapseAllyDetail();
  }

  const present = new Set(allies.map((a) => a.name));
  for (const row of [...tbody.querySelectorAll('tr.ally-row')]) {
    if (!present.has(row.dataset.name)) {
      if (row.nextElementSibling?.classList.contains('ally-detail')) {
        row.nextElementSibling.remove();
      }
      row.remove();
    }
  }

  let cursor = tbody.firstChild;
  for (const ally of allies) {
    const row = findAllyRow(ally.name) ?? buildAllyRow(ally.name);
    updateAllyRowValues(row, ally);
    if (cursor !== row) {
      tbody.insertBefore(row, cursor);
    }
    // Advance past this row, and past its detail panel if it has one, so
    // the next ally lands after both rather than splitting them apart.
    cursor = row.nextElementSibling;
    if (cursor?.classList.contains('ally-detail')) {
      cursor = cursor.nextElementSibling;
    }
  }

  if (expandedAlly !== null) {
    refreshAllyDetail(expandedAlly);
  }
}

function findAllyRow(name) {
  for (const row of document.querySelectorAll('#ally-table tr.ally-row')) {
    if (row.dataset.name === name) return row;
  }
  return null;
}

function buildAllyRow(name) {
  const row = document.createElement('tr');
  row.className = 'ally-row';
  row.dataset.name = name;
  row.innerHTML =
    '<td class="ally-name"></td><td class="num ally-total"></td><td class="num ally-pct"></td><td class="num ally-dps"></td><td class="num ally-hits"></td>';
  row.addEventListener('click', () => toggleAllyDetail(name, row));
  return row;
}

function updateAllyRowValues(row, ally) {
  const badge = ally.is_player ? ' <span class="ally-badge">you</span>' : ally.is_pet ? ' <span class="ally-badge">pet</span>' : '';
  row.classList.toggle('expanded', ally.name === expandedAlly);
  row.querySelector('.ally-name').innerHTML = `${escapeHtml(ally.name)}${badge}`;
  row.querySelector('.ally-total').textContent = ally.total.toLocaleString();
  row.querySelector('.ally-pct').textContent = ally.pct.toFixed(1);
  row.querySelector('.ally-dps').textContent = ally.dps.toFixed(1);
  row.querySelector('.ally-hits').textContent = ally.hits.toLocaleString();
}

function toggleAllyDetail(name, row) {
  if (expandedAlly === name) {
    collapseAllyDetail();
    return;
  }
  collapseAllyDetail();

  row.classList.add('expanded');
  expandedAlly = name;

  const detail = document.createElement('tr');
  detail.className = 'ally-detail';
  const cell = document.createElement('td');
  cell.colSpan = 5;
  cell.innerHTML = '<p class="muted">Loading&hellip;</p>';
  detail.appendChild(cell);
  row.after(detail);

  refreshAllyDetail(name); // first real fill-in, replacing "Loading..."
}

function collapseAllyDetail() {
  for (const r of document.querySelectorAll('#ally-table .ally-detail')) r.remove();
  for (const r of document.querySelectorAll('#ally-table .ally-row.expanded')) r.classList.remove('expanded');
  expandedAlly = null;
}

// Patches the open detail panel's numbers in place -- called right after a
// fresh expand and again on every periodic refresh. Must never show the
// "Loading..." placeholder on a refresh of an already-open panel; that's
// exactly what made it look like the panel was closing and reopening.
async function refreshAllyDetail(name) {
  const row = findAllyRow(name);
  const detail = row?.nextElementSibling;
  const cell = detail?.classList.contains('ally-detail') ? detail.querySelector('td') : null;
  if (!cell) return;
  const summary = await invoke('get_combat_summary', { zoneVisit: currentZoneVisit, encounterId: currentEncounterId, actor: name });
  if (expandedAlly === name) {
    cell.innerHTML = abilitySubtableHtml(summary.abilities);
  }
}

// ---------------------------------------------------------------- fight timeline

const SERIES_COLORS = ['#5fb3ff', '#5fd18a', '#e0b34d', '#e0616f', '#b892ff', '#4dd0e1', '#ffb74d', '#81c995'];

let currentTimelineEncounterId = null;
let currentTimelineStartMs = null;
let highlightedEntity = null;
let selectedBucketMs = null;

async function loadTimeline(encounterId) {
  currentTimelineEncounterId = encounterId;
  const dto = await invoke('get_fight_timeline', { encounterId });
  if (!dto || dto.series.length === 0) {
    el('timeline-pane').classList.add('hidden');
    return;
  }
  el('timeline-pane').classList.remove('hidden');
  currentTimelineStartMs = dto.start_ms;
  renderTimelineChart(dto);
}

function renderTimelineChart(dto) {
  const legend = el('timeline-legend');
  const chart = el('timeline-chart');
  legend.innerHTML = '';
  chart.innerHTML = '';

  const globalMax = Math.max(1, ...dto.series.flatMap((s) => s.values));

  dto.series.forEach((s, i) => {
    const color = SERIES_COLORS[i % SERIES_COLORS.length];

    const chip = document.createElement('button');
    chip.type = 'button';
    chip.className = 'legend-chip';
    chip.dataset.entity = s.name;
    chip.style.setProperty('--series-color', color);
    chip.innerHTML = `<span class="swatch"></span>${escapeHtml(s.name)} (${s.total.toLocaleString()})`;
    chip.addEventListener('click', () => {
      highlightedEntity = highlightedEntity === s.name ? null : s.name;
      applyHighlight();
    });
    legend.appendChild(chip);

    const row = document.createElement('div');
    row.className = 'series-row';
    row.dataset.entity = s.name;
    const nameEl = document.createElement('span');
    nameEl.className = 'series-name';
    nameEl.textContent = s.name;
    const bars = document.createElement('div');
    bars.className = 'bars';
    bars.style.setProperty('--series-color', color);
    s.values.forEach((v, bi) => {
      const bar = document.createElement('div');
      bar.className = 'bar';
      bar.style.setProperty('--series-color', color);
      bar.style.height = `${Math.max(4, (v / globalMax) * 100)}%`;
      bar.title = `${v.toLocaleString()} dmg`;
      const bucketMs = dto.buckets[bi] ?? dto.start_ms;
      bar.dataset.ts = String(bucketMs);
      if (selectedBucketMs === bucketMs) bar.classList.add('selected');
      bar.addEventListener('click', () => showStateAt(bucketMs, bar));
      bars.appendChild(bar);
    });
    row.append(nameEl, bars);
    chart.appendChild(row);
  });

  applyHighlight();
}

function applyHighlight() {
  for (const row of document.querySelectorAll('.series-row')) {
    row.classList.toggle('dimmed', highlightedEntity !== null && row.dataset.entity !== highlightedEntity);
  }
  for (const chip of document.querySelectorAll('.legend-chip')) {
    chip.classList.toggle('dimmed', highlightedEntity !== null && chip.dataset.entity !== highlightedEntity);
  }
}

async function showStateAt(tsMs, barEl) {
  selectedBucketMs = tsMs;
  for (const b of document.querySelectorAll('.bar.selected')) b.classList.remove('selected');
  if (barEl) barEl.classList.add('selected');

  const states = await invoke('get_fight_state_at', { encounterId: currentTimelineEncounterId, tsMs });

  const panel = el('timeline-state');
  panel.classList.remove('hidden');
  const into = currentTimelineStartMs !== null ? tsMs - currentTimelineStartMs : 0;
  el('timeline-state-time').textContent = `${fmtDuration(into)} into the fight`;

  const tbody = document.querySelector('#timeline-state-table tbody');
  tbody.innerHTML = '';
  for (const s of states) {
    const tr = document.createElement('tr');
    const badgeClass = `state-badge state-${s.state}${s.observed ? '' : ' inferred'}`;
    tr.innerHTML = `
      <td>${escapeHtml(s.name)}${s.is_player ? ' <span class="muted">(you)</span>' : ''}</td>
      <td><span class="${badgeClass}">${s.state}${s.observed ? '' : ' (inferred)'}</span></td>
      <td class="num">${s.dps.toFixed(1)} dps</td>
    `;
    tbody.appendChild(tr);
  }
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
