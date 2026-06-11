// TAB 1 — the Command glance board. Paints instantly from cache, then
// refreshes live: weather 30 min · portfolio 60 s · calendar/todos 5 min.
// Everything pauses while hidden (data.js). Cards degrade to a dim state
// with a one-tap path to Settings — never a blank hole.

import { inv, cached, store, poll, agoLabel, fmt, tickNumber, deltaEl, kickAll } from './data.js';

const { listen } = window.__TAURI__.event;

// ---------- greeting + clock ----------

const greetLine = document.getElementById('greet-line');
const greetDate = document.getElementById('greet-date');
const greetClock = document.getElementById('greet-clock');

function tickClock() {
  const now = new Date();
  const h = now.getHours();
  greetLine.textContent =
    h < 5 ? 'Burning the midnight oil, sir.' :
    h < 12 ? 'Good morning, sir.' :
    h < 17 ? 'Good afternoon, sir.' :
    'Good evening, sir.';
  greetDate.textContent = now.toLocaleDateString('en-GB', { weekday: 'long', day: 'numeric', month: 'long' });
  greetClock.textContent = now.toLocaleTimeString('en-GB', { hour: '2-digit', minute: '2-digit' });
}
tickClock();
setInterval(() => { if (!document.hidden) tickClock(); }, 5000);

// ---------- weather (30 min) ----------

const WX_ICON = [
  [/thunder/, '⛈'], [/snow/, '🌨'], [/heavy rain|violent/, '🌧'], [/rain|drizzle|shower/, '🌦'],
  [/fog/, '🌫'], [/overcast/, '☁'], [/partly|mostly/, '🌤'], [/clear/, '☀'],
];

function paintWeather(w, ts) {
  if (!w?.now) return;
  document.getElementById('wx-temp').textContent = `${Math.round(w.now.temp_c)}°`;
  const cond = w.now.conditions || '';
  document.getElementById('wx-icon').textContent = (WX_ICON.find(([re]) => re.test(cond)) || [, '☀'])[1];
  const today = w.today || {};
  document.getElementById('wx-line').textContent =
    `${cond} · ${Math.round(today.high_c ?? w.now.temp_c)}° high · ${today.rain_chance_pct ?? 0}% rain`;
  document.getElementById('greet-weather').title = `${w.city} · updated ${agoLabel(ts)}`;
}

{
  const c = cached('weather');
  if (c) paintWeather(c.data, c.ts);
}
poll('weather', 30 * 60 * 1000, async () => {
  const w = await inv('weather_now');
  store('weather', w);
  paintWeather(w, Date.now());
});

// ---------- portfolio (60 s) ----------

const SITE_ORDER = ['pdfedit', 'imagetool', 'audiotool', 'videotool', 'invoicetool'];
const miniReactor = document.getElementById('mini-reactor');

function buildMiniReactor() {
  const NS = 'http://www.w3.org/2000/svg';
  const mk = (tag, attrs) => {
    const e = document.createElementNS(NS, tag);
    for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, v);
    return e;
  };
  miniReactor.innerHTML = '';
  // Static structure rings.
  miniReactor.append(
    mk('circle', { cx: 60, cy: 60, r: 56, fill: 'none', stroke: 'rgba(78,216,255,0.14)', 'stroke-width': 1 }),
    mk('circle', { cx: 60, cy: 60, r: 44, fill: 'none', stroke: 'rgba(78,216,255,0.10)', 'stroke-width': 6, 'stroke-dasharray': '2 6' }),
  );
  // Progress arc: today vs 7-day average (live element — glow allowed).
  const arc = mk('circle', {
    cx: 60, cy: 60, r: 52, fill: 'none',
    stroke: 'url(#mr-grad)', 'stroke-width': 3, 'stroke-linecap': 'round',
    'stroke-dasharray': `${2 * Math.PI * 52}`, 'stroke-dashoffset': `${2 * Math.PI * 52}`,
    transform: 'rotate(-90 60 60)', id: 'mr-arc',
    style: 'transition: stroke-dashoffset 600ms cubic-bezier(.25,.8,.4,1); filter: drop-shadow(0 0 4px rgba(79,216,255,.6))',
  });
  const defs = mk('defs', {});
  defs.innerHTML = `<linearGradient id="mr-grad" x1="0" y1="0" x2="1" y2="1">
    <stop offset="0%" stop-color="#7fe4ff"/><stop offset="100%" stop-color="#1ec8ff"/></linearGradient>`;
  miniReactor.append(defs, arc);
  return arc;
}
const mrArc = buildMiniReactor();

function paintPortfolio(o, summary, ts) {
  const card = document.getElementById('card-portfolio');
  card.classList.remove('degraded');
  document.getElementById('pf-degraded').hidden = true;
  card.title = `updated ${agoLabel(ts)}`;

  const pv = o.today?.pageviews ?? 0;
  const uv = o.today?.uniques ?? 0;
  tickNumber(document.getElementById('pf-pv'), pv);
  tickNumber(document.getElementById('pf-pv2'), pv);
  tickNumber(document.getElementById('pf-uv'), uv);

  const avg = Math.round(o.weekAvgDaily ?? 0);
  tickNumber(document.getElementById('pf-avg'), avg);

  // Arc: today's pv as a share of the 7-day daily average.
  const ratio = avg > 0 ? Math.min(pv / avg, 1) : 0;
  const C = 2 * Math.PI * 52;
  mrArc.setAttribute('stroke-dashoffset', String(C * (1 - ratio)));
  const vsAvg = avg > 0 ? Math.round((pv / avg) * 100) : null;
  document.getElementById('pf-vs-avg').textContent = vsAvg != null ? `${vsAvg}% of avg` : '';

  if (summary?.delta) {
    deltaEl(document.getElementById('pf-pv-delta'), summary.delta.pageviews);
    deltaEl(document.getElementById('pf-uv-delta'), summary.delta.uniques);
  }

  // Live-now counter.
  const liveN = o.last30minEvents ?? 0;
  const liveEl = document.getElementById('live-now');
  liveEl.hidden = false;
  tickNumber(document.getElementById('live-now-n'), liveN);

  // 5-dot site strip — glow = traffic share.
  const strip = document.getElementById('site-strip');
  strip.innerHTML = '';
  const sites = o.today?.sites ?? [];
  for (const id of SITE_ORDER) {
    const s = sites.find((x) => x.id === id) || { id, label: id, pv: 0, share: 0 };
    const dot = document.createElement('span');
    dot.className = 'site-dot';
    dot.dataset.tip = `${s.label || s.id} — ${fmt(s.pv)} views`;
    const a = 0.15 + s.share * 0.85;
    dot.style.background = `rgba(79,216,255,${a.toFixed(2)})`;
    if (s.share > 0.05) dot.style.boxShadow = `0 0 ${Math.round(4 + s.share * 14)}px rgba(79,216,255,${(s.share * 0.9).toFixed(2)})`;
    strip.appendChild(dot);
  }

  // Anomaly chips (amber is reserved for exactly this).
  const chips = document.getElementById('pf-anomalies');
  chips.innerHTML = '';
  for (const a of o.anomalies ?? []) {
    const chip = document.createElement('span');
    chip.className = 'chip';
    chip.textContent = `${a.site} ${a.pct > 0 ? '+' : ''}${a.pct}%`;
    chips.appendChild(chip);
  }
}

function degradePortfolio(err) {
  const card = document.getElementById('card-portfolio');
  if (!cached('portfolio')) card.classList.add('degraded');
  const deg = document.getElementById('pf-degraded');
  deg.hidden = false;
  deg.firstElementChild.textContent = String(err).includes('not configured')
    ? 'service token required' : 'analytics offline — cached view';
}

{
  const c = cached('portfolio');
  if (c) paintPortfolio(c.data.overview, c.data.summary, c.ts);
}
poll('portfolio', 60 * 1000, async () => {
  try {
    const today = new Date(Date.now() + 5.5 * 3600_000).toISOString().slice(0, 10); // IST date
    const [overview, summary] = await Promise.all([
      inv('worker_api', { path: '/api/overview' }),
      inv('worker_api', { path: `/api/summary?from=${today}&to=${today}` }),
    ]);
    store('portfolio', { overview, summary });
    paintPortfolio(overview, summary, Date.now());
  } catch (err) {
    degradePortfolio(err);
    throw err;
  }
});

// ---------- today / calendar (5 min) ----------

function fmtTime(iso) {
  if (!iso) return '';
  if (!iso.includes('T')) return 'all day';
  return new Date(iso).toLocaleTimeString('en-GB', { hour: '2-digit', minute: '2-digit' });
}

function paintToday(data, ts) {
  const card = document.getElementById('card-today');
  card.classList.remove('degraded');
  document.getElementById('today-degraded').hidden = true;
  card.title = `updated ${agoLabel(ts)}`;

  const events = (data.events || []).filter((e) => e.start);
  const now = Date.now();
  const upcoming = events.filter((e) => e.start.includes('T') && Date.parse(e.start) > now);
  const next = upcoming[0];

  const nm = document.getElementById('next-meeting');
  if (next) {
    nm.hidden = false;
    document.getElementById('nm-title').textContent = next.title || '(untitled)';
    document.getElementById('nm-time').textContent = fmtTime(next.start);
    const mins = Math.round((Date.parse(next.start) - now) / 60000);
    document.getElementById('nm-in').textContent =
      mins < 60 ? `in ${mins} min` : `in ${Math.floor(mins / 60)} h ${mins % 60} min`;
  } else {
    nm.hidden = true;
  }

  const list = document.getElementById('event-list');
  list.innerHTML = '';
  const rest = events.filter((e) => e !== next);
  for (const e of rest) {
    const li = document.createElement('li');
    const past = e.end && Date.parse(e.end) < now;
    if (past) li.className = 'past';
    li.innerHTML = `<span class="ev-time num"></span><span class="ev-title"></span><span class="ev-badge"></span>`;
    li.querySelector('.ev-time').textContent = fmtTime(e.start);
    li.querySelector('.ev-title').textContent = e.title || '(untitled)';
    if (e.work) li.querySelector('.ev-badge').innerHTML = '<span class="badge-w">W</span>';
    list.appendChild(li);
  }

  document.getElementById('today-count').textContent = events.length ? `${events.length} events` : '';
  document.getElementById('today-empty').hidden = events.length > 0;

  // Free-gap hint: largest clear stretch between now and 21:00.
  const hint = document.getElementById('gap-hint');
  hint.hidden = true;
  const dayEnd = new Date(); dayEnd.setHours(21, 0, 0, 0);
  const busy = events
    .filter((e) => e.start.includes('T') && Date.parse(e.end || e.start) > now)
    .map((e) => [Date.parse(e.start), Date.parse(e.end || e.start)])
    .sort((a, b) => a[0] - b[0]);
  let cursor = now;
  let best = null;
  for (const [s, e] of busy) {
    if (s - cursor >= 2 * 3600_000) best = [cursor, s];
    cursor = Math.max(cursor, e);
  }
  if (dayEnd.getTime() - cursor >= 2 * 3600_000) best = [cursor, dayEnd.getTime()];
  if (best) {
    const hrs = Math.floor((best[1] - best[0]) / 3600_000);
    const at = new Date(best[0]).toLocaleTimeString('en-GB', { hour: '2-digit', minute: '2-digit' });
    hint.hidden = false;
    hint.textContent = `◷ ${hrs} h clear after ${at}`;
  }
}

function degradeToday(err) {
  const card = document.getElementById('card-today');
  const notConnected = String(err).includes('not connected') || String(err).includes('Settings');
  if (!cached('calendar') || notConnected) {
    card.classList.add('degraded');
    const deg = document.getElementById('today-degraded');
    deg.hidden = false;
    deg.firstElementChild.textContent = notConnected ? 'calendar not connected' : 'calendar unreachable';
  }
}

{
  const c = cached('calendar');
  if (c) paintToday(c.data, c.ts);
}
poll('calendar', 5 * 60 * 1000, async () => {
  try {
    const data = await inv('calendar_today');
    store('calendar', data);
    paintToday(data, Date.now());
  } catch (err) {
    degradeToday(err);
    throw err;
  }
});

// ---------- to-do (5 min + change events) ----------

async function refreshTodos() {
  const data = await inv('todo_list');
  store('todos', data);
  paintTodos(data);
}

function paintTodos(data) {
  const items = (data.items || []).slice(0, 5);
  const list = document.getElementById('todo-list');
  list.innerHTML = '';
  for (const t of items) {
    const li = document.createElement('li');
    const check = document.createElement('input');
    check.type = 'checkbox';
    check.className = 'todo-check';
    check.addEventListener('change', async () => {
      li.classList.add('done');
      await inv('todo_complete', { id: t.id });
      setTimeout(refreshTodos, 400);
    });
    const text = document.createElement('span');
    text.className = 'todo-text';
    text.textContent = t.text;
    text.title = t.text;
    li.append(check, text);
    if (t.status === 'suggested') {
      const tag = document.createElement('span');
      tag.className = 'tag-suggested';
      tag.textContent = 'suggested';
      const ok = document.createElement('button');
      ok.className = 'todo-mini'; ok.textContent = '✓'; ok.title = 'Confirm';
      ok.addEventListener('click', async (e) => { e.stopPropagation(); await inv('todo_confirm', { id: t.id }); refreshTodos(); });
      const no = document.createElement('button');
      no.className = 'todo-mini'; no.textContent = '✕'; no.title = 'Dismiss';
      no.addEventListener('click', async (e) => { e.stopPropagation(); await inv('todo_dismiss', { id: t.id }); refreshTodos(); });
      li.append(tag, ok, no);
    }
    list.appendChild(li);
  }
  const badge = document.getElementById('todo-count');
  badge.hidden = !data.open_count;
  badge.textContent = data.open_count;
  document.getElementById('todo-empty').hidden = (data.items || []).length > 0;
}

{
  const c = cached('todos');
  if (c) paintTodos(c.data);
}
poll('todos', 5 * 60 * 1000, refreshTodos);
document.addEventListener('board-refresh', refreshTodos);

document.getElementById('todo-add-form').addEventListener('submit', async (e) => {
  e.preventDefault();
  const inp = document.getElementById('todo-add-input');
  const text = inp.value.trim();
  if (!text) return;
  inp.value = '';
  await inv('todo_add', { text });
  refreshTodos();
});

// ---------- inbox/slack strip (work stage lights this up) ----------

async function refreshInbox() {
  // Until work credentials exist the strip stays a quiet nudge.
  const [hasSlack, hasWorkGoogle] = await Promise.all([
    inv('secret_exists', { key: 'slack_token' }).catch(() => false),
    inv('secret_exists', { key: 'work_google_oauth_token' }).catch(() => false),
  ]);
  const connected = hasSlack || hasWorkGoogle;
  document.getElementById('inbox-nudge').hidden = connected;
  document.getElementById('inbox-rows').hidden = !connected;
  if (connected) document.dispatchEvent(new CustomEvent('work-strip-refresh'));
}
poll('inbox', 5 * 60 * 1000, refreshInbox);

// ---------- stagger paint on load + in sync with the spoken briefing ----------

function staggerPaint() {
  const board = document.querySelector('.board');
  board.classList.remove('stagger');
  void board.offsetWidth; // restart the animation
  board.classList.add('stagger');
  setTimeout(() => board.classList.remove('stagger'), 900);
}

staggerPaint();

// Morning briefing: refresh every card and stagger-paint (~600ms) so what
// JARVIS says matches what appears.
listen('briefing-start', () => {
  kickAll();
  staggerPaint();
});
