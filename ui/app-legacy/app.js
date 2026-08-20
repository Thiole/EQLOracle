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
  // The counting-up number, not just a static "replaying..." label -- the
  // backend now emits a tick after every ~100k-line chunk instead of one
  // tick at the very end, so this climbs visibly instead of jumping from 0
  // straight to "done" (see tail_worker.rs's BACKFILL_CHUNK_LINES).
  if (status.backfilling) {
    el('backfill-note').textContent = `replaying history… (${counts.total.toLocaleString()} lines)`;
  }

  el('stat-total').textContent = counts.total.toLocaleString();
  el('stat-matched').textContent = counts.matched.toLocaleString();
  el('stat-unmatched').textContent = counts.unmatched.toLocaleString();

  const coverable = counts.matched + counts.unmatched;
  const pct = coverable > 0 ? (100 * counts.matched) / coverable : 0;
  el('stat-coverage').textContent = `${pct.toFixed(1)}%`;
  el('stat-pets').textContent = status.pets_attributed.toLocaleString();

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

// `session_start_ms`/`session_duration_ms` on the backend advance with
// the log's own clock, which keeps ticking during a live idle stretch
// (see `Ingest::tick`'s doc) even with no new lines arriving -- so this
// is polled on an interval (see the bottom of this file), not just
// re-fetched on parse-tick, the same reasoning refreshCombat/
// refreshMonsters already poll instead of relying solely on events.
async function refreshOverviewSession() {
  if (activeModule !== 'overview') return;
  let s;
  try {
    s = await invoke('get_session');
  } catch {
    return;
  }
  if (activeModule !== 'overview') return; // switched tabs mid-fetch

  const sub = el('session-sub');
  if (s.session_start_ms === null) {
    sub.textContent = 'No session yet -- this fills in once something has been parsed.';
  } else {
    sub.textContent = s.afk
      ? 'Currently AFK -- averages below are frozen as of when you last stopped being.'
      : 'Since the log started, or since you last stopped being AFK, whichever is more recent.';
  }

  el('session-duration').textContent = fmtDuration(s.session_duration_ms);
  el('session-plat').textContent = s.platinum_per_hour === null ? '—' : Math.round(s.platinum_per_hour).toLocaleString();
  el('session-xp').textContent = s.xp_pct_per_hour === null ? '—' : `${s.xp_pct_per_hour.toFixed(2)}%`;
  el('session-level').textContent = s.current_level === null ? '—' : s.current_level;
  el('session-progress').textContent = s.progress_pct === null ? '—' : `${s.progress_pct.toFixed(1)}%`;
  el('session-eta').textContent = s.eta_hours === null ? '—' : fmtEta(s.eta_hours);
}

// Hours as a compact "Xh Ym" (or just "Ym" under an hour) -- fmtDuration
// already covers m:ss for a fight's own length, but an ETA in the tens of
// hours reads better without a seconds column nobody needs at that scale.
function fmtEta(hours) {
  const totalMin = Math.max(0, Math.round(hours * 60));
  const h = Math.floor(totalMin / 60);
  const m = totalMin % 60;
  return h > 0 ? `${h}h ${m}m` : `${m}m`;
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
  if (name === 'overview') refreshOverviewSession();
  if (name === 'combat') refreshCombat();
  if (name === 'monsters') refreshMonsters();
  if (name === 'gamedata') refreshGameData();
  if (name === 'settings') refreshSettings();
  if (name === 'debug') showDebugSub(debugSub);
  // Character owns 3 subpages of its own (Character/Gear/AA) -- see
  // showCsSub. Re-dispatches to whichever one was last open rather than
  // always landing back on "Character", the same "you left it where you
  // left it" stance every other module already gets for free.
  if (name === 'character') showCsSub(csSub);
}

// ---------------------------------------------------------------- character subpages

// Gear used to be its own top-level module; folded in here since a build
// is really one thing -- race/classes, the gear that scores against them,
// and the AA spent on them -- not three unrelated screens. 'sheet' is the
// original Character view (race, planner, vitals/stat sheet).
let csSub = 'sheet';

function csShowing(sub) {
  return activeModule === 'character' && csSub === sub;
}

function showCsSub(name) {
  csSub = name;
  for (const btn of document.querySelectorAll('#cs-tabs .gd-tab')) {
    btn.classList.toggle('active', btn.dataset.sub === name);
  }
  for (const section of document.querySelectorAll('.cs-subpage')) {
    section.classList.toggle('hidden', section.id !== `cs-sub-${name}`);
  }
  if (name === 'sheet') refreshCharacter();
  if (name === 'gear') refreshGearPlanner();
  if (name === 'aa') refreshAaLog();
  if (name === 'spellbook') refreshSpellbook();
}

for (const btn of document.querySelectorAll('#cs-tabs .gd-tab')) {
  btn.addEventListener('click', () => showCsSub(btn.dataset.sub));
}

// ---------------------------------------------------------------- cross-module back stack

// Every module already keeps its own drilled-into state in plain globals
// (gdOpen, expandedMonster, gpExpandedSlot, ...) rather than resetting it
// when you switch away -- switching *back* to a module you left already
// shows you where you were, for free. What that alone can't do is get you
// back *across* a jump like Game Data's "open in Combat" link, which
// deliberately lands you on a different module entirely: there's nothing
// to switch "back" to, because you're not returning to your last visit to
// that module, you're trying to undo a specific jump. This stack is for
// exactly that -- a real, growing trail of "here's what you jumped away
// from", each entry a label plus a closure that puts things back.
let navHistory = [];

function navPush(label, restore) {
  navHistory.push({ label, restore });
  renderNavHistory();
}

// Clicking any crumb restores that checkpoint and discards everything
// after it -- standard breadcrumb semantics (jumping back three steps at
// once doesn't leave the two steps in between still on the trail).
function navGoTo(index) {
  const entry = navHistory[index];
  if (!entry) return;
  navHistory.length = index;
  entry.restore();
  renderNavHistory();
}

function renderNavHistory() {
  const bar = el('nav-history');
  if (!navHistory.length) {
    bar.classList.add('hidden');
    bar.innerHTML = '';
    return;
  }
  bar.classList.remove('hidden');
  bar.innerHTML = navHistory.map((h, i) => `<button class="nav-crumb" data-idx="${i}">&larr; ${escapeHtml(h.label)}</button>`).join('<span class="nav-crumb-sep">/</span>');
  bar.onclick = (e) => {
    const btn = e.target.closest('.nav-crumb');
    if (btn) navGoTo(Number(btn.dataset.idx));
  };
}

// The other half of "easily go back": clicking the nav tab you're already
// on resets *that module* to its own default view instead of doing
// nothing (today, re-clicking Game Data while looking at an NPC page just
// re-rendered the exact same NPC page). A second click reading as "start
// over here" is a different, complementary action from the history
// stack's "undo a specific jump" -- both matter, neither substitutes for
// the other.
function resetModuleToDefault(name) {
  if (name === 'gamedata') {
    gdOpen = null;
    gdZoneEncExpanded = null;
    el('gd-page').classList.add('hidden');
    el('gd-list').classList.remove('hidden');
    renderGdList();
  } else if (name === 'monsters') {
    expandedMonster = null;
    refreshMonsters();
  } else if (name === 'combat') {
    collapseAllyDetail();
  } else if (name === 'character' && csSub === 'gear') {
    gpExpandedSlot = null;
    gpDetailItem = null;
    renderGpDoll();
    renderGpDetail();
  }
  // overview/debug, and the Character/AA subpages, have no drilled-into
  // state to reset.
}

// A cross-link to a zone/item/NPC, anywhere in the app -- jumps to the
// Game Data module, on the right tab, with that page open.
// showModule('gamedata') triggers refreshGameData() itself; gdPendingOpen
// is how that call (async, already in flight by the time this returns)
// knows what to land on once its data's loaded, rather than this function
// needing to await it.
function gdGoTo(kind, id) {
  gdPendingOpen = { kind, id };
  showModule('gamedata');
}

// One delegated listener for every `.gd-link` the app ever renders (Gear
// Planner's detail panel and alt list, and Game Data's own pages
// cross-linking each other) rather than wiring a click handler per render
// site -- a link works the same way no matter which module put it there.
document.addEventListener('click', (e) => {
  const link = e.target.closest('.gd-link');
  if (link?.dataset.kind && link.dataset.id) gdGoTo(link.dataset.kind, link.dataset.id);
});

for (const btn of document.querySelectorAll('#module-nav .nav-item')) {
  btn.addEventListener('click', () => {
    const name = btn.dataset.module;
    if (name === activeModule) {
      resetModuleToDefault(name);
    } else {
      showModule(name);
    }
  });
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
  // Only a specific fight has one target to look up history for -- the
  // aggregate selection spans however many different mobs, so "past
  // parses vs X" has no single X to show.
  const selectedEncounter = encounterId === null ? null : encounters.find((e) => e.id === encounterId);
  currentEncounterTarget = selectedEncounter ? selectedEncounter.target : null;

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
  await refreshMobHistory();
}

// Jumps the Combat module's own zone/encounter pickers to one specific
// fight -- what "bring up the encounter" means from the ally-detail
// drill-down (see the encounter rows built in encountersHtml). Two
// sequential refreshCombat() calls, not one: the first commits the zone
// selection and, as a side effect of refreshCombat rebuilding
// #encounter-select's options for whatever zone is now selected, makes
// this fight a valid option to select at all -- setting encSelect.value
// to an id that isn't yet among its current <option>s would silently fail
// to stick. Only the second call's prevEnc capture (refreshCombat's own
// mechanism for surviving an option rebuild) actually lands the encounter
// selection.
async function jumpToEncounter(zoneVisit, encounterId) {
  const zoneSelect = el('zone-select');
  const encSelect = el('encounter-select');
  zoneSelect.value = zoneVisit === null ? '-1' : String(zoneVisit);
  await refreshCombat();
  encSelect.value = String(encounterId);
  await refreshCombat();
  collapseAllyDetail();
  zoneSelect.scrollIntoView({ behavior: 'smooth', block: 'center' });
}

// Past parses against the currently selected fight's target -- see
// history.rs's module doc for why this reads from parse_history.jsonl
// rather than anything in Ingest: it's meant to outlive both the live
// store's eviction and app restarts.
async function refreshMobHistory() {
  const pane = el('history-pane');
  if (!currentEncounterTarget) {
    pane.classList.add('hidden');
    return;
  }
  pane.classList.remove('hidden');
  el('history-target').textContent = currentEncounterTarget;

  const confirmedOnly = el('history-confirmed-only').checked;
  const [records, loadouts] = await Promise.all([
    invoke('get_mob_history', { target: currentEncounterTarget, confirmedOnly }),
    invoke('get_loadout_summary', { target: currentEncounterTarget, confirmedOnly }),
  ]);

  renderBestDps(records);
  renderLoadoutSummary(loadouts);

  const tbody = document.querySelector('#history-table tbody');
  tbody.innerHTML = '';
  el('history-empty').classList.toggle('hidden', records.length > 0);
  for (const r of records) {
    const tr = document.createElement('tr');
    // score_ratio is None (JSON null) whenever there wasn't yet a baseline
    // to score against -- see ParseRecord's doc comment. Shown as "—", not
    // 0%, so "no data" is never confused with "ran at zero DPS".
    const ratioText = r.score_ratio == null ? '—' : `${(r.score_ratio * 100).toFixed(0)}%`;
    tr.innerHTML = `
      <td>${escapeHtml(new Date(r.start_ms).toLocaleString())}</td>
      <td>${escapeHtml(r.zone)}</td>
      <td>${fmtLoadout(r.loadout)}</td>
      <td class="num">${fmtDuration(r.duration_ms)}</td>
      <td class="num">${r.player_dps.toFixed(1)}</td>
      <td class="num">${ratioText}</td>
      <td>${r.confirmed_kill ? 'kill' : 'reset'}</td>
    `;
    tbody.appendChild(tr);
  }
}

// "Wizard / Enchanter" for a real combination, an em-dash (not blank) for
// an empty one -- no recognised cast landed within the recency window as
// of that fight's start, a real answer in its own right (see
// ParseRecord::loadout's doc), not a rendering gap.
function fmtLoadout(loadout) {
  return loadout && loadout.length > 0 ? escapeHtml(loadout.join(' / ')) : '—';
}

function renderLoadoutSummary(loadouts) {
  const tbody = document.querySelector('#loadout-table tbody');
  tbody.innerHTML = '';
  el('loadout-empty').classList.toggle('hidden', loadouts.length > 0);
  for (const l of loadouts) {
    const tr = document.createElement('tr');
    const ratioText = l.avg_score_ratio == null ? '—' : `${(l.avg_score_ratio * 100).toFixed(0)}%`;
    tr.innerHTML = `
      <td>${fmtLoadout(l.loadout)}</td>
      <td class="num">${l.fights.toLocaleString()}</td>
      <td class="num">${l.confirmed_kills.toLocaleString()}</td>
      <td class="num">${l.avg_dps.toFixed(1)}</td>
      <td class="num">${ratioText}</td>
    `;
    tbody.appendChild(tr);
  }
}

// Derived client-side from the same `records` the table below it renders
// (already scoped to `currentEncounterTarget` and the confirmed-only
// toggle), rather than a separate backend query -- so the headline number
// can never disagree with what's visibly sitting in the table under it.
function renderBestDps(records) {
  const dpsEl = el('history-best-dps');
  const subEl = el('history-best-dps-sub');
  if (records.length === 0) {
    dpsEl.textContent = '—';
    subEl.textContent = '';
    return;
  }
  const best = records.reduce((a, b) => (b.player_dps > a.player_dps ? b : a));
  dpsEl.textContent = best.player_dps.toFixed(1);
  // .textContent, not innerHTML -- no escaping needed, same as every other
  // plain-text assignment in this file (escapeHtml is only for the
  // innerHTML template strings elsewhere).
  subEl.textContent = `${best.zone || 'unknown zone'} — ${new Date(best.start_ms).toLocaleDateString()}`;
}

function renderCombatStats(summary) {
  el('combat-fights').textContent = summary.fight_count.toLocaleString();
  el('combat-duration').textContent = fmtDuration(summary.duration_ms);
  // "team" vs "incoming" are deliberately separate stats, not one total --
  // a number that mixes damage dealt and damage taken says nothing about
  // either. See combat::summarize's doc comment.
  el('combat-damage').textContent = summary.total_damage.toLocaleString();
  el('combat-dps').textContent = summary.dps.toFixed(1);
  el('combat-enemy-damage').textContent = summary.enemy_damage.toLocaleString();
  el('combat-enemy-dps').textContent = summary.enemy_dps.toFixed(1);
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

// Every spell cast, not just the ones that dealt damage -- a buff, a CC
// spell, or a resisted/interrupted attempt has no row in the damage
// breakdown above at all, so without this it's invisible. Separate table,
// not merged into abilitySubtableHtml's: a cast row counts *attempts*, a
// damage row counts *landed hits* -- for a DoT those are different numbers
// for the very same spell, and blending them would misrepresent both.
function castSubtableHtml(casts) {
  if (casts.length === 0) {
    return '<p class="muted">No casts recorded for this selection.</p>';
  }
  const rows = casts
    .map(
      (row) => `
      <tr>
        <td>${escapeHtml(row.spell)}</td>
        <td class="num">${row.attempts.toLocaleString()}</td>
        <td class="num">${row.landed.toLocaleString()}</td>
        <td class="num">${row.resisted.toLocaleString()}</td>
        <td class="num">${row.interrupted.toLocaleString()}</td>
        <td class="num">${row.fizzled.toLocaleString()}</td>
        <td class="num">${row.unconfirmed.toLocaleString()}</td>
      </tr>`
    )
    .join('');
  return `
    <table class="ability-subtable">
      <thead>
        <tr>
          <th>spell</th><th class="num">attempts</th><th class="num">landed</th>
          <th class="num">resisted</th><th class="num">interrupted</th><th class="num">fizzled</th>
          <th class="num">unconfirmed</th>
        </tr>
      </thead>
      <tbody>${rows}</tbody>
    </table>`;
}

// ---------------------------------------------------------------- allies

let currentZoneVisit = null;
let currentEncounterId = null;
let currentEncounterTarget = null;
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
    refreshAllyClasses(row, ally.name); // fire-and-forget; see its own doc comment
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
  // .ally-classes is its own cell, built once here and filled in by
  // refreshAllyClasses -- NOT part of updateAllyRowValues's innerHTML
  // rewrite (that runs every 3s refresh and would wipe an async fill-in
  // before it could ever show, the same trap renderAllies's own doc
  // comment describes for the detail panel).
  row.innerHTML =
    '<td class="ally-name"></td><td class="ally-classes muted">&hellip;</td><td class="num ally-total"></td><td class="num ally-pct"></td><td class="num ally-dps"></td><td class="num ally-hits"></td><td class="num ally-crit"></td>';
  row.addEventListener('click', () => toggleAllyDetail(name, row));
  return row;
}

// Every class configuration seen for one ally, from spell casts recognised
// against the wiki-scraped spell/class lookup, grouped by zone visit -- see
// eqlp_session::classdetect's doc for what this can and can't promise
// (lag on a visit that ends before unambiguous evidence lands; ambiguous
// evidence from spells shared across classes only reinforcing a candidate
// already confirmed that same visit). The row cell shows only the
// *dominant* configuration (the one used in the most zone visits) plus a
// "+N configs" hint when there's more than one -- the full list, including
// an occasional loadout that's real but rare, lives in the expanded detail
// panel (see refreshAllyDetail) rather than being crowded out of the
// summary row entirely, which is the exact failure this replaced.
// Fire-and-forget from renderAllies's loop rather than awaited there, so
// one slow lookup can't stall the whole table refresh.
async function refreshAllyClasses(row, name) {
  let data;
  try {
    data = await invoke('get_class_configurations', { name });
  } catch {
    return; // don't clobber a previous good value with a transient error
  }
  const cell = row.querySelector('.ally-classes');
  if (!cell) return;
  const { configurations, unresolved_visits } = data;
  if (configurations.length === 0) {
    cell.textContent = '—';
    cell.title = '';
    return;
  }
  const [dominant, ...rest] = configurations;
  const more = rest.length > 0 ? ` <span class="muted">(+${rest.length} config${rest.length === 1 ? '' : 's'})</span>` : '';
  cell.innerHTML = `${escapeHtml(dominant.classes.join(' / '))}${more}`;
  cell.title = configurations
    .map((c) => `${c.classes.join(' / ')} -- ${c.zone_visits} zone visit${c.zone_visits === 1 ? '' : 's'}${levelSuffix(c.level_range)}`)
    .join('\n');
}

// `(min, max)` -> "10–50" (or "10" when they're equal), "" when there's no
// level evidence at all -- see ClassConfigurationDto::level_range's doc
// for why this is a range rather than one number.
function levelRangeText(range) {
  if (!range) return '';
  const [lo, hi] = range;
  return lo === hi ? String(lo) : `${lo}–${hi}`;
}

// Same, wrapped for inline use next to other text (the ally-row tooltip).
function levelSuffix(range) {
  const text = levelRangeText(range);
  return text ? ` (level ${text})` : '';
}

function updateAllyRowValues(row, ally) {
  // `is_player` (Kind::Player) means "a confirmed player" -- true for the
  // log owner *and* for any other ally proven by chat or by damaging the
  // same target as you (see Ingest::note_shared_target). The badge text
  // needs the narrower check: only the literal log owner is ever "you";
  // an earlier version showed the "you" badge on every confirmed ally
  // instead, since it read is_player alone.
  const badge =
    ally.name === 'You'
      ? ' <span class="ally-badge">you</span>'
      : ally.is_player
        ? ' <span class="ally-badge">ally</span>'
        : ally.is_pet
          ? ' <span class="ally-badge">pet</span>'
          : '';
  row.classList.toggle('expanded', ally.name === expandedAlly);
  const nameCell = row.querySelector('.ally-name');
  nameCell.innerHTML = `${escapeHtml(ally.name)}${badge}`;
  // Player/pet are confirmed by the log's own markers; everyone else in
  // this list is an unspoken name the log gives no ownership signal for --
  // could be a real groupmate, so it's left untinted rather than guessed
  // at either way.
  nameCell.classList.toggle('entity-ally', ally.is_player || ally.is_pet);
  row.querySelector('.ally-total').textContent = ally.total.toLocaleString();
  row.querySelector('.ally-pct').textContent = ally.pct.toFixed(1);
  row.querySelector('.ally-dps').textContent = ally.dps.toFixed(1);
  row.querySelector('.ally-hits').textContent = ally.hits.toLocaleString();
  row.querySelector('.ally-crit').textContent = ally.crit_pct.toFixed(0);
}

// Which configuration (JSON-stringified classes array, matching a
// data-config attribute) and which of its zone visits (JSON-stringified
// index -- a number or null) are expanded inside the currently-open ally
// panel. Reset whenever the panel itself closes or switches ally, same as
// expandedAlly -- there is only ever one ally panel open at a time, so
// these don't need to be keyed per-ally.
let expandedConfigKey = null;
let expandedConfigVisitKey = null;

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
  cell.colSpan = 6;
  cell.innerHTML = '<p class="muted">Loading&hellip;</p>';
  detail.appendChild(cell);
  row.after(detail);

  refreshAllyDetail(name); // first real fill-in, replacing "Loading..."
}

function collapseAllyDetail() {
  for (const r of document.querySelectorAll('#ally-table .ally-detail')) r.remove();
  for (const r of document.querySelectorAll('#ally-table .ally-row.expanded')) r.classList.remove('expanded');
  expandedAlly = null;
  expandedConfigKey = null;
  expandedConfigVisitKey = null;
}

// Patches the open detail panel's numbers in place -- called right after a
// fresh expand and again on every periodic refresh. Must never show the
// "Loading..." placeholder on a refresh of an already-open panel; that's
// exactly what made it look like the panel was closing and reopening.
//
// Fetches one extra level for each thing currently expanded
// (expandedConfigKey's zone visits, expandedConfigVisitKey's encounters)
// so the whole configurations -> zone visits -> encounters drill-down
// survives a periodic refresh instead of collapsing back to the top every
// 3s -- the same reason expandedAlly itself is a module-level variable
// rather than local state.
async function refreshAllyDetail(name) {
  const row = findAllyRow(name);
  const detail = row?.nextElementSibling;
  const cell = detail?.classList.contains('ally-detail') ? detail.querySelector('td') : null;
  if (!cell) return;

  // Each drill-down level is its own try/catch, deliberately: a failure
  // fetching zone visits or encounters must not take down the whole panel
  // (summary/damage/casts have nothing to do with it) or leave the click
  // handler unassigned -- either of those would make the panel look like
  // it silently stopped responding to clicks, which is exactly the
  // failure this replaced. Errors are surfaced inline, not swallowed, so
  // a real backend problem is visible instead of indistinguishable from
  // "nothing happened".
  let summary, configData;
  try {
    [summary, configData] = await Promise.all([
      invoke('get_combat_summary', { zoneVisit: currentZoneVisit, encounterId: currentEncounterId, actor: name }),
      invoke('get_class_configurations', { name }),
    ]);
  } catch (err) {
    if (expandedAlly === name) cell.innerHTML = `<p class="muted">Couldn't load this ally's detail: ${escapeHtml(String(err))}</p>`;
    return;
  }

  let visits = null;
  let visitsError = null;
  if (expandedConfigKey !== null) {
    const match = configData.configurations.find((c) => JSON.stringify(c.classes) === expandedConfigKey);
    if (match) {
      try {
        visits = await invoke('get_configuration_zone_visits', { name, classes: match.classes });
      } catch (err) {
        visitsError = String(err);
      }
    } else {
      expandedConfigKey = null; // the configuration this pointed at no longer exists (e.g. reconciled away)
      expandedConfigVisitKey = null;
    }
  }
  let encounters = null;
  let encountersError = null;
  if (visits !== null && expandedConfigVisitKey !== null) {
    const visitMatch = visits.find((v) => JSON.stringify(v.index) === expandedConfigVisitKey);
    if (visitMatch) {
      try {
        encounters = await invoke('list_encounters', { zoneVisit: visitMatch.index === null ? -1 : visitMatch.index });
      } catch (err) {
        encountersError = String(err);
      }
    } else {
      expandedConfigVisitKey = null;
    }
  }

  if (expandedAlly !== name) return; // collapsed (or switched ally) while these awaits were in flight

  cell.innerHTML = `
    <h4>configurations</h4>
    ${classConfigurationsHtml(configData, visits, encounters, visitsError, encountersError)}
    <h4>damage</h4>
    ${abilitySubtableHtml(summary.abilities)}
    <h4>casts</h4>
    ${castSubtableHtml(summary.casts)}
  `;
  // Delegated, not per-row: cell.innerHTML above just threw away any
  // previous listeners, and re-attaching per row on every refresh would
  // leak one set per refresh. Re-assigning .onclick (not addEventListener)
  // replaces the old handler outright instead of stacking a new one. Set
  // unconditionally, after every render (including an error one above
  // returns early and skips it -- intentional: nothing clickable exists
  // in that case).
  cell.onclick = (e) => {
    const encounterRow = e.target.closest('tr[data-encounter]');
    if (encounterRow) {
      // The zone visit these encounters belong to isn't on the row itself
      // (see encountersHtml's doc) -- it's whichever visit is currently
      // expanded, recovered here rather than threaded through three levels
      // of render functions for one click handler's sake.
      const zoneVisit = expandedConfigVisitKey === null ? null : JSON.parse(expandedConfigVisitKey);
      jumpToEncounter(zoneVisit, Number(encounterRow.dataset.encounter));
      return;
    }
    const visitRow = e.target.closest('tr[data-visit]');
    if (visitRow) {
      const key = decodeURIComponent(visitRow.dataset.visit);
      expandedConfigVisitKey = expandedConfigVisitKey === key ? null : key;
      refreshAllyDetail(name);
      return;
    }
    const configRow = e.target.closest('tr[data-config]');
    if (configRow) {
      const key = decodeURIComponent(configRow.dataset.config);
      const opening = expandedConfigKey !== key;
      expandedConfigKey = opening ? key : null;
      expandedConfigVisitKey = null; // switching (or closing) a configuration always collapses its visit drill-down
      refreshAllyDetail(name);
    }
  };
}

// The middle layer between the ally row's one-line summary and the
// spell-by-spell breakdown below it: every distinct class configuration
// this entity has confirmed, one row per zone-visit-count, most common
// first, reconciled against the game's fixed-3-classes rule (see
// ClassConfigurationsDto's doc -- a 1- or 2-class row never appears here,
// it's either folded into a real 3-class configuration or counted in
// unresolved_visits). Deliberately not collapsed to just the dominant
// configuration, since an occasional loadout (kept for one specific
// fight, say) is exactly the thing a single "current" value used to hide.
// Click a configuration row to drill into its zone visits; click a zone
// visit row to drill into its encounters -- `visits`/`encounters` are the
// already-fetched data for whichever one is currently expanded, or null.
function classConfigurationsHtml(configData, visits, encounters, visitsError, encountersError) {
  const { configurations, unresolved_visits } = configData;
  if (configurations.length === 0) {
    return '<p class="muted">No recognised casts yet.</p>';
  }
  const rows = configurations
    .map((c) => {
      const key = JSON.stringify(c.classes);
      const isOpen = expandedConfigKey === key;
      // Note: encodeURIComponent, not escapeHtml, for the attribute value --
      // escapeHtml (this file's own div/textContent trick) only escapes
      // what's unsafe in HTML *text* (&, <, >), not the double quotes an
      // *attribute* value needs escaped, and `key` is a JSON string full of
      // them. encodeURIComponent never produces ", <, >, or &, so it's safe
      // here regardless; decoded back with decodeURIComponent below.
      const row = `<tr data-config="${encodeURIComponent(key)}" class="config-row${isOpen ? ' expanded' : ''}"><td>${escapeHtml(c.classes.join(' / '))}</td><td class="num">${c.zone_visits.toLocaleString()}</td><td class="muted">${escapeHtml(levelRangeText(c.level_range)) || '—'}</td></tr>`;
      const drill = isOpen ? `<tr class="config-detail"><td colspan="3">${zoneVisitsHtml(visits, encounters, visitsError, encountersError)}</td></tr>` : '';
      return row + drill;
    })
    .join('');
  const unresolvedNote =
    unresolved_visits > 0
      ? `<p class="muted">${unresolved_visits.toLocaleString()} zone visit${unresolved_visits === 1 ? '' : 's'} had incomplete class evidence, not yet resolved to one of the configurations above.</p>`
      : '';
  return `
    <table class="ability-subtable">
      <thead><tr><th>configuration</th><th class="num">zone visits</th><th>level</th></tr></thead>
      <tbody>${rows}</tbody>
    </table>
    ${unresolvedNote}`;
}

// A configuration's own zone visits (label + fight count), each clickable
// to drill into `encounters` -- the fights within whichever one is
// currently expanded (expandedConfigVisitKey), or null if none is.
function zoneVisitsHtml(visits, encounters, visitsError, encountersError) {
  if (visitsError) {
    return `<p class="muted">Couldn't load zone visits: ${escapeHtml(visitsError)}</p>`;
  }
  if (visits === null) {
    return '<p class="muted">Loading&hellip;</p>';
  }
  if (visits.length === 0) {
    return '<p class="muted">No zone visits.</p>';
  }
  const rows = visits
    .map((v) => {
      const key = JSON.stringify(v.index);
      const isOpen = expandedConfigVisitKey === key;
      // encodeURIComponent, not escapeHtml, for the same reason as
      // data-config above -- this key is JSON too (v.index can be `null`
      // or a number, but JSON.stringify still wraps it, and future-proofing
      // against a string index costs nothing).
      const row = `<tr data-visit="${encodeURIComponent(key)}" class="visit-row${isOpen ? ' expanded' : ''}"><td>${v.current ? '● ' : ''}${escapeHtml(v.label)}</td><td class="num">${v.fight_count.toLocaleString()}</td></tr>`;
      const drill = isOpen ? `<tr class="visit-detail"><td colspan="2">${encountersHtml(encounters, encountersError)}</td></tr>` : '';
      return row + drill;
    })
    .join('');
  return `
    <table class="ability-subtable">
      <thead><tr><th>zone visit</th><th class="num">fights</th></tr></thead>
      <tbody>${rows}</tbody>
    </table>`;
}

// One zone visit's own encounters -- the finest level of this drill-down,
// no further expansion from here (the ally-expand panel's own damage/casts
// tables already cover per-ability detail for whatever selection is
// active elsewhere in the Combat module).
function encountersHtml(encounters, encountersError) {
  if (encountersError) {
    return `<p class="muted">Couldn't load encounters: ${escapeHtml(encountersError)}</p>`;
  }
  if (encounters === null) {
    return '<p class="muted">Loading&hellip;</p>';
  }
  if (encounters.length === 0) {
    return '<p class="muted">No encounters.</p>';
  }
  const rows = encounters
    .map((e) => {
      const tag = e.open ? 'ongoing' : e.slain ? 'kill' : 'reset';
      const others = e.entities.length > 1 ? ` +${e.entities.length - 1} other${e.entities.length > 2 ? 's' : ''}` : '';
      // data-encounter, not an href/button: clicking jumps the whole Combat
      // module's own zone/encounter pickers to this fight (see
      // jumpToEncounter) -- the encounter id alone is enough, the click
      // handler recovers which zone visit it's in from
      // expandedConfigVisitKey rather than needing it threaded through here.
      return `<tr data-encounter="${e.id}"><td>${escapeHtml(e.target)}${others}</td><td class="num">${fmtDuration(e.duration_ms)}</td><td class="num">${e.dps.toFixed(1)}</td><td class="muted">${tag}</td></tr>`;
    })
    .join('');
  return `
    <table class="ability-subtable">
      <thead><tr><th>target</th><th class="num">duration</th><th class="num">dps</th><th>result</th></tr></thead>
      <tbody>${rows}</tbody>
    </table>`;
}

// ---------------------------------------------------------------- monsters module

let expandedMonster = null;

async function refreshMonsters() {
  if (activeModule !== 'monsters') return;
  const mobs = await invoke('list_mobs');
  renderMonsters(mobs);
}

// Unlike renderAllies, this rebuilds the whole <tbody> every call rather
// than patching rows in place: a mob's loot breakdown arrives fully formed
// in the same `list_mobs` response used to draw the row, with no separate
// async drill-down call the way an ally's ability breakdown needs (see
// refreshAllyDetail) -- so there's no "Loading..." placeholder a rebuild
// could flash over, which is the failure mode renderAllies's doc comment
// is about.
function renderMonsters(mobs) {
  el('monsters-tracked').textContent = mobs.length.toLocaleString();
  el('monsters-kills').textContent = mobs.reduce((n, m) => n + m.kills, 0).toLocaleString();
  el('monsters-items').textContent = mobs.reduce((n, m) => n + lootTotal(m), 0).toLocaleString();

  el('monsters-empty').classList.toggle('hidden', mobs.length > 0);
  if (expandedMonster !== null && !mobs.some((m) => m.name === expandedMonster)) {
    expandedMonster = null;
  }

  const tbody = document.querySelector('#monster-table tbody');
  tbody.innerHTML = '';
  for (const mob of mobs) {
    const row = document.createElement('tr');
    row.className = 'monster-row';
    row.classList.toggle('expanded', mob.name === expandedMonster);
    // "known" -- the wiki scrape recorded a drop table for this mob, so
    // `mob.loot` below is the *complete* known-possible-drops list, not
    // just whatever's been looted so far. See MobDto::known's doc comment.
    const badge = mob.known ? ' <span class="ally-badge">known</span>' : '';
    // `null` (not `0`) whenever no kill of this mob has a matched XP
    // gain -- see MobDto::avg_xp_pct's own doc for why that's a different
    // fact than "you got 0% xp", and rendered as an em-dash rather than a
    // number for the same reason `lootTableHtml`'s "not yet gotten" rows
    // use one instead of a literal 0.
    const xpCell = mob.avg_xp_pct === null ? '—' : `${mob.avg_xp_pct.toFixed(2)}%`;
    row.innerHTML = `
      <td>${escapeHtml(mob.name)}${badge}</td>
      <td class="num">${mob.kills.toLocaleString()}</td>
      <td class="num">${mob.pulls.toLocaleString()}</td>
      <td class="num">${lootTotal(mob).toLocaleString()}</td>
      <td class="num">${xpCell}</td>
    `;
    row.addEventListener('click', () => {
      expandedMonster = expandedMonster === mob.name ? null : mob.name;
      renderMonsters(mobs);
    });
    tbody.appendChild(row);

    if (mob.name === expandedMonster) {
      const detail = document.createElement('tr');
      detail.className = 'monster-detail';
      const cell = document.createElement('td');
      cell.colSpan = 5;
      cell.innerHTML = lootTableHtml(mob);
      detail.appendChild(cell);
      tbody.appendChild(detail);
    }
  }
}

function lootTotal(mob) {
  return mob.loot.reduce((n, r) => n + r.count, 0);
}

// Same .gd-link the Game Data module renders everywhere else -- one
// delegated document-level click handler (see gdGoTo's doc) already
// covers it, so this row's own click-to-expand handler (bound to the
// summary <tr>, not this detail cell) never has to know this exists.
function gdNpcLinkHtml(mobName) {
  return `<span class="gd-link" data-kind="npc" data-id="${escapeHtml(mobName)}">View full NPC page &rarr;</span>`;
}

function lootTableHtml(mob) {
  const { loot, known } = mob;
  const npcLink = `<p>${gdNpcLinkHtml(mob.name)}</p>`;
  if (loot.length === 0) {
    return `<p class="muted">Nothing looted from this mob yet, and the wiki records no known drops for it.</p>${npcLink}`;
  }
  // Sorted gotten-first by the backend (see MobDto::loot's doc), so a
  // not-yet-gotten row (count 0) only ever trails a gotten one -- rendering
  // it dimmer, with an em-dash instead of "0", reads as "not yet" rather
  // than "gotten zero of these".
  const rows = loot
    .map((r) => {
      const gotten = r.count > 0;
      return `<tr class="${gotten ? '' : 'loot-ungotten'}"><td>${escapeHtml(r.item)}</td><td class="num">${gotten ? r.count.toLocaleString() : '—'}</td></tr>`;
    })
    .join('');
  const summary = known
    ? `<p class="muted">${loot.filter((r) => r.count > 0).length} of ${loot.length} known drops obtained</p>`
    : '<p class="muted">The wiki records no known drop table for this mob -- showing only what’s actually been looted.</p>';
  return `
    ${summary}
    <table class="ability-subtable">
      <thead><tr><th>item</th><th class="num">count</th></tr></thead>
      <tbody>${rows}</tbody>
    </table>
    ${npcLink}`;
}

// ---------------------------------------------------------------- game data module

// Five wiki-derived datasets (Zones/Items/NPCs/AAs/Spells), cross-linked,
// structured as list -> full page -> back, the same idiom regardless of
// which of the five you're in. All static/baked-into-the-binary data
// (zonedata.rs/itemdata.rs/npcdata.rs/aadata.rs/spelldata.rs), so fetched
// once on first open and cached for the rest of the session -- there's
// nothing that would make it stale. Loaded together (Promise.all below),
// not lazily per-tab, precisely so cross-links are never "only clickable
// if you've already opened that tab" -- an NPC's known-loot list can link
// to an Item page, and a Zone's notable-NPCs list can link an NPC page,
// from the very first click.
let gdData = { zones: null, items: null, npcs: null, aas: null, spells: null, spellEffects: null };
let gdCategory = 'zones';
let gdOpen = null; // { kind: 'zone'|'item'|'npc', key } or null (list view)
let gdFilter = '';
// Set by gdGoTo right before showModule('gamedata') triggers this reload --
// see gdGoTo's own doc for why a pending value instead of an await.
let gdPendingOpen = null;
// Bumped every gdOpenPage call; fillItemLootHistory captures its own value
// and checks it again once the fetch resolves, so a slow response for a
// page you've since navigated away from can't patch content that now
// belongs to a different item (see fillItemLootHistory's own doc).
let gdOpenToken = 0;

// Items/NPCs are thousands of rows -- rendering all of them into the DOM
// unfiltered would be real jank for no benefit before you've typed
// anything. Zones (117) never actually hits this.
const GD_ROW_CAP = 300;

// The one place the three-dataset fetch actually happens -- both the Game
// Data module itself and the Gear Planner call this, not just the former.
// gdZoneOrMobLink/gdLinkOrText only render a real link once the relevant
// dataset is in gdData; Gear Planner's own detail panel and alt list use
// those same helpers for their drop-source text, so *its* links would sit
// dead as plain text for the entire session if this only ever loaded once
// someone happened to visit Game Data first. Cached via gdLoadPromise, not
// just gdData's own null check, so two callers racing to open at once
// (plausible: Gear Planner kicks this off in the background right as you
// also click over to Game Data) share one fetch instead of firing it
// twice.
let gdLoadPromise = null;
function ensureGameData() {
  if (gdData.zones !== null) return Promise.resolve();
  if (gdLoadPromise) return gdLoadPromise;
  gdLoadPromise = Promise.all([
    invoke('list_zones'),
    invoke('list_gear_items', { classes: [], slot: null, maxEra: null }),
    invoke('list_npcs'),
    invoke('list_aa'),
    invoke('list_spells'),
    invoke('list_spell_effects'),
  ]).then(([zones, items, npcs, aas, spells, spellEffectsList]) => {
    // Keyed by spell id for O(1) lookup per row -- see spellEffectsFor.
    const spellEffects = {};
    for (const e of spellEffectsList) spellEffects[e.id] = e;
    gdData = { zones, items, npcs, aas, spells, spellEffects };
  });
  return gdLoadPromise;
}

async function refreshGameData() {
  if (activeModule !== 'gamedata') return;
  if (gdData.zones === null) {
    el('gd-empty').textContent = 'Loading game data…';
    el('gd-empty').classList.remove('hidden');
    await ensureGameData();
  }
  if (activeModule !== 'gamedata') return; // switched tabs mid-fetch

  if (gdPendingOpen !== null) {
    const { kind, id } = gdPendingOpen;
    gdPendingOpen = null;
    gdCategory = kind;
    gdOpenPage(kind, id);
    return;
  }
  renderGdTabs();
  if (gdOpen) {
    renderGdShell();
  } else {
    renderGdList();
  }
}

function renderGdTabs() {
  for (const btn of document.querySelectorAll('#gd-tabs .gd-tab')) {
    btn.classList.toggle('active', btn.dataset.category === gdCategory);
  }
}

function gdSwitchCategory(cat) {
  gdCategory = cat;
  gdOpen = null;
  gdFilter = '';
  el('gd-search').value = '';
  el('gd-page').classList.add('hidden');
  el('gd-list').classList.remove('hidden');
  renderGdTabs();
  renderGdList();
}

function gdRowHtml(key, cells) {
  return `<tr class="gd-row" data-key="${escapeHtml(key)}">${cells.map((c) => `<td>${c}</td>`).join('')}</tr>`;
}

function renderGdList() {
  // Spells' own default view once Character is configured -- see
  // renderGdSpellsByClass's doc. Every other category (and Spells itself,
  // before Character has classes set) uses the flat table below.
  if (gdCategory === 'spells' && cpClasses.length > 0) {
    el('gd-list').classList.add('hidden');
    el('gd-page').classList.add('hidden');
    el('gd-spells-byclass').classList.remove('hidden');
    renderGdSpellsByClass();
    return;
  }
  el('gd-spells-byclass').classList.add('hidden');
  el('gd-list').classList.remove('hidden');
  el('gd-page').classList.add('hidden');

  const list = gdData[gdCategory] || [];
  const q = gdFilter.trim().toLowerCase();
  const matched = list.filter((e) => !q || e.name.toLowerCase().includes(q)).sort((a, b) => a.name.localeCompare(b.name));
  const rows = matched.slice(0, GD_ROW_CAP);

  const head = el('gd-table-head');
  const tbody = document.querySelector('#gd-table tbody');
  if (gdCategory === 'zones') {
    head.innerHTML = '<tr><th>zone</th><th>era</th><th>level range</th></tr>';
    tbody.innerHTML = rows.map((z) => gdRowHtml(z.name, [escapeHtml(z.name), escapeHtml(z.era || '—'), escapeHtml(z.level_range || '—')])).join('');
  } else if (gdCategory === 'items') {
    head.innerHTML = '<tr><th>item</th><th>slot(s)</th><th>class(es)</th><th>era</th></tr>';
    tbody.innerHTML = rows
      .map((it) => gdRowHtml(it.id, [escapeHtml(it.name), escapeHtml(it.slots.join(', ') || '—'), escapeHtml(it.classes.join(', ') || 'any'), escapeHtml(it.era || '—')]))
      .join('');
  } else if (gdCategory === 'npcs') {
    head.innerHTML = '<tr><th>npc</th><th>zone</th><th>level</th></tr>';
    tbody.innerHTML = rows.map((n) => gdRowHtml(n.name, [escapeHtml(n.name), escapeHtml(n.zone || '—'), escapeHtml(n.level || '—')])).join('');
  } else if (gdCategory === 'aas') {
    head.innerHTML = '<tr><th>ability</th><th>class</th><th>ranks</th><th>cost</th></tr>';
    tbody.innerHTML = rows
      .map((a) => gdRowHtml(a.name, [escapeHtml(a.name), escapeHtml(a.category), String(a.ranks), escapeHtml(a.cost_raw)]))
      .join('');
  } else {
    head.innerHTML = '<tr><th>spell</th><th>class(es)</th><th>mana</th><th>cast time</th></tr>';
    tbody.innerHTML = rows
      .map((s) =>
        gdRowHtml(s.id, [
          escapeHtml(s.name),
          escapeHtml(s.classes.map((c) => (c.level != null ? `${c.class} ${c.level}` : c.class)).join(', ') || '—'),
          s.mana != null ? String(s.mana) : '—',
          s.casting_time != null ? `${s.casting_time}s` : '—',
        ]),
      )
      .join('');
  }
  for (const row of tbody.querySelectorAll('tr[data-key]')) {
    // gdCategory is plural (matches the tab's own data-category); every
    // other consumer of "kind" (gdFind/gdOpenPage/gdKeyOf/gdLabel) is
    // singular -- gdKindOf bridges that, and fixes what was previously a
    // real bug here: passing gdCategory straight through left Items/NPCs
    // clicks silently falling through gdFind's un-matched kind branches
    // into its zone-lookup fallback, so neither ever actually opened.
    row.addEventListener('click', () => gdOpenPage(gdKindOf(gdCategory), row.dataset.key));
  }

  const empty = el('gd-empty');
  if (matched.length === 0) {
    empty.textContent = `No ${gdCategory} match that filter.`;
    empty.classList.remove('hidden');
  } else if (matched.length > rows.length) {
    empty.textContent = `Showing ${rows.length} of ${matched.length} -- narrow your search to see the rest.`;
    empty.classList.remove('hidden');
  } else {
    empty.classList.add('hidden');
  }
}

// A spell catalog entry's own class list uses "Shadowknight" (no space)
// in a handful of rows where the app's own canonical name (everywhere
// else, including cpClasses) is "Shadow Knight" -- confirmed against the
// real scrape, not assumed. Normalized only at match time, never written
// back into gdData, so the catalog's own raw field stays exactly what the
// scrape produced.
const SPELL_CLASS_ALIASES = { Shadowknight: 'Shadow Knight' };
function normalizeSpellClass(c) {
  return SPELL_CLASS_ALIASES[c] || c;
}

// A spell name's own trailing tier suffix ("Berserker Madness III"),
// split out for cosmetic [bracket] display -- NOT a claim that same-
// stem entries share one underlying spell or stat block. Each numbered
// variant is its own fully independent catalog entry with its own real
// mana/cast-time/effect data (confirmed: some same-stem pairs, like
// "Burnout II"/"Burnout III", are not simple upgrades of each other) --
// so this never merges rows, only reformats one row's own name text.
// Catalog data tops out at "V" (checked directly), so that's all this
// matches.
function spellRankSplit(name) {
  const m = name.match(/^(.+) (I|II|III|IV|V)$/);
  return m ? { base: m[1], rank: m[2] } : { base: name, rank: null };
}

function spellDisplayName(name) {
  const { base, rank } = spellRankSplit(name);
  return rank ? `${escapeHtml(base)} <span class="gd-spell-rank">[${rank}]</span>` : escapeHtml(base);
}

// gdData.spellEffects (keyed by id) is only populated once ensureGameData
// resolves -- undefined until then, same "not loaded yet" meaning a
// missing gdFind match has everywhere else in this file.
function spellEffectsFor(id) {
  return gdData.spellEffects && gdData.spellEffects[id];
}

// Seconds -> a short human string. `null`/`undefined` (nothing parsed)
// reads as "?", not "0s" or a blank -- see spelleffect.rs's own doc for
// why a real fraction of durations don't parse into a clean number.
function fmtDurationSecs(secs) {
  if (secs == null) return '?';
  if (secs === 0) return 'instant';
  if (secs < 60) return `${Math.round(secs)}s`;
  if (secs < 3600) return `${(secs / 60).toFixed(secs % 60 === 0 ? 0 : 1)}m`;
  return `${(secs / 3600).toFixed(1)}h`;
}

function spellDurationLabel(effects) {
  if (!effects) return '?';
  const d = effects.duration;
  if (d.is_permanent) return 'permanent';
  if (d.is_instant) return 'instant';
  if (d.max_secs == null) return '?';
  if (d.min_secs != null && d.min_secs !== d.max_secs) return `${fmtDurationSecs(d.min_secs)}–${fmtDurationSecs(d.max_secs)}`;
  return fmtDurationSecs(d.max_secs);
}

// One small pill per tag ("Damage", "AE Damage over Time", "Mez", ...) --
// see spelleffect.rs's own categorize() doc for exactly how these are
// decided. `null`/no entry (spell effects not loaded yet, or genuinely no
// tag) reads as a muted dash, not a fabricated "Utility".
function spellTagsHtml(id) {
  const effects = spellEffectsFor(id);
  if (!effects || !effects.tags.length) return '<span class="muted">&middot;</span>';
  return effects.tags.map((t) => `<span class="aa-tag aa-tag-stat">${escapeHtml(t)}</span>`).join(' ');
}

// The detail panel's own effect breakdown -- duration, every parsed
// `slots` component, and the description-derived damage/heal number when
// `slots` had nothing usable (see spelleffect.rs's own doc for why the
// two sources are kept visually distinct: a `slots` component names the
// exact stat it changes, a description-derived number is a best-effort
// read of prose and says so).
function spellEffectDetailHtml(id) {
  const effects = spellEffectsFor(id);
  if (!effects) return '';
  const parts = [];
  parts.push(`<div class="gp-detail-row"><b>Duration:</b> ${escapeHtml(spellDurationLabel(effects))}</div>`);
  if (effects.tags.length) {
    parts.push(`<div class="gp-detail-row">${spellTagsHtml(id)}</div>`);
  }
  for (const c of effects.components) {
    const range = c.min_amount != null && c.min_amount !== c.max_amount ? `${c.min_amount}–${c.max_amount}` : c.min_amount;
    const unit = c.unit === 'percent' ? '%' : '';
    const rate = c.per_tick ? '/tick' : '';
    parts.push(`<div class="gp-detail-row muted">${escapeHtml(c.direction)} ${escapeHtml(c.stat)}: ${range}${unit}${rate}</div>`);
  }
  if (effects.description_damage) {
    const d = effects.description_damage;
    const range = d.min_amount !== d.max_amount ? `${d.min_amount}–${d.max_amount}` : d.min_amount;
    parts.push(`<div class="gp-detail-row muted">damage (from description): ${range}${d.is_over_time ? ' over time' : ''}${d.repetitions ? ` × up to ${d.repetitions}` : ''}</div>`);
  }
  if (effects.description_heal) {
    const d = effects.description_heal;
    const range = d.min_amount !== d.max_amount ? `${d.min_amount}–${d.max_amount}` : d.min_amount;
    parts.push(`<div class="gp-detail-row muted">heal (from description): ${range}${d.is_over_time ? ' over time' : ''}</div>`);
  }
  return parts.join('');
}

// The Spells tab's default view once Character has at least one active
// class set: one thin column per active class that actually has spells
// in the catalog (a class with none -- Warrior, Monk, Berserker -- gets
// no column at all, rather than an empty one), each spell gated to that
// class's own tracked level (cpLevels, the same per-class number the
// Character Planner itself uses -- not the trio-wide character_level,
// since a class you've already reached a level on keeps that level
// independently of whichever 3 are active right now). The "type" pills
// are real (spelleffect.rs), not the placeholder this view started with
// -- best-effort, see that module's own doc for exactly what's parsed
// vs. genuinely unknown.
function renderGdSpellsByClass() {
  const box = el('gd-spells-columns');
  const columns = cpClasses
    .map((cls) => {
      const level = cpLevels[cls] ?? 1;
      const spells = (gdData.spells || [])
        .filter((s) => s.classes.some((c) => normalizeSpellClass(c.class) === cls && c.level != null && c.level <= level))
        .sort((a, b) => {
          const la = a.classes.find((c) => normalizeSpellClass(c.class) === cls)?.level ?? 0;
          const lb = b.classes.find((c) => normalizeSpellClass(c.class) === cls)?.level ?? 0;
          return la - lb || a.name.localeCompare(b.name);
        });
      return { cls, level, spells };
    })
    .filter((col) => col.spells.length > 0); // "if they have spells" -- skip a column with none

  if (!columns.length) {
    box.innerHTML = '<p class="muted">None of your active classes have any catalog spells at their current level yet.</p>';
    return;
  }

  box.innerHTML = columns
    .map(({ cls, level, spells }) => {
      const rows = spells
        .map((s) => {
          const reqLevel = s.classes.find((c) => normalizeSpellClass(c.class) === cls)?.level;
          const icon = s.icon ? `<img class="gd-spell-row-icon" src="planner/icons/${encodeURIComponent(s.icon)}" alt="">` : '<span class="gd-spell-row-icon gd-spell-row-icon-blank"></span>';
          return `<li class="gd-spell-row" data-id="${escapeHtml(s.id)}" draggable="true">
            ${icon}
            <span class="gd-spell-row-name">${spellDisplayName(s.name)} <span class="muted">(${reqLevel ?? '?'})</span></span>
            <span class="gd-spell-row-type">${spellTagsHtml(s.id)}</span>
          </li>`;
        })
        .join('');
      return `<div class="gd-spell-column">
        <h3>${escapeHtml(cls)} <span class="muted">lvl ${level}</span></h3>
        <ul class="gd-spell-list">${rows}</ul>
      </div>`;
    })
    .join('');

  for (const row of box.querySelectorAll('.gd-spell-row')) {
    row.addEventListener('click', () => {
      box.querySelectorAll('.gd-spell-row.active').forEach((r) => r.classList.remove('active'));
      row.classList.add('active');
      openGdSpellDetail(row.dataset.id);
    });
    row.addEventListener('dragstart', (e) => {
      e.dataTransfer.setData('text/spell-id', row.dataset.id);
      e.dataTransfer.effectAllowed = 'copy';
    });
  }

  refreshSpellSlotCount().then(renderSpellSlots);
}

// ---------------------------------------------------------------- spell slot planner

// A character's own gem window is one shared pool, not one per class --
// even though this view shows all 3 active classes' books side by side
// for planning, only one set of slots exists below them, and any spell
// from any of the 3 columns can go in any slot (there's no per-slot
// restriction the way gear has a slot *type* per item). In-memory only,
// same "purges every start" stance the Gear tab's own gpChosen/gpEquipped
// already take -- this is a planning scratchpad, not a session fact to
// preserve.
let spellSlots = []; // Spell object or null, per slot

// 8 base gem slots, confirmed by the user for this fork (not assumed
// from vanilla classic EQ) -- Mnemonic Retention adds its own rank
// on top, 1:1 (rank 6 = +6, for 8+6=14 total at max).
const SPELL_SLOT_BASE = 8;
let spellSlotMnemonicRank = 0;

function totalSpellSlots() {
  return SPELL_SLOT_BASE + spellSlotMnemonicRank;
}

// Grows/shrinks spellSlots to match the current total without disturbing
// whatever's already assigned -- a rank bought mid-session (or the AA log
// simply loading late) should add empty slots at the end, never reshuffle
// or clear existing picks.
function ensureSpellSlotsLength() {
  const total = totalSpellSlots();
  while (spellSlots.length < total) spellSlots.push(null);
  if (spellSlots.length > total) spellSlots.length = total;
}

// Mnemonic Retention's own rank is read straight from the AA log
// (confirmed purchases), not the Character sheet's AA subpage cache --
// this view can be the first thing opened in a session, so it fetches
// for itself rather than assuming that subpage has already loaded.
// Multiple grants (rank 1, later rank 2, ...) are real, separate log
// entries -- the *highest* rank seen is what's currently owned.
async function refreshSpellSlotCount() {
  try {
    const log = await invoke('get_aa_log');
    const mine = log.grants.filter((g) => g.name === 'Mnemonic Retention');
    spellSlotMnemonicRank = mine.length ? Math.max(...mine.map((g) => g.rank)) : 0;
  } catch {
    spellSlotMnemonicRank = 0;
  }
}

function renderSpellSlots() {
  ensureSpellSlotsLength();
  const box = el('gd-spell-slots');
  if (!box) return;
  box.innerHTML = spellSlots
    .map((s, i) => {
      if (!s) return `<div class="gd-spell-slot empty" data-slot="${i}"></div>`;
      const icon = s.icon ? `<img class="gd-spell-row-icon" src="planner/icons/${encodeURIComponent(s.icon)}" alt="">` : '';
      return `<div class="gd-spell-slot filled" data-slot="${i}" data-id="${escapeHtml(s.id)}" draggable="true">
        ${icon}
        <span class="gd-spell-slot-name">${spellDisplayName(s.name)}</span>
        <button class="gd-spell-slot-remove" data-slot="${i}" title="Remove">&times;</button>
      </div>`;
    })
    .join('');

  for (const slotEl of box.querySelectorAll('.gd-spell-slot')) {
    slotEl.addEventListener('dragover', (e) => {
      e.preventDefault();
      slotEl.classList.add('drag-over');
    });
    slotEl.addEventListener('dragleave', () => slotEl.classList.remove('drag-over'));
    slotEl.addEventListener('drop', (e) => {
      e.preventDefault();
      slotEl.classList.remove('drag-over');
      const id = e.dataTransfer.getData('text/spell-id');
      const spell = id && gdFind('spell', id);
      if (!spell) return;
      const destIdx = Number(slotEl.dataset.slot);
      // Set only when the drag started from an existing slot (see the
      // .filled dragstart handler below) -- clearing it turns this into
      // a real move instead of leaving a duplicate behind in the slot it
      // came from. Unset (empty string) when dragging fresh from a
      // spellbook column, which should only ever fill, never clear
      // anything.
      const originStr = e.dataTransfer.getData('text/spell-slot-origin');
      if (originStr !== '') {
        const originIdx = Number(originStr);
        if (originIdx === destIdx) return; // dropped back on itself
        spellSlots[originIdx] = null;
      }
      spellSlots[destIdx] = spell;
      renderSpellSlots();
    });
  }
  for (const slotEl of box.querySelectorAll('.gd-spell-slot.filled')) {
    slotEl.addEventListener('dragstart', (e) => {
      e.dataTransfer.setData('text/spell-id', slotEl.dataset.id);
      e.dataTransfer.setData('text/spell-slot-origin', slotEl.dataset.slot);
      e.dataTransfer.effectAllowed = 'move';
    });
    slotEl.addEventListener('click', (e) => {
      if (e.target.closest('.gd-spell-slot-remove')) return;
      openGdSpellDetail(slotEl.dataset.id);
    });
  }
  for (const btn of box.querySelectorAll('.gd-spell-slot-remove')) {
    btn.addEventListener('click', (e) => {
      e.stopPropagation();
      spellSlots[Number(btn.dataset.slot)] = null;
      renderSpellSlots();
    });
  }

  const filled = spellSlots.filter(Boolean);
  const totalMana = filled.reduce((sum, s) => sum + (s.mana || 0), 0);
  const mnemonicNote = spellSlotMnemonicRank ? ` (${SPELL_SLOT_BASE} base + ${spellSlotMnemonicRank} Mnemonic Retention)` : ` (${SPELL_SLOT_BASE} base, no Mnemonic Retention yet)`;
  el('gd-spell-slots-summary').textContent =
    `${filled.length} / ${spellSlots.length} filled${mnemonicNote}${filled.length ? ` -- ${totalMana} total mana` : ''}`;
}

// Bumped per open, same guard shape as gdOpenToken -- a slow spellbook
// fetch for a spell you've since clicked away from must not clobber the
// panel's now-different content.
let gdSpellDetailToken = 0;

async function openGdSpellDetail(spellId) {
  const spell = gdFind('spell', spellId);
  const panel = el('gd-spell-detail');
  if (!spell) return;
  panel.classList.remove('muted');
  const token = ++gdSpellDetailToken;

  const { base, rank } = spellRankSplit(spell.name);
  const rankLabel = rank ? ` <span class="gd-spell-rank">[${rank}]</span>` : '';
  const classes = spell.classes.map((c) => `${normalizeSpellClass(c.class)}${c.level != null ? ` (${c.level})` : ''}`).join(', ');
  const fields = [
    gdFieldRow('Class(es)', escapeHtml(classes) || '—'),
    gdFieldRow('Mana', spell.mana != null ? String(spell.mana) : ''),
    gdFieldRow('Casting time', spell.casting_time != null ? `${spell.casting_time}s` : ''),
    gdFieldRow('Recast time', spell.recast_time != null ? `${spell.recast_time}s` : ''),
    gdFieldRow('Range', spell.range != null ? String(spell.range) : ''),
    gdFieldRow('Duration', spell.duration && escapeHtml(spell.duration)),
    gdFieldRow('Target', spell.target_type && escapeHtml(spell.target_type)),
    gdFieldRow('Resist', spell.resist && escapeHtml(spell.resist)),
  ]
    .filter(Boolean)
    .join('');
  const wikiUrl = spell.url || `https://eqlwiki.com/${encodeURIComponent(spell.name.replace(/ /g, '_'))}`;

  panel.innerHTML = `
    <div class="gp-detail-head">
      ${spell.icon ? `<img class="gp-detail-icon" src="planner/icons/${encodeURIComponent(spell.icon)}" alt="">` : ''}
      <div class="gp-detail-name">${escapeHtml(base)}${rankLabel}</div>
    </div>
    <dl class="gd-fields">${fields}</dl>
    ${spell.description ? `<p class="gd-description">${escapeHtml(spell.description)}</p>` : ''}
    <h3>Effects</h3>
    ${spellEffectDetailHtml(spell.id)}
    <h3>Confirmed known</h3>
    <div id="gd-spell-detail-known" class="muted">Loading&hellip;</div>
    <a class="gd-wiki-link" href="${escapeHtml(wikiUrl)}" target="_blank" rel="noopener">eqlwiki ↗ (backup)</a>
  `;

  let entries;
  try {
    entries = await invoke('get_spellbook');
  } catch (e) {
    if (token !== gdSpellDetailToken) return;
    const slot = document.getElementById('gd-spell-detail-known');
    if (slot) slot.innerHTML = `<p class="muted">Couldn't load your spellbook: ${escapeHtml(String(e))}</p>`;
    return;
  }
  if (token !== gdSpellDetailToken) return;
  const slot = document.getElementById('gd-spell-detail-known');
  if (!slot) return;
  slot.innerHTML = spellOwnershipHtml(entries.find((e) => e.name === spell.name));
}

el('gd-search').addEventListener('input', (e) => {
  gdFilter = e.target.value;
  renderGdList();
});
el('gd-back').addEventListener('click', () => {
  gdOpen = null;
  el('gd-page').classList.add('hidden');
  el('gd-list').classList.remove('hidden');
  renderGdList();
});
for (const btn of document.querySelectorAll('#gd-tabs .gd-tab')) {
  btn.addEventListener('click', () => gdSwitchCategory(btn.dataset.category));
}

// kind (singular: 'zone'/'item'/'npc'/'aa'/'spell') <-> gdData/gdCategory
// bucket key (plural). Two separate, deliberately-not-unified vocabularies
// meeting here: gdCategory tracks which *tab* is open (plural, matches
// the HTML's own data-category values), while every cross-link, .gd-link
// span, and gdFind/gdOpenPage/gdKeyOf/gdLabel call uses the singular kind
// -- "one zone", "one item". gdKindOf is the one place that translates
// between them, used by renderGdList's own row click wiring so a list
// click and a cross-link click open the exact same page through the exact
// same singular-kind path.
function gdKindOf(category) {
  return { zones: 'zone', items: 'item', npcs: 'npc', aas: 'aa', spells: 'spell' }[category] || category;
}

function gdListFor(kind) {
  const bucket = { zone: 'zones', item: 'items', npc: 'npcs', aa: 'aas', spell: 'spells' }[kind] || 'zones';
  return gdData[bucket] || [];
}

// Items key by id (stable even if two items ever shared a name); zones by
// name. NPCs try name first, then wiki page id with underscores turned
// back to spaces -- a disambiguated title ("Cazic Thule (God)") is how a
// raid boss actually reaches this field from elsewhere (see
// gdZoneOrMobLink's doc), and only ever matches the id, since the NPC's
// own display `name` is the shorter "Cazic Thule".
// Mirrors zone::zone_key/zone_matches/ZONE_ALIASES in zone.rs (Rust)
// exactly. A raw zone string reaches the frontend as-is wherever it's for
// *display* (monsters::item_loot_history's own zone field, deliberately --
// that's the honest record of what the log actually said), so *matching*
// it against the wiki's own zone name needs the identical normalization
// on both sides. zone_key alone (tier/raid suffix, then a leading "The")
// isn't enough -- checked against the real reference log on the Rust
// side, 42 of 120 real zone labels still didn't resolve after that: the
// wiki's own zone-guide titles use a different naming convention outright
// for a real chunk of zones. ZONE_ALIASES is that same confirmed list,
// kept in sync by hand since it's small and rarely changes -- see
// zone.rs's own copy for how each entry was verified.
const ZONE_TIER_SUFFIXES = [' 1 (Awakened)', ' 2 (Adaptive)', ' 3 (Fused)', ' 4 (Refined)'];
const ZONE_ALIASES = {
  'clan crushbone': 'Crushbone',
  'east freeport': 'Freeport',
  'west freeport': 'Freeport',
  'erudin palace': 'Erudin',
  'everquest legends tutorial': 'Tutorial Zone',
  'kerra isle': 'Kerra Island',
  'neriak - commons': 'Neriak',
  'neriak - foreign quarter': 'Neriak',
  'neriak - third gate': 'Neriak',
  'north kaladim': 'Kaladim',
  'south kaladim': 'Kaladim',
  'north qeynos': 'Qeynos',
  'south qeynos': 'Qeynos',
  'northern felwithe': 'Felwithe',
  'permafrost keep': 'Permafrost',
  'permafrost caverns': 'Permafrost',
  'temple of cazic-thule': 'Cazic Thule (Zone)',
  'city of guk': 'Upper Guk',
  'lair of the splitpaw': 'Splitpaw Lair',
  'liberated citadel of runnyeye': 'Runnyeye',
  'qeynos aqueduct system': 'Qeynos Aqueducts',
  'ruins of old guk': 'Lower Guk',
  'southern plains of karana': 'Southern Karana',
  'western plains of karana': 'Western Karana',
};

function zoneKey(raw) {
  let base = raw;
  for (const suffix of ZONE_TIER_SUFFIXES) {
    if (base.endsWith(suffix)) {
      base = base.slice(0, -suffix.length);
      break;
    }
  }
  if (base.endsWith(' - Group')) base = base.slice(0, -' - Group'.length);
  if (base.startsWith('The ')) base = base.slice(4);
  return base;
}

function zoneMatches(raw, wikiName) {
  const key = zoneKey(raw);
  const resolved = ZONE_ALIASES[key.toLowerCase()] || key;
  return resolved.toLowerCase() === zoneKey(wikiName).toLowerCase();
}

function gdFind(kind, key) {
  const list = gdListFor(kind);
  const k = String(key).toLowerCase();
  if (kind === 'item') {
    return list.find((it) => it.id === key) || list.find((it) => it.name.toLowerCase() === k);
  }
  if (kind === 'npc') {
    return list.find((n) => n.name.toLowerCase() === k) || list.find((n) => n.id.replace(/_/g, ' ').toLowerCase() === k);
  }
  if (kind === 'aa') {
    return list.find((a) => a.name.toLowerCase() === k);
  }
  if (kind === 'spell') {
    return list.find((s) => s.id === key) || list.find((s) => s.name.toLowerCase() === k);
  }
  // zone -- exact match first (the common, untiered case), then
  // zoneMatches so a tiered/raid-instance/differently-worded log string
  // still resolves to its real wiki zone page.
  return list.find((e) => e.name.toLowerCase() === k) || list.find((e) => zoneMatches(String(key), e.name));
}

function gdKeyOf(kind, entry) {
  return kind === 'item' || kind === 'spell' ? entry.id : entry.name;
}

function gdLabel(kind) {
  return { zone: 'Zones', item: 'Items', npc: 'NPCs', aa: 'AAs', spell: 'Spells' }[kind] || 'NPCs';
}

// A cross-reference name (a zone's notable NPC, an NPC's known-loot item,
// ...) becomes a real .gd-link only if that dataset actually has a match
// -- both scrapes are independent passes over the same wiki, so one
// naming something the other doesn't cover is a real possibility, not a
// bug this should paper over with a dead link.
function gdLinkOrText(kind, name) {
  const found = gdFind(kind, name);
  return found ? `<span class="gd-link" data-kind="${kind}" data-id="${escapeHtml(gdKeyOf(kind, found))}">${escapeHtml(name)}</span>` : escapeHtml(name);
}

function gdLinkList(kind, names) {
  return names.length ? names.map((n) => gdLinkOrText(kind, n)).join(', ') : '';
}

// An item's own `zones` list is really "drop-source strings", and not
// every one of them is actually a zone. The scrape's dropsfrom parser
// (parse_drops in scrape.py) reads "* [[mob]]" bullets as mobs and any
// other linked line as a new zone -- which breaks for a wiki "Dropped By"
// list that names a raid boss directly, with no explicit zone line
// wrapping it (Slime Blood of Cazic-Thule's page lists "Plane of Fear",
// "Fright", "Dread", "Terror", then a bare "Cazic Thule (God)" link with
// no bullet -- the last one parses as its own zone entry, mobs: [], even
// though it's actually the encounter that drops the item). Rather than
// fixing that one item's data by hand, this checks every such string
// against *both* datasets at render time: a real zone wins first (the
// common case), otherwise a matching NPC wins instead, tagged RAID if the
// wiki's own "Raid Encounters" category says so -- so the string still
// renders correctly (and links somewhere real) no matter which scrape
// pass actually named it, and any other item hitting the same parsing
// quirk gets the same fix for free.
function gdZoneOrMobLink(name) {
  if (gdFind('zone', name)) return gdLinkOrText('zone', name);
  const npc = gdFind('npc', name);
  if (npc) {
    const raid = npc.categories.includes('Raid Encounters') ? ' <span class="gd-raid-badge">raid</span>' : '';
    return `${gdLinkOrText('npc', name)}${raid}`;
  }
  return escapeHtml(name);
}

function gdZoneOrMobLinkList(names) {
  return names.length ? names.map(gdZoneOrMobLink).join(', ') : '';
}

// An NPC's own `location` field is raw wikitext ("[[Zone A]], [[Zone
// B]]", or just "Various") -- strip the link brackets and try each
// comma-separated piece as its own zone link.
function gdLocationHtml(location) {
  if (!location) return '';
  return location
    .split(',')
    .map((part) => part.trim().replace(/\[\[|\]\]/g, ''))
    .filter(Boolean)
    .map((name) => gdLinkOrText('zone', name))
    .join(', ');
}

// `value` is pre-formatted (plain escaped text or a .gd-link span already
// built by the caller), not raw -- matches every field a page renders
// through this. A field nothing was scraped for (most NPCs carry half
// these blank, per npcs.json) is omitted outright rather than shown empty.
function gdFieldRow(label, value) {
  return value ? `<dt>${escapeHtml(label)}</dt><dd>${value}</dd>` : '';
}

function gdOpenPage(kind, key) {
  const entry = gdFind(kind, key);
  if (!entry) return;
  gdOpen = { kind, key };
  gdZoneEncExpanded = null;
  const token = ++gdOpenToken;
  renderGdShell();
  if (kind === 'item') {
    fillItemLootHistory(entry.name, token);
  } else if (kind === 'zone') {
    fillZoneEncounters(entry.id, token);
  } else if (kind === 'npc') {
    fillMobStats(entry.name, token);
    fillMobEncounters(entry.name, token);
  } else if (kind === 'aa') {
    fillAaOwnership(entry.name, token);
  } else if (kind === 'spell') {
    fillSpellOwnership(entry.name, token);
  }
}

const GD_PAGE_RENDERERS = { zone: gdZonePageHtml, item: gdItemPageHtml, npc: gdNpcPageHtml, aa: gdAaPageHtml, spell: gdSpellPageHtml };

function renderGdShell() {
  el('gd-list').classList.add('hidden');
  el('gd-page').classList.remove('hidden');
  el('gd-back-label').textContent = gdLabel(gdOpen.kind);
  const entry = gdFind(gdOpen.kind, gdOpen.key);
  const body = el('gd-page-body');
  body.innerHTML = (GD_PAGE_RENDERERS[gdOpen.kind] || gdNpcPageHtml)(entry);
  // .content, not .gd-page, is the actual scrolling ancestor -- opening a
  // page from partway down a long list shouldn't leave the new page's own
  // top scrolled out of view.
  document.querySelector('.content').scrollTop = 0;
}

// "Notable NPCs" (below, from the zone's own wiki page) is the wiki's own
// curated highlight list -- per npcdata.rs's doc, the NPC scrape only ever
// covers "Named Mobs" pages to begin with, so even that curated list is
// already a subset of everything actually in the zone. This cross-checks
// the *other* direction instead: every NPC in the full bestiary whose own
// `zone` field names this zone, which is why an ordinary trash mob can
// show up here even when the wiki's own zone page never called it out.
function gdMobsInZone(zoneName) {
  const names = [...new Set(gdData.npcs.filter((n) => n.zone && n.zone.toLowerCase() === zoneName.toLowerCase()).map((n) => n.name))].sort(
    (a, b) => a.localeCompare(b),
  );
  if (!names.length) return '';
  return `<h3>Mobs found here</h3><p class="gd-mob-list">${gdLinkList('npc', names)}</p>`;
}

// Which zone-page encounter row (by EncounterDto id) is currently
// expanded -- page-scoped, reset in gdOpenPage every time a new page
// opens rather than carried over from whatever zone you last looked at.
let gdZoneEncExpanded = null;

function fmtTtk(ms) {
  const total = Math.max(0, Math.round(ms / 1000));
  const m = Math.floor(total / 60);
  const s = total % 60;
  return m > 0 ? `${m}m ${s}s` : `${s}s`;
}

// First few drop names, rarer-first (already sorted server-side -- see
// combat::encounter_detail's doc on drop ranking), for the preview line --
// the full list with quantities, and damage totals, only show once a row
// is expanded. `detail` is null until get_encounter_detail resolves for
// this row (see fillZoneEncounters' progressive per-row fetch) --
// distinguished from "fetched, genuinely nothing looted", which reads as
// an empty `detail.drops` instead.
function gdDropPreviewHtml(detail) {
  if (!detail) return '<span class="muted">…</span>';
  if (!detail.drops.length) return '<span class="muted">no drops recorded</span>';
  const shown = detail.drops.slice(0, 3).map((d) => escapeHtml(d.item));
  return shown.join(', ') + (detail.drops.length > 3 ? ` +${detail.drops.length - 3} more` : '');
}

// Shared by zone pages (list_zone_encounters) and NPC pages
// (list_mob_encounters) -- both return the identical ZoneEncounterDto
// shape, so one render path covers both. `showZone`, when true, adds each
// fight's own zone to the preview line -- meaningful on an NPC page (the
// same mob can turn up in different zones), redundant on a zone page
// (you're already looking at that zone) so left off there.
function gdZoneEncounterRowHtml(ze, showZone) {
  const e = ze.encounter;
  const expanded = gdZoneEncExpanded === e.id;
  const outcome = e.open ? 'ongoing' : e.slain ? 'kill' : 'reset';
  const tierLabel = ze.tier > 0 ? `tier ${ze.tier}` : 'base difficulty';
  const zoneBit = showZone && ze.zone ? ` · ${escapeHtml(ze.zone)}` : '';
  const detail = ze._detail; // set once get_encounter_detail resolves, else undefined

  const detailBody = !detail
    ? '<div class="muted">Loading&hellip;</div>'
    : `<div>${detail.total_damage.toLocaleString()} dmg dealt (${Math.round(detail.dps).toLocaleString()} dps) -- ${detail.enemy_damage.toLocaleString()} dmg taken (${Math.round(detail.enemy_dps).toLocaleString()} dps)</div>
       <div>${
         detail.drops.length
           ? detail.drops.map((d) => `${escapeHtml(d.item)}${d.qty > 1 ? ` ×${d.qty}` : ''}`).join(', ')
           : '<span class="muted">no drops recorded</span>'
       }</div>`;
  const expandedHtml = expanded
    ? `<div class="gd-enc-detail">
        ${detailBody}
        <button type="button" class="link-button gd-enc-open" data-zone-visit="${e.zone_visit === null ? '' : e.zone_visit}" data-encounter-id="${e.id}">open in Combat &rarr;</button>
      </div>`
    : '';

  return `<div class="gd-enc-row${expanded ? ' expanded' : ''}" data-encounter-id="${e.id}">
    <div class="gd-enc-preview">
      <span class="gd-enc-target">${escapeHtml(e.target)}</span>
      <span class="gd-enc-meta">${escapeHtml(new Date(e.start_ms).toLocaleString())} · ttk ${fmtTtk(e.duration_ms)} · ${tierLabel} · ${outcome}${zoneBit}</span>
      <span class="gd-enc-drops-preview">${gdDropPreviewHtml(detail)}</span>
    </div>
    ${expandedHtml}
  </div>`;
}

function renderGdZoneEncounters(list, showZone) {
  const box = document.getElementById('gd-zone-encounters');
  if (!box) return; // navigated to a different page while this list was still open
  if (!list.length) {
    box.innerHTML = '<p class="muted">No parsed encounters here yet.</p>';
    return;
  }
  // The backend caps at GD_ZONE_ENC_LIMIT and returns newest-first -- a
  // full-length result means there's likely more further back that
  // simply isn't fetched, not that this zone/mob has exactly that many
  // fights total ever. Said plainly rather than silently truncating with
  // no indication older ones exist.
  const capNote =
    list.length >= GD_ZONE_ENC_LIMIT ? `<p class="muted">Showing the ${GD_ZONE_ENC_LIMIT} most recent -- older fights aren't loaded here.</p>` : '';
  box.innerHTML = list.map((ze) => gdZoneEncounterRowHtml(ze, showZone)).join('') + capNote;
  // Delegated and re-assigned each render (not addEventListener, which
  // would stack a new handler per expand/collapse) -- same pattern
  // renderAllies/renderMonsters already use for their own expand rows.
  box.onclick = (e) => {
    const openBtn = e.target.closest('.gd-enc-open');
    if (openBtn) {
      const zv = openBtn.dataset.zoneVisit === '' ? null : Number(openBtn.dataset.zoneVisit);
      // Pushed before leaving, not after -- this is "where we're jumping
      // *from*". gdOpen/gdZoneEncExpanded are already preserved as plain
      // globals (see resetModuleToDefault's doc), so the restore doesn't
      // need to snapshot them itself -- re-showing Game Data naturally
      // re-renders whatever they still hold, whichever page kind it was.
      const openEntry = gdOpen ? gdFind(gdOpen.kind, gdOpen.key) : null;
      navPush(openEntry ? openEntry.name : 'Game Data', () => showModule('gamedata'));
      showModule('combat');
      jumpToEncounter(zv, Number(openBtn.dataset.encounterId));
      return;
    }
    const row = e.target.closest('.gd-enc-row');
    if (!row) return;
    const id = Number(row.dataset.encounterId);
    gdZoneEncExpanded = gdZoneEncExpanded === id ? null : id;
    renderGdZoneEncounters(list, showZone);
  };
}

// Newest-first, capped -- see combat::list_zone_encounters' own doc for
// why this stays fast (a bounded reverse scan of already-in-memory data,
// not a log re-read) regardless of how long the session's history is.
const GD_ZONE_ENC_LIMIT = 30;

// Damage totals + drops load one encounter at a time, after the list
// itself is already on screen -- not awaited together, so each one paints
// in as its own (cheap, windowed -- see combat::encounter_detail's doc)
// fetch resolves, instead of the whole list waiting on all of them
// together. Shared by fillZoneEncounters and fillMobEncounters.
function fillEncounterDetails(list, token, showZone) {
  for (const ze of list) {
    invoke('get_encounter_detail', { encounterId: ze.encounter.id })
      .then((detail) => {
        if (token !== gdOpenToken) return; // navigated elsewhere while this was in flight
        ze._detail = detail || { total_damage: 0, dps: 0, enemy_damage: 0, enemy_dps: 0, drops: [] };
        renderGdZoneEncounters(list, showZone);
      })
      .catch(() => {
        if (token !== gdOpenToken) return;
        ze._detail = { total_damage: 0, dps: 0, enemy_damage: 0, enemy_dps: 0, drops: [] }; // failed, but stop showing "Loading..." forever
        renderGdZoneEncounters(list, showZone);
      });
  }
}

// Fetched after the page shell already rendered (the zone's own wiki
// fields don't need to wait on this), same token-guard pattern
// fillItemLootHistory uses: a slow response for a zone page you've since
// navigated away from can't patch content that now belongs to a
// different page, because #gd-zone-encounters' id is reused across every
// zone/NPC page rather than being page-specific.
async function fillZoneEncounters(zoneId, token) {
  const box = document.getElementById('gd-zone-encounters');
  let list = [];
  try {
    list = await invoke('list_zone_encounters', { zoneId, limit: GD_ZONE_ENC_LIMIT });
  } catch (err) {
    // A real failure (bad IPC args, a backend panic) reads identically to
    // "no encounters" if this stays silent -- said plainly instead, since
    // an empty zone and a broken fetch need different reactions from you.
    if (token === gdOpenToken && box) {
      box.classList.remove('muted');
      box.innerHTML = `<p class="muted">Couldn't load encounters: ${escapeHtml(String(err))}</p>`;
    }
    return;
  }
  if (token !== gdOpenToken) return;
  renderGdZoneEncounters(list, false);
  fillEncounterDetails(list, token, false);
}

function gdEncounterSection() {
  return `<h3>Your parsed encounters here</h3><div id="gd-zone-encounters" class="muted">Loading&hellip;</div>`;
}

// An NPC page's own personalized section -- kills/pulls totals plus the
// same recent-encounters list a zone page shows, with each row's own zone
// shown too (see gdZoneEncounterRowHtml's showZone) since the same mob
// can turn up in more than one.
function gdMobHistorySection() {
  return `<h3>Your history with this mob</h3><p id="gd-mob-stats" class="muted">Loading&hellip;</p><div id="gd-zone-encounters" class="muted">Loading&hellip;</div>`;
}

async function fillMobStats(mobName, token) {
  let stats = null;
  try {
    stats = await invoke('get_mob_stats', { mobName });
  } catch {
    stats = null;
  }
  if (token !== gdOpenToken) return;
  const box = document.getElementById('gd-mob-stats');
  if (!box) return;
  box.classList.remove('muted');
  if (!stats || stats.pulls === 0) {
    box.innerHTML = '<span class="muted">Not fought yet this session.</span>';
    return;
  }
  box.textContent = `${stats.kills.toLocaleString()} confirmed kill${stats.kills === 1 ? '' : 's'} across ${stats.pulls.toLocaleString()} pull${stats.pulls === 1 ? '' : 's'}.`;
}

async function fillMobEncounters(mobName, token) {
  const box = document.getElementById('gd-zone-encounters');
  let list = [];
  try {
    list = await invoke('list_mob_encounters', { mobName, limit: GD_ZONE_ENC_LIMIT });
  } catch (err) {
    if (token === gdOpenToken && box) {
      box.classList.remove('muted');
      box.innerHTML = `<p class="muted">Couldn't load encounters: ${escapeHtml(String(err))}</p>`;
    }
    return;
  }
  if (token !== gdOpenToken) return;
  renderGdZoneEncounters(list, true);
  fillEncounterDetails(list, token, true);
}

function gdZonePageHtml(zone) {
  const fields = [
    // The same id combat::list_zone_encounters matches against and
    // debugview shows per encounter as "resolved zone id" -- shown here
    // specifically so the two are a plain visual comparison, not
    // something to just trust.
    gdFieldRow('Zone ID', `<code>${escapeHtml(zone.id)}</code>`),
    gdFieldRow('Level range', zone.level_range && escapeHtml(zone.level_range)),
    gdFieldRow('Monster types', zone.monster_types && escapeHtml(zone.monster_types)),
    gdFieldRow('Notable NPCs', gdLinkList('npc', zone.notable_npcs)),
    gdFieldRow('Unique items', gdLinkList('item', zone.unique_items)),
    gdFieldRow('Related quests', zone.related_quests.length ? escapeHtml(zone.related_quests.join(', ')) : ''),
    gdFieldRow('Guilds', zone.guilds.length ? escapeHtml(zone.guilds.join(', ')) : ''),
    gdFieldRow('Tradeskill facilities', zone.tradeskill_facilities.length ? escapeHtml(zone.tradeskill_facilities.join(', ')) : ''),
    gdFieldRow('City races', zone.city_races.length ? escapeHtml(zone.city_races.join(', ')) : ''),
    gdFieldRow('Adjacent zones', gdLinkList('zone', zone.adjacent_zones)),
    gdFieldRow('Spawn timer', zone.spawn_timer && escapeHtml(zone.spawn_timer)),
    gdFieldRow('Succor/Evac', zone.succor_evacuate && escapeHtml(zone.succor_evacuate)),
    gdFieldRow('/who name', zone.who_name && escapeHtml(zone.who_name)),
  ]
    .filter(Boolean)
    .join('');
  return `
    <h2 class="gd-page-title">${escapeHtml(zone.name)}</h2>
    <dl class="gd-fields">${fields}</dl>
    ${gdMobsInZone(zone.name)}
    ${gdEncounterSection()}
    <a class="gd-wiki-link" href="${escapeHtml(zone.url)}" target="_blank" rel="noopener">eqlwiki ↗ (backup / full page)</a>
  `;
}

function gdItemPageHtml(item) {
  const statRows = Object.entries(item.stats || {})
    .sort((a, b) => b[1] - a[1])
    .map(([k, v]) => `<span>${escapeHtml(k)} ${v >= 0 ? '+' : ''}${v}</span>`)
    .join('');
  const weapon =
    item.dmg != null && item.delay != null
      ? `<div class="gp-detail-row">${item.dmg} / ${item.delay} (${(item.dmg / item.delay).toFixed(2)} ratio)</div>`
      : '';
  const tagsHtml =
    item.tags.length || item.skill
      ? `<div class="gp-tags">${item.tags.map((t) => gpTagPill(t)).join('')}${item.skill ? gpSkillPill(item.skill) : ''}</div>`
      : '';
  const wikiUrl = item.url || `https://eqlwiki.com/${encodeURIComponent(item.name.replace(/ /g, '_'))}`;
  const zonesLine = item.zones.length ? `<div class="gp-detail-row">Drops in: ${gdZoneOrMobLinkList(item.zones)}</div>` : '';
  const mobsLine = item.mobs.length ? `<div class="gp-detail-row">From: ${gdLinkList('npc', item.mobs)}</div>` : '';
  return `
    <div class="gp-detail-head">
      ${item.icon ? `<img class="gp-detail-icon" src="planner/icons/${encodeURIComponent(item.icon)}" alt="">` : ''}
      <div>
        <div class="gp-detail-name">${escapeHtml(item.name)}<a href="${escapeHtml(wikiUrl)}" target="_blank" rel="noopener">eqlwiki ↗ (backup)</a></div>
        <div class="gp-detail-row">${escapeHtml(item.classes.join(' / ') || 'any class')} -- ${escapeHtml(item.slots.join(', ') || '—')}${item.era ? ` -- ${escapeHtml(item.era)}` : ''}${item.wt != null ? ` -- WT ${item.wt}` : ''}${item.size ? ` -- ${escapeHtml(item.size)}` : ''}</div>
      </div>
    </div>
    ${statRows ? `<div class="gp-detail-stats">${statRows}</div>` : ''}
    ${weapon}
    ${tagsHtml}
    ${zonesLine}
    ${mobsLine}
    <div class="gd-loot-history">
      <h3>Your history with this item</h3>
      <div id="gd-item-loot" class="muted">Loading&hellip;</div>
    </div>
  `;
}

// Individual loot events (see monsters::item_loot_history's doc for
// exactly what each one does and doesn't know -- a real timestamp and
// best-effort zone, but never a specific encounter). Fetched after the
// page shell already rendered, not before -- an item's own fields don't
// need to wait on a store scan just to show a "Loading..." page.
//
// `token` guards against exactly one race: click item A, click item B
// before A's fetch resolves, A's response lands after B's page is already
// showing -- without this check it would silently overwrite B's loot
// section with A's data, since #gd-item-loot's id is reused across every
// item page rather than being item-specific.
async function fillItemLootHistory(itemName, token) {
  let events;
  try {
    events = await invoke('get_item_loot_history', { item: itemName });
  } catch (err) {
    // Distinguish a real failure from "genuinely never looted" -- a
    // silent catch would show the same "not looted yet" text either way.
    if (token === gdOpenToken) {
      const slot = document.getElementById('gd-item-loot');
      if (slot) {
        slot.classList.remove('muted');
        slot.innerHTML = `<p class="muted">Couldn't load loot history: ${escapeHtml(String(err))}</p>`;
      }
    }
    return;
  }
  if (token !== gdOpenToken) return; // navigated elsewhere while this was in flight
  const slot = document.getElementById('gd-item-loot');
  if (!slot) return;
  slot.classList.remove('muted');
  if (!events.length) {
    slot.innerHTML = '<p class="muted">Not looted yet this session.</p>';
    return;
  }
  slot.innerHTML = events
    .slice()
    .reverse()
    .map((e) => {
      const when = new Date(e.ts_ms).toLocaleString();
      const zone = e.zone ? gdLinkOrText('zone', e.zone) : 'unknown zone';
      const qty = e.qty > 1 ? `${e.qty}x ` : '';
      return `<div class="gd-loot-blip">${escapeHtml(when)} — ${qty}from <strong>${escapeHtml(e.mob)}</strong> in ${zone}</div>`;
    })
    .join('');
}

function gdNpcPageHtml(npc) {
  const fields = [
    gdFieldRow('Race', npc.race && escapeHtml(npc.race)),
    gdFieldRow('Class', npc.class && escapeHtml(npc.class)),
    gdFieldRow('Level', npc.level && escapeHtml(npc.level)),
    gdFieldRow('Zone', npc.zone ? gdLinkOrText('zone', npc.zone) : ''),
    gdFieldRow('Location', gdLocationHtml(npc.location)),
    gdFieldRow('Respawn timer', npc.respawn_time && escapeHtml(npc.respawn_time)),
    gdFieldRow('AC', npc.ac != null ? String(npc.ac) : ''),
    gdFieldRow('HP', npc.hp != null ? npc.hp.toLocaleString() : ''),
    gdFieldRow('Special', npc.special && escapeHtml(npc.special)),
  ]
    .filter(Boolean)
    .join('');
  const lootRows = npc.known_loot
    .map((l) => `<tr><td>${gdLinkOrText('item', l.item)}</td><td class="muted">${escapeHtml(l.rarity || '—')}</td></tr>`)
    .join('');
  const lootTable = npc.known_loot.length
    ? `<table class="ability-subtable"><thead><tr><th>item</th><th>rarity</th></tr></thead><tbody>${lootRows}</tbody></table>`
    : '<p class="muted">The wiki records no known drop table for this NPC.</p>';
  const raid = npc.categories.includes('Raid Encounters') ? ' <span class="gd-raid-badge">raid</span>' : '';
  return `
    <h2 class="gd-page-title">${escapeHtml(npc.name)}${raid}</h2>
    <dl class="gd-fields">${fields}</dl>
    <h3>Known loot</h3>
    ${lootTable}
    ${gdMobHistorySection()}
    <a class="gd-wiki-link" href="${escapeHtml(npc.url)}" target="_blank" rel="noopener">eqlwiki ↗ (backup / full page)</a>
  `;
}

// Every AA lives on one shared wiki page (table rows under section
// headings, not individual pages the way items/NPCs/spells get) -- see
// aadata.rs's own module doc. One constant link for all of them, not a
// per-entry `url` field the scrape never had anything to fill in.
const AA_WIKI_URL = 'https://eqlwiki.com/Alternate_Advancement';

function gdAaPageHtml(aa) {
  const fields = [
    gdFieldRow('Category', escapeHtml(aa.category)),
    gdFieldRow('Ranks', String(aa.ranks)),
    gdFieldRow('Cost per rank', escapeHtml(aa.cost_raw)),
  ]
    .filter(Boolean)
    .join('');
  const uncertain = aa.certain
    ? ''
    : '<p class="muted">The wiki table\'s rank/cost/level columns didn\'t line up cleanly for this entry -- treat the numbers above as approximate.</p>';
  return `
    <h2 class="gd-page-title">${escapeHtml(aa.name)}</h2>
    <dl class="gd-fields">${fields}</dl>
    ${aa.description ? `<p class="gd-description">${escapeHtml(aa.description)}</p>` : ''}
    ${uncertain}
    <h3>Your progress</h3>
    <div id="gd-aa-owned" class="muted">Loading&hellip;</div>
    <a class="gd-wiki-link" href="${AA_WIKI_URL}" target="_blank" rel="noopener">eqlwiki ↗ (backup / full AA list)</a>
  `;
}

// Fetched fresh per page open, not cached in gdData -- unlike the wiki
// catalog, "which ranks you actually own" is session-live and can change
// mid-visit (buy a rank, come back to this same page). Mirrors
// fillItemLootHistory's own token-guarded shape.
async function fillAaOwnership(aaName, token) {
  let log;
  try {
    log = await invoke('get_aa_log');
  } catch (err) {
    if (token === gdOpenToken) {
      const slot = document.getElementById('gd-aa-owned');
      if (slot) {
        slot.classList.remove('muted');
        slot.innerHTML = `<p class="muted">Couldn't load your AA history: ${escapeHtml(String(err))}</p>`;
      }
    }
    return;
  }
  if (token !== gdOpenToken) return;
  const slot = document.getElementById('gd-aa-owned');
  if (!slot) return;
  slot.classList.remove('muted');
  const mine = log.grants.filter((g) => g.name === aaName);
  if (!mine.length) {
    slot.innerHTML = '<p class="muted">Not purchased yet this session.</p>';
    return;
  }
  const best = mine.reduce((a, b) => (b.rank > a.rank ? b : a));
  const maxRank = best.max_rank ? ` / ${best.max_rank}` : '';
  slot.innerHTML = `<p>You own rank <b>${best.rank}${maxRank}</b>, first purchased ${escapeHtml(new Date(mine[0].ts_ms).toLocaleString())}.</p>`;
}

function gdSpellPageHtml(spell) {
  const fields = [
    gdFieldRow('Class(es)', escapeHtml(spell.classes.map((c) => (c.level != null ? `${c.class} ${c.level}` : c.class)).join(', ')) || ''),
    gdFieldRow('Skill', spell.skill && escapeHtml(spell.skill.replace(/\[\[|\]\]/g, '').replace(/^Skill /, ''))),
    gdFieldRow('Mana', spell.mana != null ? String(spell.mana) : ''),
    gdFieldRow('Casting time', spell.casting_time != null ? `${spell.casting_time}s` : ''),
    gdFieldRow('Recast time', spell.recast_time != null ? `${spell.recast_time}s` : ''),
    gdFieldRow('Range', spell.range != null ? String(spell.range) : ''),
    gdFieldRow('Duration', spell.duration && escapeHtml(spell.duration)),
    gdFieldRow('Target', spell.target_type && escapeHtml(spell.target_type)),
    gdFieldRow('Resist', spell.resist && escapeHtml(spell.resist)),
    gdFieldRow('Era', spell.era && escapeHtml(spell.era)),
  ]
    .filter(Boolean)
    .join('');
  const wikiUrl = spell.url || `https://eqlwiki.com/${encodeURIComponent(spell.name.replace(/ /g, '_'))}`;
  return `
    <div class="gp-detail-head">
      ${spell.icon ? `<img class="gp-detail-icon" src="planner/icons/${encodeURIComponent(spell.icon)}" alt="">` : ''}
      <div class="gd-page-title">${escapeHtml(spell.name)}</div>
    </div>
    <dl class="gd-fields">${fields}</dl>
    ${spell.description ? `<p class="gd-description">${escapeHtml(spell.description)}</p>` : ''}
    <h3>Effects</h3>
    ${spellEffectDetailHtml(spell.id)}
    <h3>Confirmed known</h3>
    <div id="gd-spell-owned" class="muted">Loading&hellip;</div>
    <a class="gd-wiki-link" href="${escapeHtml(wikiUrl)}" target="_blank" rel="noopener">eqlwiki ↗ (backup)</a>
  `;
}

// Same shape as fillAaOwnership -- "have you actually confirmed-memorized
// this spell" is session-live, fetched fresh per open rather than cached.
async function fillSpellOwnership(spellName, token) {
  let entries;
  try {
    entries = await invoke('get_spellbook');
  } catch (err) {
    if (token === gdOpenToken) {
      const slot = document.getElementById('gd-spell-owned');
      if (slot) {
        slot.classList.remove('muted');
        slot.innerHTML = `<p class="muted">Couldn't load your spellbook: ${escapeHtml(String(err))}</p>`;
      }
    }
    return;
  }
  if (token !== gdOpenToken) return;
  const slot = document.getElementById('gd-spell-owned');
  if (!slot) return;
  slot.classList.remove('muted');
  slot.innerHTML = spellOwnershipHtml(entries.find((e) => e.name === spellName));
}

// Shared by both "Confirmed known" slots -- the Game Data spell page's
// own (fillSpellOwnership, above) and the by-class picker's inline detail
// panel (openGdSpellDetail) -- so the two can never phrase the same
// Known/Possible/neither fact differently.
function spellOwnershipHtml(mine) {
  if (!mine) return '<p class="muted">No evidence yet this session.</p>';
  if (mine.confidence === 'known') {
    return `<p>${spellConfidenceTag('known')} since ${escapeHtml(new Date(mine.first_seen_ms).toLocaleString())}.</p>`;
  }
  return `<p>${spellConfidenceTag('possible')} -- began ${escapeHtml(new Date(mine.first_seen_ms).toLocaleString())}, never confirmed finished.</p>`;
}

// ---------------------------------------------------------------- character module

// Race isn't something any log line states -- unlike level or class,
// there's no "You are a Dark Elf" the parser could ever pick up -- so this
// is set by hand rather than inferred, and lives here (not as gear-
// planner-local state) specifically so the Character tab and the Gear
// Planner's own race picker are two views onto one fact, not two
// independent settings that can quietly disagree. '' means not set.
let charRace = '';

function setCharRace(value) {
  charRace = value;
  // Keep both selects in sync -- whichever one a change came from, the
  // other's displayed value would otherwise silently go stale until its
  // own module happened to re-render.
  const gpSelect = document.getElementById('gp-race');
  const charSelect = document.getElementById('char-race');
  if (gpSelect) gpSelect.value = value;
  if (charSelect) charSelect.value = value;
  if (gpRecommendations !== null) {
    gpExpandedSlot = null;
    gpDetailItem = null;
    refreshGearPlanner();
  }
  if (csShowing('sheet')) refreshCpEstimate();
}

async function refreshCharacter() {
  if (!csShowing('sheet')) return;

  const race = document.getElementById('char-race');
  if (race.options.length === 0) {
    race.innerHTML = '<option value="">— not set —</option>' + GP_ALL_RACES.map((r) => `<option value="${escapeHtml(r)}">${escapeHtml(r)}</option>`).join('');
    race.onchange = () => setCharRace(race.value);
  }
  race.value = charRace;

  let classes = [];
  try {
    classes = await invoke('get_default_gear_classes', { name: 'You' });
  } catch {
    classes = [];
  }
  if (!csShowing('sheet')) return;
  el('char-config').textContent = classes.length
    ? `Confirmed class configuration: ${classes.join(' / ')}`
    : 'No confirmed class configuration yet -- fight a bit and check back.';

  renderCpRoster();
  const estBtn = document.getElementById('cp-estimate');
  // Bound once (checked via a marker property, not `.onclick` -- that's
  // reassigned as a no-op nowhere else here, so checking it would be fine
  // too, but a dedicated flag reads clearer about *why* this only runs
  // once per page load).
  if (estBtn && !estBtn.dataset.bound) {
    estBtn.dataset.bound = '1';
    estBtn.onclick = estimateCpLevels;
  }
  refreshCpEstimate();
}

// ---------------------------------------------------------------- character planner

// EQL lets you level up to 3 classes at once, and a trio always levels
// together -- but each class remembers its own level once reached (see
// crates/app/src/character.rs's module doc for the full mechanic). So the
// roster below tracks a level for all 16 classes (whatever you've ever
// trained), separately from cpClasses -- which 3 of those 16 are the
// *active* trio right now, the only ones whose race+class adds actually
// land on the sheet.
//
// In-memory only, by design: nothing here is written to disk, so a
// relaunch always starts with an empty roster, the same "purges every
// start" stance `history`'s module doc already takes for parse history.
let cpClasses = []; // up to 3 full class names, selection order
// Class name -> level (1-50), for all 16 -- not just cpClasses. Populated
// lazily per row (renderCpRoster reads `?? 1`, so an untouched class
// simply reads as 1 rather than needing to be pre-seeded) and in bulk by
// the Estimate button (see estimateCpLevels) -- always a starting guess
// meant to be hand-corrected per class, never treated as a confirmed fact
// the way get_default_gear_classes' own confirmed configuration is.
let cpLevels = {};
let cpEstimate = null; // last CharacterEstimateDto from the backend, or null

// The Estimate button's own logic. Baseline: every class starts at 10 --
// EQL levels every class together up to 10 before a trio is even a real
// choice, so 10 is the one number that's true for a class with zero
// evidence, not a guess (see character.rs's module doc for the full
// mechanic). Above that, only a *confirmed* class configuration
// (get_class_configurations -- real, unambiguous cast evidence, not an
// assumed pairing) can raise a class's estimate, and only up to the
// highest level actually observed during that specific trio's own played
// windows (ClassConfigurationDto.level_range's upper bound). A class that
// shows up in more than one confirmed trio over the file's history (kept
// as Wiz/Enc/Dru for a while, later swapped to Wiz/Enc/Mag) takes the
// higher of the two estimates for Wiz and Enc, since a real level only
// ever goes up -- and Mag/Dru each keep whatever their own trio's evidence
// supports independently, never borrowing from a trio they weren't part
// of. A class with no confirmed trio at all -- no cast ever unambiguously
// resolved it -- has nothing to raise it above 10, so it stays there.
async function estimateCpLevels() {
  const estimate = {};
  GP_ALL_CLASSES.forEach((c) => { estimate[c] = 10; });
  try {
    const dto = await invoke('get_class_configurations', { name: 'You' });
    for (const cfg of dto.configurations) {
      if (!cfg.level_range) continue;
      const best = cfg.level_range[1]; // highest observed within this trio's own windows
      for (const c of cfg.classes) {
        if (best > estimate[c]) estimate[c] = best;
      }
    }
  } catch {
    // Fall through with the level-10-everywhere baseline -- no
    // configuration evidence is a real, if uninformative, answer, not a
    // reason to leave the roster stale.
  }
  cpLevels = estimate;
  renderCpRoster();
  refreshCpEstimate();
}

function renderCpRoster() {
  const box = document.getElementById('cp-roster');
  if (!box) return;
  const atCap = cpClasses.length >= GP_MAX_CLASSES;
  box.innerHTML = GP_ALL_CLASSES.map((c) => {
    const on = cpClasses.includes(c);
    const disabled = !on && atCap;
    return `<div class="cp-roster-row">
      <button class="gp-class-chip${on ? ' confirmed' : ''}" data-class="${escapeHtml(c)}"${disabled ? ' disabled title="Already at 3 active -- every class plays exactly 3 at once"' : ' title="Mark as one of your 3 currently active classes"'}>${escapeHtml(c)}</button>
      <input type="number" min="1" max="50" step="1" data-class="${escapeHtml(c)}" value="${cpLevels[c] ?? 1}">
    </div>`;
  }).join('');
  box.onclick = (e) => {
    const btn = e.target.closest('.gp-class-chip');
    if (!btn || btn.disabled) return;
    const c = btn.dataset.class;
    if (cpClasses.includes(c)) {
      cpClasses = cpClasses.filter((x) => x !== c);
    } else if (cpClasses.length < GP_MAX_CLASSES) {
      cpClasses = [...cpClasses, c];
    }
    renderCpRoster();
    refreshCpEstimate();
  };
  box.oninput = (e) => {
    const input = e.target.closest('input[data-class]');
    if (!input) return;
    let v = Math.round(Number(input.value));
    if (!Number.isFinite(v)) v = 1;
    v = Math.min(50, Math.max(1, v));
    cpLevels[input.dataset.class] = v;
    // Only the 3 active classes' levels feed the sheet -- no need to
    // re-fetch over an inactive class's level changing.
    if (cpClasses.includes(input.dataset.class)) refreshCpEstimate();
  };
}

// Sums stats across whatever's currently resolved onto the Gear Planner's
// own doll -- equipped (gpEquipped) > manually chosen (gpChosen) > top
// recommendation, the exact same priority renderGpDoll uses via
// gpChosenItem, so the Character Planner's gear column can never silently
// disagree with what the doll itself shows as worn. Empty until the Gear
// Planner has resolved at least once this session (gpRecommendations
// starts null) -- the sheet still renders fine with an all-zero gear
// column in that case, same as before gear support existed at all.
function gpCurrentGearStats() {
  const totals = {};
  if (!gpRecommendations) return totals;
  const bySlot = new Map(gpRecommendations.map((r) => [r.slot, r.items]));
  const primaryItem = gpChosenItem('PRIMARY', bySlot.get('PRIMARY') || []);
  const primaryIsTwoHand = gpIsTwoHand(primaryItem);
  for (const entry of GP_SLOTS) {
    if (!entry) continue;
    const [key] = entry;
    // A two-handed Primary occupies Secondary without anything separate
    // actually being worn there -- see renderGpDoll's own comment on the
    // same rule; counting Secondary's own top pick here too would double
    // an item that isn't really equipped.
    if (key === 'SECONDARY' && primaryIsTwoHand) continue;
    const item = gpChosenItem(key, bySlot.get(key) || []);
    if (!item) continue;
    for (const [stat, val] of Object.entries(item.stats || {})) {
      totals[stat] = (totals[stat] || 0) + val;
    }
  }
  return totals;
}

// Bumped on every fetch, checked on return -- a race switch or class pick
// while a fetch is in flight shouldn't let a stale response overwrite a
// newer one that already landed (or is about to).
let cpEstimateToken = 0;

async function refreshCpEstimate() {
  if (!csShowing('sheet')) return;
  if (!charRace || !cpClasses.length) {
    cpEstimate = null;
    renderCpSheet();
    return;
  }
  const token = ++cpEstimateToken;
  let est = null;
  try {
    est = await invoke('get_character_estimate', {
      race: charRace,
      classes: cpClasses,
      classLevels: cpClasses.map((c) => cpLevels[c] ?? 1),
      gear: gpCurrentGearStats(),
    });
  } catch {
    est = null;
  }
  if (token !== cpEstimateToken || !csShowing('sheet')) return;
  cpEstimate = est;
  renderCpSheet();
}

function renderCpSheet() {
  renderCsModules();
  const box = document.getElementById('cp-sheet');
  if (!box) return;
  if (!charRace) {
    box.innerHTML = '<div class="empty"><b>Set a race above</b>Race base attributes are needed before class adds and mana can be shown.</div>';
    return;
  }
  if (!cpClasses.length) {
    box.innerHTML = '<p class="muted">Mark up to 3 classes above as active to see a full sheet.</p>';
    return;
  }
  if (!cpEstimate) {
    box.innerHTML = '<p class="muted">Loading&hellip;</p>';
    return;
  }
  const est = cpEstimate;
  const cols = est.classes;

  let h = '';
  if (cols.length < 3) {
    h += `<p class="muted">${3 - cols.length} active class slot${cols.length === 2 ? '' : 's'} empty -- totals below only count what's marked active.</p>`;
  }
  const limitNote = est.limiting_class
    ? `capped by <b>${escapeHtml(est.limiting_class)}</b>`
    : cols.length > 1
      ? `${cols.length} classes tied at the lowest`
      : '';
  h += `<p class="cp-level-summary">Character level <b>${est.character_level}</b>${limitNote ? ` -- ${limitNote}` : ''}</p>`;

  h += `<table class="sheet"><thead><tr><th></th><th>Base</th>${cols.map((c) => `<th>${escapeHtml(c)}</th>`).join('')}<th>Naked</th><th>Gear</th><th>Total</th></tr></thead><tbody>`;
  for (const row of est.attrs) {
    const over = row.total > est.attr_cap;
    h += `<tr><td>${escapeHtml(row.attr)}</td><td class="g">${row.base}</td>` +
      row.class_adds.map((v) => `<td class="g">${v ? '+' + v : '·'}</td>`).join('') +
      `<td>${row.naked}</td>` +
      `<td class="gear">${row.gear ? (row.gear > 0 ? '+' : '') + Math.round(row.gear) : '·'}</td>` +
      `<td class="tot${over ? ' over' : ''}">${Math.round(row.total)}${over ? '‡' : ''}</td></tr>`;
  }
  h += '</tbody></table>';

  h += '<div class="data-note">';
  if (est.mana.length) {
    h += '<b>Mana:</b> ' +
      est.mana.map((m) => `${escapeHtml(m.class)} (${m.casting_stat}) ${Math.round(m.pool)}${m.counted ? '' : ' <span class="muted">(not counted)</span>'}`).join(', ') +
      ` -- pool comes from your <b>two highest</b> classes only, total <b>${Math.round(est.total_mana)}</b>. Includes gear.<br>`;
  }
  if (est.attrs.some((r) => r.total > est.attr_cap)) {
    h += `‡ over the reported ${est.attr_cap} ceiling. Players report it; it isn't confirmed in the client, so nothing here is clamped.<br>`;
  }
  if (est.bad_class_adds.length) {
    h += `<span style="color:var(--warn)"><b>chardata is off:</b> ${est.bad_class_adds.map(escapeHtml).join(', ')} don't add up to what every other class does.</span><br>`;
  }
  h += 'Attribute numbers are <b>classic EQ values, unverified for EQL</b> -- eqlwiki doesn\'t publish them. Treat this as an estimate, not a promise.';
  h += '</div>';

  box.innerHTML = h;
}

// The 4-module quick-glance sheet's top two modules (Char Vitals,
// Stat/Resist) -- reads the exact same cpEstimate as renderCpSheet's own
// detailed per-class table, just condensed to one number per line. Called
// alongside renderCpSheet on every estimate refresh (including the null
// case -- see refreshCpEstimate) so the two never show stats for
// different characters.
function csRow(label, val, suffix = '') {
  const shown = val === null || val === undefined ? '&mdash;' : Math.round(val) + suffix;
  return `<div class="cs-label">${escapeHtml(label)}</div><div class="cs-value">${shown}</div>`;
}

function renderCsModules() {
  const vBox = document.getElementById('cs-vitals');
  const srBox = document.getElementById('cs-statresist');
  if (!vBox || !srBox) return;
  const est = cpEstimate;
  if (!est) {
    const msg = '<p class="muted">No estimate yet.</p>';
    vBox.innerHTML = msg;
    srBox.innerHTML = msg;
    return;
  }
  const sep = '<div class="cs-sep"></div>';

  vBox.innerHTML =
    csRow('HP', est.vitals.hp) +
    csRow('Mana', est.total_mana) +
    csRow('Endurance', est.vitals.endurance) +
    sep +
    csRow('AC', est.vitals.ac) +
    csRow('Attack', est.vitals.attack) +
    csRow('Velocity', est.vitals.velocity) +
    sep +
    csRow('HP Regen', est.vitals.hp_regen) +
    csRow('Mana Regen', est.vitals.mana_regen) +
    csRow('End Regen', est.vitals.end_regen);

  // est.attrs is keyed by ATTRS' own order (STR/STA/AGI/DEX/WIS/INT/CHA)
  // -- re-ordered here to the order the user asked for on the sheet.
  const attrTotal = (code) => est.attrs.find((r) => r.attr === code)?.total ?? null;
  srBox.innerHTML =
    csRow('Str', attrTotal('STR')) +
    csRow('Stam', attrTotal('STA')) +
    csRow('Int', attrTotal('INT')) +
    csRow('Wis', attrTotal('WIS')) +
    csRow('Agi', attrTotal('AGI')) +
    csRow('Dex', attrTotal('DEX')) +
    csRow('Cha', attrTotal('CHA')) +
    sep +
    csRow('SV Magic', est.resists.magic) +
    csRow('SV Fire', est.resists.fire) +
    csRow('SV Cold', est.resists.cold) +
    csRow('SV Disease', est.resists.disease) +
    csRow('SV Poison', est.resists.poison) +
    csRow('SV Void', est.resists.void);
}

// ---------------------------------------------------------------- AA history

// Bumped on every fetch, same race-guard shape as cpEstimateToken above.
let aaLogToken = 0;

async function refreshAaLog() {
  if (!csShowing('aa')) return;
  const box = el('cs-aa');
  if (!box) return;
  // Not awaited -- same "load in the background, re-render once it lands"
  // shape refreshGearPlanner's own ensureGameData call uses, so an AA
  // name's link to its Game Data page upgrades from plain text to
  // clickable without needing Game Data visited first.
  ensureGameData().then(() => {
    if (csShowing('aa')) refreshAaLog();
  });
  const token = ++aaLogToken;
  let log, cfgs;
  try {
    [log, cfgs] = await Promise.all([invoke('get_aa_log'), invoke('get_class_configurations', { name: 'You' })]);
  } catch (e) {
    if (token !== aaLogToken || !csShowing('aa')) return;
    box.innerHTML = `<p class="muted">Couldn't load AA history: ${escapeHtml(String(e))}</p>`;
    return;
  }
  if (token !== aaLogToken || !csShowing('aa')) return;

  if (!log.grants.length) {
    box.innerHTML = '<p class="muted">No AA purchases parsed yet.</p>';
    return;
  }

  // Every class ever confirmed active for "You", across every configuration
  // this log has evidence for -- not just the currently active trio. An AA
  // whose category lands here (but isn't in cpClasses right now) is a real,
  // still-owned purchase from a build you've since swapped away from, not
  // a stale or bad entry -- see aaClassTag.
  const everPlayed = new Set();
  for (const cfg of cfgs.configurations) {
    for (const c of cfg.classes) everPlayed.add(c);
  }

  const rows = [...log.grants].reverse(); // newest purchase first
  let h = `<p class="cp-level-summary"><b>${log.grants.length}</b> AA rank${log.grants.length === 1 ? '' : 's'} purchased this session, <b>${log.total_spent}</b> ability point${log.total_spent === 1 ? '' : 's'} spent.</p>`;
  h += '<table class="sheet aa-table"><thead><tr><th>When</th><th>Ability</th><th class="num">Rank</th><th class="num">Cost</th><th>Class</th><th>Links to</th></tr></thead><tbody>';
  for (const g of rows) {
    const rankLabel = g.max_rank && g.max_rank > 1 ? `${g.rank} / ${g.max_rank}` : String(g.rank);
    h += `<tr>
      <td>${escapeHtml(new Date(g.ts_ms).toLocaleString())}</td>
      <td${g.description ? ` title="${escapeHtml(g.description)}"` : ''}>${gdLinkOrText('aa', g.name)}</td>
      <td class="num">${escapeHtml(rankLabel)}</td>
      <td class="num">${g.cost}</td>
      <td>${aaClassTag(g.category, everPlayed)}</td>
      <td>${aaRelevantStatsHtml(g.relevant_stats)}</td>
    </tr>`;
  }
  h += '</tbody></table>';
  box.innerHTML = h;
}

// Best-effort cross-link from an owned AA to the Character sheet's own
// Vitals/Stat-Resist rows it may affect -- see aadata.rs's own doc for
// exactly how `relevant_stats` is derived (keyword-matched from the AA's
// description, not a guaranteed or numeric effect). Shown as plain
// stat-name pills, not a link -- there's no dedicated page per stat row
// to jump to, just an at-a-glance "this AA is probably relevant to..."
// note.
function aaRelevantStatsHtml(stats) {
  if (!stats || !stats.length) return '<span class="muted">&middot;</span>';
  return stats.map((s) => `<span class="aa-tag aa-tag-stat">${escapeHtml(s)}</span>`).join(' ');
}

// A purchased AA's category names the one class it required *at the time
// you bought it* -- "general"/"archetype" AAs need no specific class. A
// purchase is never undone by a later loadout swap, so a category that
// isn't one of your currently active classes (cpClasses) doesn't mean
// anything is wrong -- it means the AA came from an earlier build, which
// `everPlayed` (this log's own confirmed class-configuration history)
// still accounts for. Only a category that's never once been a confirmed
// class in this log at all is worth calling out: either the purchase
// predates this log file, or the catalog's category doesn't match what
// actually happened -- shown as "unconfirmed", not an error.
function aaClassTag(category, everPlayed) {
  if (category == null) return '<span class="aa-tag aa-tag-unknown">uncatalogued</span>';
  if (category === 'general' || category === 'archetype') return `<span class="aa-tag aa-tag-general">${escapeHtml(category)}</span>`;
  if (cpClasses.includes(category)) return `<span class="aa-tag aa-tag-active">${escapeHtml(category)}</span>`;
  if (everPlayed.has(category)) return `<span class="aa-tag aa-tag-past">${escapeHtml(category)} &middot; past build</span>`;
  return `<span class="aa-tag aa-tag-unknown">${escapeHtml(category)} &middot; unconfirmed</span>`;
}

// Bumped on every fetch, same guard shape as aaLogToken above.
let spellbookToken = 0;

async function refreshSpellbook() {
  if (!csShowing('spellbook')) return;
  const box = el('cs-spellbook');
  if (!box) return;
  ensureGameData().then(() => {
    if (csShowing('spellbook')) refreshSpellbook();
  });
  const token = ++spellbookToken;
  let entries;
  try {
    entries = await invoke('get_spellbook');
  } catch (e) {
    if (token !== spellbookToken || !csShowing('spellbook')) return;
    box.innerHTML = `<p class="muted">Couldn't load your spellbook: ${escapeHtml(String(e))}</p>`;
    return;
  }
  if (token !== spellbookToken || !csShowing('spellbook')) return;

  if (!entries.length) {
    box.innerHTML = '<p class="muted">Nothing scribed or memorized yet this session.</p>';
    return;
  }

  const knownCount = entries.filter((s) => s.confidence === 'known').length;
  const possibleCount = entries.length - knownCount;
  let h = `<p class="cp-level-summary"><b>${knownCount}</b> known this session`;
  if (possibleCount) h += `, <b>${possibleCount}</b> possible (began scribing or memorizing, never confirmed finished)`;
  h += '.</p>';
  h += '<table class="sheet aa-table"><thead><tr><th>Confidence</th><th>When</th><th>Spell</th><th>Class(es)</th><th class="num">Mana</th><th class="num">Cast</th><th>Effect</th></tr></thead><tbody>';
  for (const s of entries) {
    const classes = s.classes.map((c) => (c.level != null ? `${c.class} ${c.level}` : c.class)).join(', ');
    h += `<tr>
      <td>${spellConfidenceTag(s.confidence)}</td>
      <td>${escapeHtml(new Date(s.first_seen_ms).toLocaleString())}</td>
      <td>${gdLinkOrText('spell', s.name)}</td>
      <td>${escapeHtml(classes || '—')}</td>
      <td class="num">${s.mana != null ? s.mana : '—'}</td>
      <td class="num">${s.casting_time != null ? `${s.casting_time}s` : '—'}</td>
      <td class="gd-spell-effect"${s.description ? ` title="${escapeHtml(s.description)}"` : ''}>${escapeHtml(s.description || '—')}</td>
    </tr>`;
  }
  h += '</tbody></table>';
  box.innerHTML = h;
}

// "Known" (a scribe or memorize *finished*, unambiguous) vs "possible"
// (only a "Beginning to..." line ever landed for it -- a real attempt,
// but never confirmed complete: could be a genuine interrupt, or just
// the log ending mid-action). See ingest::SpellLog's own doc. Reuses the
// AA subpage's own tag look (aa-tag) rather than inventing a new visual
// vocabulary for the same "how sure are we" idea.
function spellConfidenceTag(confidence) {
  return confidence === 'known'
    ? '<span class="aa-tag aa-tag-active">known</span>'
    : '<span class="aa-tag aa-tag-unknown">possible</span>';
}

// ---------------------------------------------------------------- gear planner

// A native module, not the standalone tool embedded -- see
// crates/app/src/gearplanner.rs's doc for what's ported (item filtering,
// the class-weighted scoring vector) and what deliberately isn't yet
// (hand-pairing, LORE-coverage-aware picking, exaltation auto-assignment).
// The one thing this has that the standalone tool never could: gpClasses
// defaults from get_default_gear_classes, i.e. whatever configuration
// classdetect has actually confirmed for "You" from the live parse,
// instead of asking you to re-tell the planner what you're playing.

let gpClasses = [];
// No picker for this, and deliberately no era value held here at all --
// omitting maxEra (see the invoke call below) lets gearplanner::in_era's
// own CURRENT_ERA default decide, so "what's currently live" is the
// backend's call to own and update, not a string this file would have had
// to keep in sync with it.
let gpRecommendations = null; // Vec<SlotRecommendationDto>, or null while loading
let gpExpandedSlot = null;
let gpDetailItem = null; // the ItemDto currently shown in the detail panel, or null
// slot key -> item id. What you actually picked from a slot's alt list,
// overriding the top-scored default for *that slot's square* specifically
// (renderGpSquare falls back to items[0] when a slot has no entry here, or
// when the id it names isn't in the current results -- e.g. after a
// class/race change re-filters the list). Picking a new alt is meant to
// change what's shown as equipped, not just what the detail panel says.
let gpChosen = {};
// slot key -> full ItemDto, from a loaded /outputfile inventory dump (see
// the inv-toast wiring near the bottom of this file). Takes priority over
// both gpChosen and the recommendation list in gpChosenItem below -- a
// real equipped item isn't necessarily one of a slot's top-scored
// candidates at all, so it can't be represented as just an id to look up
// there. Manually picking a different alt for a slot clears its entry
// here (see doll.onclick): browsing away from what you actually have on
// is exactly what that action means.
let gpEquipped = {};
let gpDefaultSource = null; // 'confirmed' | 'none' -- for the note under the class chips
// Your most recently observed level (backend: Ingest::levels), re-fetched
// each refresh -- feeds get_gear_recommendations/get_gear_weights so
// derived_weights can score INT/WIS as actual mana-pool value instead of
// its flat per-class fallback. null whenever no level.up line has fired
// yet this session (commonly just "you've been this level the whole log
// file"), in which case the backend already degrades gracefully on its
// own -- this file doesn't need its own fallback logic for that.
let gpLevel = null;
let gpWeights = null; // derived-from-classes vector, as last fetched from the backend
let gpCustomWeights = null; // null = use gpWeights; once the compact weight row is edited, a full override object
// Grouped for the weight editor: defensive/resource stats, then the seven
// core attributes, then the two non-stat scoring terms (a weapon's
// dmg/delay ratio, and the flat bonus per scored focus/click/worn slot --
// see score_item's doc on the backend for why proc doesn't count). Labels
// are display-only -- `key` is still the exact field name
// gearplanner::derived_weights/score_item use, just shown as something
// clearer than the raw key.
const GP_WEIGHT_GROUPS = [
  ['stats', [
    ['AC', 'AC'], ['HP', 'HP'], ['MANA', 'Mana'],
  ]],
  ['attributes', [
    ['STR', 'Str'], ['STA', 'Sta'], ['AGI', 'Agi'], ['DEX', 'Dex'], ['WIS', 'Wis'], ['INT', 'Int'], ['CHA', 'Cha'],
  ]],
  ['other', [
    ['RATIO', 'Wep Ratio'], ['EFFECT', 'Focus/Click/Worn'],
  ]],
];

// Alphabetical, laid out column-major (see .gp-classes' grid-auto-flow) --
// column 1 is Bard/Beastlord/Berserker/Cleric, column 2 is Druid/
// Enchanter/Magician/Monk, and so on, 4 classes per column.
const GP_ALL_CLASSES = [
  'Bard', 'Beastlord', 'Berserker', 'Cleric',
  'Druid', 'Enchanter', 'Magician', 'Monk',
  'Necromancer', 'Paladin', 'Ranger', 'Rogue',
  'Shadow Knight', 'Shaman', 'Warrior', 'Wizard',
];
const GP_ALL_RACES = [
  'Human', 'Barbarian', 'Erudite', 'Wood Elf', 'High Elf', 'Dark Elf', 'Halfling', 'Dwarf',
  'Troll', 'Ogre', 'Gnome', 'Iksar', 'Vah Shir', 'Froglok', 'Half Elf',
];
// Every class plays exactly 3 at once above level 10 -- see
// eqlp_session::classdetect's doc, the same fixed-cardinality rule the
// Combat module's "configurations" feature is built around. The class
// picker enforces it here too rather than silently scoring against an
// impossible 4-class loadout.
const GP_MAX_CLASSES = 3;

async function refreshGearPlanner() {
  if (!csShowing('gear')) return;

  // Not awaited -- the doll/detail panel's own drop-source links
  // (gdZoneOrMobLink et al.) need gdData loaded to render as real links
  // instead of plain text, but nothing else here depends on it, so this
  // shouldn't hold up the planner's own recommendations just to fetch it.
  // Re-renders once it lands so links upgrade from plain text to clickable
  // without needing a manual refresh; a no-op if gdData was already
  // loaded (Game Data visited first, or this call itself already primed
  // it) since ensureGameData resolves immediately in that case.
  ensureGameData().then(() => {
    if (csShowing('gear')) {
      renderGpDoll();
      renderGpDetail();
    }
  });

  // Only re-derive the default on a fresh open (gpRecommendations === null),
  // never on a background re-render -- otherwise switching zones on the
  // Combat tab while this module sits open behind it would silently yank
  // your manually-picked classes back to whatever's dominant right now.
  if (gpRecommendations === null && gpClasses.length === 0) {
    try {
      const defaults = await invoke('get_default_gear_classes', { name: 'You' });
      gpClasses = defaults.slice(0, GP_MAX_CLASSES);
      gpDefaultSource = defaults.length ? 'confirmed' : 'none';
    } catch {
      gpDefaultSource = 'none';
    }
  }

  // Re-fetched every refresh, not just on open -- a level.up mid-session
  // should retune INT/WIS the same way switching classes would, without
  // needing its own separate trigger.
  try {
    gpLevel = await invoke('get_current_level');
  } catch {
    gpLevel = null;
  }

  renderGpControls();

  try {
    const [recs, weights] = await Promise.all([
      invoke('get_gear_recommendations', { classes: gpClasses, race: charRace || null, maxEra: null, perSlot: 6, weights: gpCustomWeights, level: gpLevel }),
      invoke('get_gear_weights', { classes: gpClasses, level: gpLevel }),
    ]);
    gpRecommendations = recs;
    gpWeights = weights;
  } catch (err) {
    document.getElementById('gp-doll').innerHTML = `<p class="muted">Couldn't load recommendations: ${escapeHtml(String(err))}</p>`;
    return;
  }
  if (!csShowing('gear')) return; // switched tabs mid-fetch

  renderGpDoll();
  renderGpZones();
  renderGpWeights();
  renderGpDetail();
}

function renderGpControls() {
  const cls = document.getElementById('gp-classes');
  const atCap = gpClasses.length >= GP_MAX_CLASSES;
  cls.innerHTML = GP_ALL_CLASSES.map((c) => {
    const on = gpClasses.includes(c);
    const disabled = !on && atCap;
    return `<button class="gp-class-chip${on ? ' confirmed' : ''}" data-class="${escapeHtml(c)}"${disabled ? ' disabled title="Already at 3 -- every class plays exactly 3 at once"' : ''}>${escapeHtml(c)}</button>`;
  }).join('');
  cls.onclick = (e) => {
    const btn = e.target.closest('.gp-class-chip');
    if (!btn || btn.disabled) return;
    const c = btn.dataset.class;
    if (gpClasses.includes(c)) {
      gpClasses = gpClasses.filter((x) => x !== c);
    } else if (gpClasses.length < GP_MAX_CLASSES) {
      gpClasses = [...gpClasses, c];
    }
    gpExpandedSlot = null;
    gpDetailItem = null;
    refreshGearPlanner();
  };

  const note = document.getElementById('gp-classes-note');
  const classNote =
    gpDefaultSource === 'confirmed'
      ? 'pre-filled from your confirmed configuration -- click to change (max 3)'
      : gpDefaultSource === 'none'
        ? 'no confirmed configuration yet -- pick up to 3 classes, or leave empty to browse everything'
        : '';
  // Whether INT/WIS are scoring as real mana-pool value (derived_weights'
  // level-aware path) or its flat per-class fallback -- worth surfacing
  // since it silently changes item rankings, not just a cosmetic detail.
  const levelNote = gpLevel
    ? `mana weighting: level ${gpLevel}`
    : 'mana weighting: level unknown yet -- using flat INT/WIS priorities until a level.up line is seen';
  note.textContent = [classNote, levelNote].filter(Boolean).join(' — ');

  const race = document.getElementById('gp-race');
  if (race.options.length === 0) {
    race.innerHTML = '<option value="">— any —</option>' + GP_ALL_RACES.map((r) => `<option value="${escapeHtml(r)}">${escapeHtml(r)}</option>`).join('');
    race.onchange = () => setCharRace(race.value);
  }
  race.value = charRace;
}

// A compact "AC15 HP20 INT8"-style summary of an item's own stats, most
// weighted-first, capped at 3 -- what the alt list highlights between name
// and score so a ranking is explainable at a glance instead of opaque.
function gpStatSummary(item, weights) {
  const entries = Object.entries(item.stats || {});
  if (!entries.length) return '';
  entries.sort((a, b) => (weights?.[b[0]] || 0) * b[1] - (weights?.[a[0]] || 0) * a[1]);
  return entries
    .slice(0, 3)
    .map(([k, v]) => `${k}${v >= 0 ? '+' : ''}${v}`)
    .join(' ');
}

// Left-to-right, top-to-bottom exactly the physical paper-doll layout: ear/
// neck/face/head/ear, finger/wrist/arm/hands/wrist/finger, shoulders/chest/
// back/waist/legs/feet, primary/secondary/range/ammo/any/any. `null` is a
// blank cell (row 1 only has 5 real slots against every other row's 6) --
// rendered as an empty spacer so the grid stays a clean 6 columns instead
// of the fifth slot silently sliding out of alignment with the rows below.
const GP_SLOTS = [
  ['EAR1', 'Ear'], ['NECK', 'Neck'], ['FACE', 'Face'], ['HEAD', 'Head'], ['EAR2', 'Ear'], null,
  ['FINGER1', 'Finger'], ['WRIST1', 'Wrist'], ['ARMS', 'Arm'], ['HANDS', 'Hands'], ['WRIST2', 'Wrist'], ['FINGER2', 'Finger'],
  ['SHOULDERS', 'Shldr'], ['CHEST', 'Chest'], ['BACK', 'Back'], ['WAIST', 'Waist'], ['LEGS', 'Legs'], ['FEET', 'Feet'],
  ['PRIMARY', 'Prim'], ['SECONDARY', 'Sec'], ['RANGE', 'Range'], ['AMMO', 'Ammo'], ['ANY1', 'Any'], ['ANY2', 'Any'],
];

function gpItemIcon(item, cls) {
  return item.icon ? `<img class="${cls}" src="planner/icons/${encodeURIComponent(item.icon)}" alt="" loading="lazy">` : '';
}

// gpEquipped[key] if a loaded inventory dump named one, else gpChosen[key]
// if it's still in this slot's current results, else the top-scored
// default -- the one place "what's shown as equipped in this slot" gets
// decided, so the square and the open-on-click default agree.
function gpChosenItem(key, items) {
  if (gpEquipped[key]) return gpEquipped[key];
  const id = gpChosen[key];
  return (id && items.find((it) => it.id === id)) || items[0];
}

// A weapon's skill starts with "2H" ("2H Blunt", "2H Slashing", "2H
// Piercing") precisely when it's two-handed -- distinct from a 1H item
// that merely happens to list PRIMARY/SECONDARY as valid slots (dual-
// wieldable), which shouldn't lock anything.
function gpIsTwoHand(item) {
  return !!(item && item.skill && item.skill.startsWith('2H'));
}

// No name/score/source baked into the square itself -- deliberately, to
// read like the game's own inventory grid (icon + a small corner slot
// label, nothing else). The name/score/source a name-and-numbers view
// needs still exist, just one level down: hover for a tooltip, click for
// the alt list and detail panel below.
//
// `lock`, when given, means "this square isn't independently pickable" --
// currently only Secondary while Primary is two-handed (see renderGpDoll).
// It renders the primary's own icon ghosted in with a 2H badge instead of
// this slot's own top pick, since there effectively isn't one: you can't
// wear anything else there while wielding a 2-hander.
function renderGpSquare(entry, bySlot, lock) {
  if (!entry) return '<div class="gp-slot gp-slot-spacer"></div>';
  const [key, label] = entry;
  if (lock) {
    const name = lock.name || 'a two-handed weapon';
    return `<div class="gp-slot gp-slot-locked" data-slot="${key}" title="Occupied -- wielding ${escapeHtml(name)} two-handed">
      <span class="gp-slot-label">${escapeHtml(label)}</span>
      ${gpItemIcon(lock, 'gp-slot-icon gp-slot-icon-ghost')}
      <span class="gp-slot-2h">2H</span>
    </div>`;
  }
  const items = bySlot.get(key) || [];
  const top = gpChosenItem(key, items);
  const isOpen = gpExpandedSlot === key;
  const equipped = !!gpEquipped[key];
  const tooltip = top
    ? `${top.name}${equipped ? ' -- currently equipped' : ''}${top.source ? ` -- ${top.source}` : ''}`
    : 'no eligible item found';
  return `<div class="gp-slot${top ? '' : ' empty'}${isOpen ? ' expanded' : ''}${equipped ? ' gp-slot-equipped' : ''}" data-slot="${key}" title="${escapeHtml(tooltip)}">
    <span class="gp-slot-label">${escapeHtml(label)}</span>
    ${top ? gpItemIcon(top, 'gp-slot-icon') : ''}
  </div>`;
}

function renderGpAlts(key, items) {
  return `<div class="gp-slot-alts" data-slot-alts="${key}">${
    items.length
      ? items
          .map((it, i) => {
            const active = gpDetailItem && gpDetailItem.id === it.id;
            // Zone(s), then mobs, each its own line -- a long mob list on
            // one item shouldn't push a short zone name (or the stat line
            // below both) out of shape. Both link into Game Data
            // (gdLinkOrText degrades to plain text on its own if that
            // dataset hasn't loaded yet -- see its doc) rather than a
            // pre-truncated string, so CSS's own text-overflow: ellipsis
            // (already on .gp-alt-src) clips the line visually without
            // cutting a link's text off mid-word.
            const zoneLine = it.zones.length ? `<div class="gp-alt-src">${gdZoneOrMobLinkList(it.zones)}</div>` : '';
            const mobLine = it.mobs.length ? `<div class="gp-alt-src">${gdLinkList('npc', it.mobs)}</div>` : '';
            return `<div class="gp-alt-row${active ? ' active' : ''}" data-item-id="${escapeHtml(it.id)}">${gpItemIcon(it, 'gp-slot-icon-sm')}<div class="gp-alt-main"><div class="gp-alt-top"><span class="name">${i === 0 ? '★ ' : ''}${escapeHtml(it.name)}</span><span class="score">${it.score.toFixed(1)}</span></div>${zoneLine}${mobLine}<span class="stats">${escapeHtml(gpStatSummary(it, gpWeights))}</span></div></div>`;
          })
          .join('')
      : '<p class="muted">No eligible items for this slot with the current class/race filter.</p>'
  }</div>`;
}

function renderGpDoll() {
  const bySlot = new Map((gpRecommendations || []).map((r) => [r.slot, r.items]));
  const doll = document.getElementById('gp-doll');

  // A two-handed Primary occupies Secondary too -- real EQ inventory rule,
  // not a scoring preference, so it's enforced here rather than left to
  // the recommendation ranking to happen to work out. If Secondary's alt
  // list was open when this became true (you just picked a 2-hander while
  // it was expanded), there's nothing left to pick there -- collapse it.
  const primaryItem = gpChosenItem('PRIMARY', bySlot.get('PRIMARY') || []);
  const primaryIsTwoHand = gpIsTwoHand(primaryItem);
  if (primaryIsTwoHand && gpExpandedSlot === 'SECONDARY') {
    gpExpandedSlot = null;
    gpDetailItem = null;
  }

  // GP_SLOTS is already laid out 6-wide per visual row -- chunk it back
  // into those rows and render each as its own wrapper (see #gp-doll's
  // doc in styles.css for why: a flat list let an expanded panel bump the
  // rest of its own row down into the row below it).
  const rows = [];
  for (let i = 0; i < GP_SLOTS.length; i += 6) rows.push(GP_SLOTS.slice(i, i + 6));

  doll.innerHTML = rows
    .map((row) => {
      const squares = row
        .map((entry) => renderGpSquare(entry, bySlot, entry && entry[0] === 'SECONDARY' && primaryIsTwoHand ? primaryItem : null))
        .join('');
      // Secondary renders locked, not expandable, while occupied -- so it
      // never contributes an alt panel of its own in that state.
      const openEntry = row.find((entry) => entry && entry[0] === gpExpandedSlot && !(entry[0] === 'SECONDARY' && primaryIsTwoHand));
      const alts = openEntry ? renderGpAlts(openEntry[0], bySlot.get(openEntry[0]) || []) : '';
      return `<div class="gp-doll-row">${squares}</div>${alts}`;
    })
    .join('');

  doll.onclick = (e) => {
    const altRow = e.target.closest('.gp-alt-row');
    if (altRow) {
      const key = e.target.closest('.gp-slot-alts').dataset.slotAlts;
      const item = (bySlot.get(key) || []).find((it) => it.id === altRow.dataset.itemId);
      if (item) {
        // Picking an alternative both (a) becomes the square's shown item
        // -- gpChosen is what renderGpSquare reads, so this isn't just a
        // detail-panel preview -- and (b) closes the slot back up, since
        // you've made your choice; the detail panel is where it stays
        // visible from here, no need to also click the slot to dismiss
        // the list. Also clears any loaded-inventory entry for this slot --
        // gpEquipped otherwise outranks gpChosen entirely (see
        // gpChosenItem), so picking an alt here would silently do nothing
        // if a real equipped item were still on file for it.
        gpChosen[key] = item.id;
        delete gpEquipped[key];
        gpDetailItem = item;
        gpExpandedSlot = null;
        renderGpDoll();
        renderGpDetail();
      }
      return;
    }
    const slotEl = e.target.closest('.gp-slot:not(.gp-slot-spacer)');
    if (!slotEl || slotEl.classList.contains('gp-slot-locked')) return;
    const key = slotEl.dataset.slot;
    if (gpExpandedSlot === key) {
      gpExpandedSlot = null;
      gpDetailItem = null;
    } else {
      gpExpandedSlot = key;
      gpDetailItem = gpChosenItem(key, bySlot.get(key) || []) || null;
    }
    renderGpDoll();
    renderGpDetail();
  };
}

function renderGpZones() {
  const el = document.getElementById('gp-zones');
  const counts = new Map(); // zone -> [{slot, item}]
  for (const { slot, items } of gpRecommendations || []) {
    const top = items[0];
    if (!top || !top.source || top.source.startsWith('quest:') || top.source.startsWith('vendor:') || top.source === 'crafted') continue;
    const zone = top.source.split(' — ')[0];
    if (!counts.has(zone)) counts.set(zone, []);
    counts.get(zone).push({ slot, item: top.name });
  }
  const rows = [...counts.entries()].sort((a, b) => b[1].length - a[1].length);
  if (!rows.length) {
    el.innerHTML = '<p class="muted">No mob-drop recommendations to summarize yet -- pick your classes above.</p>';
    return;
  }
  el.innerHTML = rows
    .map(
      ([zone, items]) =>
        `<div class="zone-row"><span>${escapeHtml(zone)}</span><span class="zone-count">${items.length}</span></div>`,
    )
    .join('');
}

// The scoring vector currently in force -- shown at the bottom of the
// module rather than up top, since it explains the ranking above it
// (both the doll's top picks and the alt list's stat highlights) instead
// of asking to be read first.
function renderGpWeights() {
  const el = document.getElementById('gp-weights');
  const active = gpCustomWeights || gpWeights;
  if (!active) {
    el.innerHTML = '';
    return;
  }
  el.innerHTML =
    GP_WEIGHT_GROUPS.map(
      ([group, keys]) =>
        `<div class="gp-weight-group"><span class="label">${escapeHtml(group)}</span>${keys
          .map(
            ([k, display]) =>
              `<label class="w-item">${escapeHtml(display)}<input type="number" step="0.1" data-weight="${k}" value="${(active[k] ?? 0).toFixed(2)}"></label>`,
          )
          .join('')}</div>`,
    ).join('') + (gpCustomWeights ? '<button class="link-button" id="gp-weights-reset">reset to class default</button>' : '');

  for (const input of el.querySelectorAll('input[data-weight]')) {
    input.onchange = () => {
      if (!gpCustomWeights) gpCustomWeights = { ...gpWeights };
      gpCustomWeights[input.dataset.weight] = Number(input.value) || 0;
      refreshGearPlanner();
    };
  }
  const resetBtn = document.getElementById('gp-weights-reset');
  if (resetBtn) {
    resetBtn.onclick = () => {
      gpCustomWeights = null;
      refreshGearPlanner();
    };
  }
}

// Tag -> pill color. MAGIC and LORE are the ones worth a strong color --
// they change how you play the item (castable vs. not, losable on death
// vs. not) and players scan for them at a glance. NO_DROP/NO_TRADE are
// just "can't move it", not a decision either way, so they stay neutral.
// NO_RENT is the one that can actually cost you the item (log off with it
// on a rented corpse and it's gone) -- that gets the warning color.
// Everything else (ARTIFACT, ATTUNEABLE, QUEST, EXPENDABLE, TEMPORARY) is
// bookkeeping rather than something to react to, so it falls through to
// the muted default rather than getting its own case.
const GP_TAG_STYLE = {
  MAGIC: 'blue',
  LORE: 'orange',
  LORE_EQUIPPED: 'orange',
  NO_DROP: 'white',
  NO_TRADE: 'white',
  NO_RENT: 'red',
};

function gpTagLabel(tag) {
  return tag
    .split('_')
    .map((w) => w[0] + w.slice(1).toLowerCase())
    .join(' ');
}

function gpTagPill(tag, styleOverride) {
  const style = styleOverride || GP_TAG_STYLE[tag] || 'muted';
  return `<span class="gp-tag gp-tag-${style}">${escapeHtml(gpTagLabel(tag))}</span>`;
}

// The weapon skill ("2H Blunt", "SHIELD", ...) gets the same pill
// treatment but is already display-cased by the scrape -- gpTagLabel's
// snake_case-to-title-case pass would mangle "2H" into "2h", so this
// prints it verbatim instead of routing it through gpTagPill.
function gpSkillPill(skill) {
  return `<span class="gp-tag gp-tag-muted">${escapeHtml(skill)}</span>`;
}

// source_label's other shapes ("quest: X", "vendor: X", "crafted") aren't
// zones at all -- only a real "Zone — Mob" drop source gets the in-app
// link; those three stay plain text.
function gpSourceHtml(source) {
  if (!source) return 'no known source recorded';
  if (source.startsWith('quest:') || source.startsWith('vendor:') || source === 'crafted') {
    return escapeHtml(source);
  }
  const [zone, mob] = source.split(' — ');
  // gdZoneOrMobLink, not a plain zone link -- `zone` here is drops[0]'s own
  // zone string, which can itself be a mob name if that drop entry hit the
  // scrape's zone/mob parsing ambiguity (see that function's doc).
  const zoneHtml = gdZoneOrMobLink(zone);
  return mob ? `${zoneHtml} — ${gdLinkOrText('npc', mob)}` : zoneHtml;
}

// Full detail for whichever item is currently selected (the slot's top
// pick by default, or whichever alt row was last clicked) -- lives under
// "Suggested zones" rather than replacing it, so both stay visible at
// once instead of one view hiding the other.
function renderGpDetail() {
  const el = document.getElementById('gp-detail');
  const item = gpDetailItem;
  if (!item) {
    el.classList.add('hidden');
    el.innerHTML = '';
    return;
  }
  el.classList.remove('hidden');
  const statRows = Object.entries(item.stats || {})
    .sort((a, b) => b[1] - a[1])
    .map(([k, v]) => `<span>${escapeHtml(k)} ${v >= 0 ? '+' : ''}${v}</span>`)
    .join('');
  const weapon = item.dmg != null && item.delay != null ? `<div class="gp-detail-row">${item.dmg} / ${item.delay} (${(item.dmg / item.delay).toFixed(2)} ratio)</div>` : '';
  const wikiUrl = item.url || `https://eqlwiki.com/${encodeURIComponent(item.name.replace(/ /g, '_'))}`;
  el.innerHTML = `
    <div class="gp-detail-head">
      ${item.icon ? `<img class="gp-detail-icon" src="planner/icons/${encodeURIComponent(item.icon)}" alt="">` : ''}
      <div>
        <div class="gp-detail-name">${escapeHtml(item.name)}<a href="${escapeHtml(wikiUrl)}" target="_blank" rel="noopener">eqlwiki ↗</a></div>
        <div class="gp-detail-row">${escapeHtml(item.classes.join(' / ') || 'any class')} -- ${escapeHtml(item.slots.join(', '))}${item.era ? ` -- ${escapeHtml(item.era)}` : ''}${item.wt != null ? ` -- WT ${item.wt}` : ''}${item.size ? ` -- ${escapeHtml(item.size)}` : ''}</div>
      </div>
    </div>
    ${statRows ? `<div class="gp-detail-stats">${statRows}</div>` : ''}
    ${weapon}
    ${item.tags.length || item.skill ? `<div class="gp-tags">${item.tags.map((t) => gpTagPill(t)).join('')}${item.skill ? gpSkillPill(item.skill) : ''}</div>` : ''}
    <div class="gp-detail-row">${gpSourceHtml(item.source)}</div>
  `;
}

// ---------------------------------------------------------------- fight timeline

const SERIES_COLORS = ['#5fb3ff', '#5fd18a', '#e0b34d', '#e0616f', '#b892ff', '#4dd0e1', '#ffb74d', '#81c995'];
const SVG_NS = 'http://www.w3.org/2000/svg';

// Fixed coordinate space for the chart; CSS stretches the <svg> to fill its
// container (preserveAspectRatio="none"), so these are just a convenient
// unit to compute point positions in, not pixels.
const CHART_VIEW_W = 1000;
const CHART_VIEW_H = 200;
// Headroom above the tallest point so a peak doesn't touch the top edge.
const CHART_PAD_TOP = 10;

let currentTimelineEncounterId = null;
let currentTimelineStartMs = null;
let currentTimelineDto = null;
let highlightedEntity = null;
let selectedBucketMs = null;

async function loadTimeline(encounterId) {
  currentTimelineEncounterId = encounterId;
  const dto = await invoke('get_fight_timeline', { encounterId });
  if (!dto || dto.series.length === 0) {
    el('timeline-pane').classList.add('hidden');
    currentTimelineDto = null;
    return;
  }
  el('timeline-pane').classList.remove('hidden');
  currentTimelineStartMs = dto.start_ms;
  currentTimelineDto = dto;
  renderTimelineChart(dto);
}

// One shared chart, all entities as overlapping lines, rather than a row of
// bars per entity -- easier to compare shapes over time across several
// people at once. Line/swatch colors stay distinct per person; ally/enemy
// is conveyed separately as a tint on the name text so it doesn't cost that
// per-person distinction. Selecting a legend chip makes that person's line
// solid and full-opacity, fading the rest -- see .series-line.highlighted
// / .dimmed.
function renderTimelineChart(dto) {
  const legend = el('timeline-legend');
  const chart = el('timeline-chart');
  legend.innerHTML = '';

  const globalMax = Math.max(1, ...dto.series.flatMap((s) => s.values));
  const bucketCount = dto.buckets.length;
  const xStep = bucketCount > 1 ? CHART_VIEW_W / (bucketCount - 1) : 0;
  const xFor = (i) => (bucketCount > 1 ? i * xStep : CHART_VIEW_W / 2);
  const yFor = (v) => CHART_PAD_TOP + (CHART_VIEW_H - CHART_PAD_TOP) * (1 - v / globalMax);

  const svg = document.createElementNS(SVG_NS, 'svg');
  svg.setAttribute('viewBox', `0 0 ${CHART_VIEW_W} ${CHART_VIEW_H}`);
  svg.setAttribute('preserveAspectRatio', 'none');

  dto.series.forEach((s, i) => {
    const color = SERIES_COLORS[i % SERIES_COLORS.length];
    const sideClass = s.is_player || s.is_pet ? 'entity-ally' : s.is_enemy ? 'entity-enemy' : '';

    const chip = document.createElement('button');
    chip.type = 'button';
    chip.className = 'legend-chip';
    chip.dataset.entity = s.name;
    chip.style.setProperty('--series-color', color);
    chip.innerHTML = `<span class="swatch"></span><span class="${sideClass}">${escapeHtml(s.name)}</span> (${s.total.toLocaleString()})`;
    chip.addEventListener('click', () => {
      highlightedEntity = highlightedEntity === s.name ? null : s.name;
      applyHighlight();
    });
    legend.appendChild(chip);

    const points = s.values.map((v, bi) => `${xFor(bi).toFixed(1)},${yFor(v).toFixed(1)}`).join(' ');
    const line = document.createElementNS(SVG_NS, 'polyline');
    line.setAttribute('class', 'series-line');
    line.setAttribute('points', points);
    line.style.setProperty('--series-color', color);
    // Property assignment, not string interpolation into markup -- safe
    // regardless of what characters a log-derived name contains.
    line.dataset.entity = s.name;
    svg.appendChild(line);
  });

  const scrubIdx = selectedBucketMs === null ? -1 : dto.buckets.indexOf(selectedBucketMs);
  if (scrubIdx >= 0) {
    const x = xFor(scrubIdx);
    const scrub = document.createElementNS(SVG_NS, 'line');
    scrub.setAttribute('class', 'timeline-scrub');
    scrub.setAttribute('x1', x);
    scrub.setAttribute('y1', 0);
    scrub.setAttribute('x2', x);
    scrub.setAttribute('y2', CHART_VIEW_H);
    svg.appendChild(scrub);
  }

  svg.addEventListener('click', (event) => {
    const rect = svg.getBoundingClientRect();
    const relX = ((event.clientX - rect.left) / rect.width) * CHART_VIEW_W;
    const idx = bucketCount > 1 ? Math.round(relX / xStep) : 0;
    const clamped = Math.max(0, Math.min(bucketCount - 1, idx));
    showStateAt(dto.buckets[clamped] ?? dto.start_ms);
  });

  chart.innerHTML = '';
  chart.appendChild(svg);

  applyHighlight();
}

function applyHighlight() {
  for (const line of document.querySelectorAll('.series-line')) {
    const isHighlighted = highlightedEntity !== null && line.dataset.entity === highlightedEntity;
    line.classList.toggle('highlighted', isHighlighted);
    line.classList.toggle('dimmed', highlightedEntity !== null && !isHighlighted);
  }
  for (const chip of document.querySelectorAll('.legend-chip')) {
    chip.classList.toggle('dimmed', highlightedEntity !== null && chip.dataset.entity !== highlightedEntity);
  }
}

async function showStateAt(tsMs) {
  selectedBucketMs = tsMs; // bucket *start* -- matches dto.buckets, for the scrub line
  if (currentTimelineDto) renderTimelineChart(currentTimelineDto); // redraws the scrub line

  // get_fight_state_at's dps is a trailing window ending *at* the instant
  // it's given -- querying at the clicked bucket's own start would look
  // backward into the *previous* bucket almost entirely, showing that
  // bucket's dps instead of the one you clicked. Query at this bucket's
  // end instead, so the window actually overlaps the bucket you clicked
  // (fully, whenever a bucket covers more than the inspect window).
  const bucketMs = currentTimelineDto ? currentTimelineDto.bucket_ms : 0;
  const fightEnd = currentTimelineDto ? currentTimelineDto.start_ms + currentTimelineDto.duration_ms : tsMs;
  const queryMs = Math.min(tsMs + bucketMs, fightEnd);

  const states = await invoke('get_fight_state_at', { encounterId: currentTimelineEncounterId, tsMs: queryMs });

  const panel = el('timeline-state');
  panel.classList.remove('hidden');
  const into = currentTimelineStartMs !== null ? tsMs - currentTimelineStartMs : 0;
  el('timeline-state-time').textContent = `${fmtDuration(into)} into the fight`;

  const tbody = document.querySelector('#timeline-state-table tbody');
  tbody.innerHTML = '';
  for (const s of states) {
    const tr = document.createElement('tr');
    const badgeClass = `state-badge state-${s.state}${s.observed ? '' : ' inferred'}`;
    const sideClass = s.is_player || s.is_pet ? 'entity-ally' : s.is_enemy ? 'entity-enemy' : '';
    tr.innerHTML = `
      <td><span class="${sideClass}">${escapeHtml(s.name)}</span>${s.is_player ? ' <span class="muted">(you)</span>' : ''}</td>
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
el('history-confirmed-only').addEventListener('change', refreshMobHistory);

setInterval(refreshCombat, COMBAT_REFRESH_MS);
setInterval(refreshMonsters, COMBAT_REFRESH_MS);
setInterval(refreshOverviewSession, COMBAT_REFRESH_MS);

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
  refreshOverviewSession();
});

listen('parse-error', (event) => {
  showError(String(event.payload));
});

// ---------------------------------------------------------------- inventory dump toast

// The file named by the "Outputfile Complete" line currently on offer in
// the toast, or null between dumps / once it's been acted on. Backend-side
// detection lives in tail_worker.rs's emit_tick -- this is purely "what do
// we do once told about it".
let invPendingFile = null;
let invToastTimer = null;

function hideInvToast() {
  el('inv-toast').classList.remove('visible');
  clearTimeout(invToastTimer);
  invToastTimer = null;
}

// The initial prompt -- offers a Load button, stays up until dismissed or
// acted on (no auto-hide: missing a fresh dump because it slid away on its
// own would be worse than it sitting there a while).
function showInvPrompt(file, character) {
  invPendingFile = file;
  el('inv-toast-text').textContent = `${character || 'A character'}'s inventory dump just finished writing (${file}).`;
  el('inv-toast-load').classList.remove('hidden');
  el('inv-toast').classList.add('visible');
  clearTimeout(invToastTimer);
  invToastTimer = null;
}

// The post-load confirmation -- same banner, no Load button (nothing left
// to do), auto-hides since it's just an acknowledgment, not a decision.
function showInvNote(text) {
  invPendingFile = null;
  el('inv-toast-text').textContent = text;
  el('inv-toast-load').classList.add('hidden');
  el('inv-toast').classList.add('visible');
  clearTimeout(invToastTimer);
  invToastTimer = setTimeout(hideInvToast, 5000);
}

el('inv-toast-dismiss').addEventListener('click', hideInvToast);

el('inv-toast-load').addEventListener('click', async () => {
  const file = invPendingFile;
  hideInvToast();
  if (!file) return;
  try {
    const dump = await invoke('get_inventory_dump', { file });
    // Wholesale replace, not merge -- "load my current inventory" means
    // the doll should show what you actually have on, not a mix of that
    // and whatever you'd been browsing before. gpChosen (manual alt picks)
    // is cleared for the same reason; gpEquipped is what now drives the
    // doll (see gpChosenItem).
    gpEquipped = dump.resolved;
    gpChosen = {};
    gpDetailItem = null;
    gpExpandedSlot = null;
    csSub = 'gear';
    showModule('character');
    const resolvedCount = Object.keys(dump.resolved).length;
    const unresolvedCount = Object.keys(dump.unresolved).length;
    showInvNote(
      unresolvedCount
        ? `Loaded ${resolvedCount} equipped item${resolvedCount === 1 ? '' : 's'} -- ${unresolvedCount} not matched to a known item.`
        : `Loaded ${resolvedCount} equipped item${resolvedCount === 1 ? '' : 's'} into the Character tab's Gear page.`,
    );
  } catch (e) {
    showError(`Couldn't load inventory dump: ${e}`);
  }
});

listen('inventory-dump', (event) => {
  showInvPrompt(event.payload.file, event.payload.character);
});

// ---------------------------------------------------------------- notification sounds

// The backend already filters out disabled kinds before ever emitting
// (see tail_worker.rs's own doc) -- anything that reaches this listener
// is real and enabled, so there's no enabled-check to repeat here, only
// "which sound" and "show the toast".
listen('notification', (event) => {
  playNotificationSound(event.payload.kind);
  showNotifToast(event.payload.message);
});

let notifToastTimer = null;
function showNotifToast(message) {
  const box = el('notif-toast');
  box.textContent = message;
  box.classList.add('visible');
  clearTimeout(notifToastTimer);
  notifToastTimer = setTimeout(() => box.classList.remove('visible'), 4000);
}

// kind -> data: URL, only populated for kinds that actually have a custom
// sound uploaded (see refreshSettings) -- a kind with no entry here plays
// its synthesized default instead. Refreshed whenever the Settings module
// loads or a sound is changed, not fetched per-event: a notification can
// fire from any tab, any time, and shouldn't need an extra IPC round
// trip in the moment to find out what to play.
let notifSoundCache = {};

function playNotificationSound(kind) {
  const cached = notifSoundCache[kind];
  if (cached) {
    new Audio(cached).play().catch(() => {});
  } else {
    playDefaultTone(kind);
  }
}

// One shared AudioContext, created lazily on first real use -- browsers
// (and Tauri's webview is one) refuse to start an AudioContext before any
// user gesture has happened on the page, so building this at script-load
// time would leave it permanently suspended for a session where the user
// never clicked anything before the first real notification fired. Every
// call site (playDefaultTone, the Test button) goes through this getter
// instead of touching a module-level instance directly.
let notifAudioCtx = null;
function audioCtx() {
  if (!notifAudioCtx) notifAudioCtx = new (window.AudioContext || window.webkitAudioContext)();
  return notifAudioCtx;
}

// One short envelope-shaped tone (attack, hold, release -- avoids the
// audible click a hard on/off gain edge would make). `freq` in Hz,
// `startAt`/`dur` in seconds relative to now.
function playTone(freq, startAt, dur, type = 'sine', peakGain = 0.2) {
  const ctx = audioCtx();
  const osc = ctx.createOscillator();
  const gain = ctx.createGain();
  osc.type = type;
  osc.frequency.value = freq;
  osc.connect(gain);
  gain.connect(ctx.destination);
  const t0 = ctx.currentTime + startAt;
  gain.gain.setValueAtTime(0, t0);
  gain.gain.linearRampToValueAtTime(peakGain, t0 + 0.02);
  gain.gain.linearRampToValueAtTime(0, t0 + dur);
  osc.start(t0);
  osc.stop(t0 + dur + 0.02);
}

// Synthesized, not shipped audio files -- no bundle size, no licensing to
// track, and no asset-serving/CSP concerns for what's ultimately just a
// handful of short tones. Each kind gets a distinct, purpose-fitting
// shape: a two-note rising chime for a genuine reward (level up, AA), a
// single warm tone for a soft heads-up (invis fading), and a sharper
// two-beat buzz for something that needs attention now (charm breaking).
function playDefaultTone(kind) {
  if (kind === 'level_up') {
    playTone(523.25, 0, 0.16, 'triangle'); // C5
    playTone(783.99, 0.14, 0.22, 'triangle'); // G5
  } else if (kind === 'aa_gained') {
    playTone(659.25, 0, 0.14, 'triangle'); // E5
    playTone(880, 0.12, 0.2, 'triangle'); // A5
  } else if (kind === 'invis_fading') {
    playTone(392, 0, 0.35, 'sine', 0.15); // G4, soft and held
  } else if (kind === 'charm_broken') {
    playTone(311.13, 0, 0.12, 'square', 0.12); // Eb4
    playTone(311.13, 0.16, 0.12, 'square', 0.12);
  } else {
    playTone(440, 0, 0.15, 'sine');
  }
}

let notifKinds = []; // [{kind, label}]
let notifSettings = null; // raw NotificationSettings from the backend

async function refreshSettings() {
  if (activeModule !== 'settings') return;
  const box = el('notif-kinds');
  try {
    [notifKinds, notifSettings] = await Promise.all([invoke('list_notification_kinds'), invoke('get_notification_settings')]);
  } catch (e) {
    box.innerHTML = `<p class="muted">Couldn't load notification settings: ${escapeHtml(String(e))}</p>`;
    return;
  }
  if (activeModule !== 'settings') return;
  await refreshNotifSoundCache();
  if (activeModule !== 'settings') return;
  renderNotifKinds();
}

// Pulls actual sound *data* only for kinds that have a custom upload --
// the settings payload itself only carries a filename, not the audio, so
// this is what turns "has a custom sound" into "here's a URL Audio() can
// play". Called on every settings load (not just once) since a kind's
// custom sound can change while this tab is open.
async function refreshNotifSoundCache() {
  const withCustom = notifKinds.map((k) => k.kind).filter((kind) => notifSettings.custom_sound && notifSettings.custom_sound[kind]);
  const urls = await Promise.all(withCustom.map((kind) => invoke('get_notification_sound_data', { kind }).catch(() => null)));
  notifSoundCache = {};
  withCustom.forEach((kind, i) => {
    if (urls[i]) notifSoundCache[kind] = urls[i];
  });
}

function renderNotifKinds() {
  const box = el('notif-kinds');
  box.innerHTML = notifKinds
    .map(({ kind, label }) => {
      const enabled = notifSettings.enabled?.[kind] ?? true;
      const hasCustom = !!(notifSettings.custom_sound && notifSettings.custom_sound[kind]);
      return `<div class="notif-kind-card">
        <label class="notif-toggle">
          <input type="checkbox" data-kind="${escapeHtml(kind)}" ${enabled ? 'checked' : ''}>
          <span>${escapeHtml(label)}</span>
        </label>
        <div class="notif-kind-body">
          <span class="notif-sound-source ${hasCustom ? '' : 'muted'}">${hasCustom ? 'Custom sound uploaded' : 'Default sound'}</span>
          <button class="notif-test" data-kind="${escapeHtml(kind)}">Test</button>
          <button class="notif-upload" data-kind="${escapeHtml(kind)}">Upload sound&hellip;</button>
          ${hasCustom ? `<button class="notif-reset link-button" data-kind="${escapeHtml(kind)}">Reset to default</button>` : ''}
        </div>
      </div>`;
    })
    .join('');

  box.querySelectorAll('.notif-toggle input').forEach((input) => {
    input.addEventListener('change', async () => {
      const kind = input.dataset.kind;
      try {
        notifSettings = await invoke('set_notification_enabled', { kind, on: input.checked });
      } catch (e) {
        showError(`Couldn't save notification setting: ${e}`);
        input.checked = !input.checked; // revert on failure
      }
    });
  });
  box.querySelectorAll('.notif-test').forEach((btn) => {
    btn.addEventListener('click', () => playNotificationSound(btn.dataset.kind));
  });
  box.querySelectorAll('.notif-upload').forEach((btn) => {
    btn.addEventListener('click', async () => {
      const kind = btn.dataset.kind;
      try {
        const updated = await invoke('pick_notification_sound', { kind });
        if (updated) {
          notifSettings = updated;
          await refreshNotifSoundCache();
          renderNotifKinds();
        }
      } catch (e) {
        showError(`Couldn't set notification sound: ${e}`);
      }
    });
  });
  box.querySelectorAll('.notif-reset').forEach((btn) => {
    btn.addEventListener('click', async () => {
      const kind = btn.dataset.kind;
      try {
        notifSettings = await invoke('clear_notification_sound', { kind });
        delete notifSoundCache[kind];
        renderNotifKinds();
      } catch (e) {
        showError(`Couldn't reset notification sound: ${e}`);
      }
    });
  });
}

// ---------------------------------------------------------------- debug module

// Two subpages, same "you left it where you left it" tab idiom the
// Character/Game Data modules already use: Encounters (the original
// table) and Unparsed (new -- see refreshUnparsed's own doc).
let debugSub = 'encounters';

function debugShowing(sub) {
  return activeModule === 'debug' && debugSub === sub;
}

function showDebugSub(name) {
  debugSub = name;
  for (const btn of document.querySelectorAll('#debug-tabs .gd-tab')) {
    btn.classList.toggle('active', btn.dataset.sub === name);
  }
  for (const section of document.querySelectorAll('.debug-subpage')) {
    section.classList.toggle('hidden', section.id !== `debug-sub-${name}`);
  }
  if (name === 'encounters') refreshDebug();
  if (name === 'unparsed') refreshUnparsed();
}

for (const btn of document.querySelectorAll('#debug-tabs .gd-tab')) {
  btn.addEventListener('click', () => showDebugSub(btn.dataset.sub));
}

// Not a feature -- a direct look at what Ingest actually tagged each
// encounter with, zone-wise, so "is this working" has a real answer
// instead of trusting the Zones page's own filtering blind. Re-fetched
// every open (no cache): this exists specifically to reflect current
// backend state accurately, not to be fast at the expense of that.
async function refreshDebug() {
  if (!debugShowing('encounters')) return;
  let rows = [];
  try {
    rows = await invoke('list_debug_encounters', { limit: 100 });
  } catch (err) {
    document.querySelector('#debug-table tbody').innerHTML = '';
    el('debug-empty').textContent = `Couldn't load: ${String(err)}`;
    el('debug-empty').classList.remove('hidden');
    return;
  }
  if (!debugShowing('encounters')) return;
  renderDebugTable(rows);
}

function renderDebugTable(rows) {
  el('debug-empty').classList.toggle('hidden', rows.length > 0);
  if (!rows.length) {
    el('debug-empty').textContent = 'No encounters parsed yet.';
  }
  document.querySelector('#debug-table tbody').innerHTML = rows
    .map((r) => {
      // Present but unresolved is the interesting case (a real zone-
      // match miss); absent entirely just means no zone.enter line has
      // been seen yet (the "Unknown" bucket) -- these read differently on
      // purpose, not as the same blank dash.
      const unmatched = r.raw_zone && !r.resolved_zone_id;
      return `<tr class="${unmatched ? 'zone-unmatched' : ''}">
        <td>${r.id}</td>
        <td>${escapeHtml(r.target)}</td>
        <td>${escapeHtml(new Date(r.start_ms).toLocaleString())}</td>
        <td class="num">${fmtTtk(r.duration_ms)}</td>
        <td class="num">${r.tier}</td>
        <td>${r.raw_zone ? escapeHtml(r.raw_zone) : '<span class="debug-unknown">none yet</span>'}</td>
        <td class="debug-resolved">${r.resolved_zone_id ? escapeHtml(r.resolved_zone_id) : r.raw_zone ? 'NO MATCH' : '<span class="debug-unknown">—</span>'}</td>
      </tr>`;
    })
    .join('');
}

// Every line that never matched any rule, clustered into shapes (see
// crates/core/src/shape.rs) exactly the way the offline `eqlp coverage`
// CLI command does -- same algorithm, same ranking, just kept live in the
// running app so working toward more coverage doesn't require pulling
// the log file out and running a separate tool. Re-fetched every open,
// same reasoning refreshDebug's own doc gives.
async function refreshUnparsed() {
  if (!debugShowing('unparsed')) return;
  let cov;
  try {
    cov = await invoke('get_unmatched_coverage', { top: 200 });
  } catch (err) {
    document.querySelector('#unparsed-table tbody').innerHTML = '';
    el('unparsed-empty').textContent = `Couldn't load: ${String(err)}`;
    el('unparsed-empty').classList.remove('hidden');
    return;
  }
  if (!debugShowing('unparsed')) return;
  renderUnparsedTable(cov);
}

function renderUnparsedTable(cov) {
  const pct = cov.total_lines > 0 ? ((cov.unmatched_total / cov.total_lines) * 100).toFixed(1) : '0.0';
  let summary = `<b>${cov.unmatched_total.toLocaleString()}</b> unmatched line${cov.unmatched_total === 1 ? '' : 's'} (${pct}% of ${cov.total_lines.toLocaleString()} total) -- <b>${cov.distinct_shapes.toLocaleString()}</b> distinct shape${cov.distinct_shapes === 1 ? '' : 's'}.`;
  if (cov.shapes_overflow > 0) {
    summary += ` <span class="muted">${cov.shapes_overflow.toLocaleString()} more lines seen past the tracking cap -- their shapes exist but aren't counted.</span>`;
  }
  el('unparsed-summary').innerHTML = summary;

  el('unparsed-empty').classList.toggle('hidden', cov.shapes.length > 0);
  if (!cov.shapes.length) {
    el('unparsed-empty').textContent = 'No unmatched lines yet.';
  }
  document.querySelector('#unparsed-table tbody').innerHTML = cov.shapes
    .map(
      (s) => `<tr>
        <td class="num">${s.count.toLocaleString()}</td>
        <td class="unparsed-shape">${escapeHtml(s.shape)}</td>
        <td class="unparsed-example muted" title="${escapeHtml(s.example)}">${escapeHtml(s.example)}</td>
      </tr>`,
    )
    .join('');
}

(async () => {
  const result = await invoke('get_status');
  renderStatus(result.status, result.counts);
  showScreen(result.configured ? 'main' : 'setup');
  showModule('overview');
})();
