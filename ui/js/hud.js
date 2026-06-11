// TAB 2 — HUD: the analytics dashboard rebuilt natively against the same
// /api/* endpoints, authenticated through the Rust worker proxy (service
// token; no webview embedding, no Access login). Views: Overview / Sites /
// Live / Revenue. Polling runs only while the HUD tab is visible; Live polls
// at 10s, everything else at 60s. Offline grace: every payload is cached and
// repainted with a staleness stamp when the Worker is unreachable.

import { inv, cached, store, agoLabel, fmt } from './data.js';
import { navigate } from './app.js';

const SITES = ['pdfedit', 'imagetool', 'audiotool', 'videotool', 'invoicetool'];
const SITE_LABEL = {
  pdfedit: 'PDF Edit', imagetool: 'Image Tool', audiotool: 'Audio Tool',
  videotool: 'Video Tool', invoicetool: 'Invoice Tool',
};
const body = document.getElementById('hud-body');
const staleEl = document.getElementById('hud-stale');

let view = 'overview';
let site = SITES[0];
let range = '7d';
let hudVisible = false;
let timer = null;

const istToday = () => new Date(Date.now() + 5.5 * 3600_000).toISOString().slice(0, 10);
const shiftDate = (d, n) => new Date(Date.parse(d) + n * 86400_000).toISOString().slice(0, 10);
const istMonth = () => istToday().slice(0, 7);

// ---------- cached fetch with staleness ----------

async function hudGet(path) {
  try {
    const data = await inv('worker_api', { path });
    store(`hud:${path}`, data);
    setStale(null);
    return { data, ts: Date.now(), stale: false };
  } catch (err) {
    const c = cached(`hud:${path}`);
    if (c) { setStale(c.ts); return { data: c.data, ts: c.ts, stale: true }; }
    throw err;
  }
}

function setStale(ts) {
  staleEl.textContent = ts ? `offline — cached ${agoLabel(ts)}` : '';
}

// ---------- view switching + polling ----------

function setView(v) {
  view = v;
  document.querySelectorAll('.hud-tab').forEach((b) => b.classList.toggle('active', b.dataset.view === v));
  render();
  schedule();
}

document.querySelectorAll('.hud-tab').forEach((b) =>
  b.addEventListener('click', () => setView(b.dataset.view)));

document.addEventListener('tab-shown', ({ detail }) => {
  hudVisible = detail.tab === 'hud';
  if (hudVisible) {
    if (detail.detail?.site) { site = detail.detail.site; view = 'site'; }
    if (detail.detail?.view) view = detail.detail.view;
    setView(view);
  } else {
    clearInterval(timer);
    timer = null;
  }
});

function schedule() {
  clearInterval(timer);
  if (!hudVisible) return;
  const ms = view === 'live' ? 10_000 : 60_000;
  timer = setInterval(() => { if (!document.hidden && hudVisible) render(); }, ms);
}

document.addEventListener('visibilitychange', () => {
  if (!document.hidden && hudVisible) render();
});

function render() {
  ({ overview: renderOverview, site: renderSite, live: renderLive, revenue: renderRevenue })[view]();
}

// ==========================================================================
// OVERVIEW — full arc reactor + orbiting site nodes + toplines
// ==========================================================================

async function renderOverview() {
  if (!body.querySelector('.hud-overview')) {
    body.innerHTML = `
      <div class="hud-overview">
        <div class="reactor-stage"><svg id="big-reactor" viewBox="0 0 420 420"></svg></div>
        <div class="hud-side">
          <div class="card hud-panel">
            <div class="card-head"><span class="card-label">Today</span><span class="live-now"><span class="live-dot"></span><span class="num" id="ov-live">0</span> / 30 min</span></div>
            <div class="topline"><span class="card-label">Pageviews</span><span class="num big" id="ov-pv">—</span><span class="delta" id="ov-pv-d"></span></div>
            <div class="topline"><span class="card-label">Uniques</span><span class="num big" id="ov-uv">—</span><span class="delta" id="ov-uv-d"></span></div>
            <div class="topline"><span class="card-label">7-day avg/day</span><span class="num big" id="ov-avg">—</span><span></span></div>
            <div class="topline"><span class="card-label">Last event</span><span class="num" id="ov-last">—</span><span></span></div>
          </div>
          <div class="card hud-panel">
            <div class="card-head"><span class="card-label">Anomalies</span></div>
            <div class="chips" id="ov-anomalies"><span class="card-label dimmer">none — all quiet</span></div>
          </div>
        </div>
      </div>`;
    buildReactor();
  }
  let res;
  try {
    res = await hudGet('/api/overview');
  } catch (err) {
    body.innerHTML = `<div class="hud-fail">analytics offline — ${String(err).replace(/</g, '&lt;')}<br/><button class="link-btn" data-goto-tab="settings">Settings →</button></div>`;
    return;
  }
  const o = res.data;

  const yesterday = await hudGet(`/api/summary?from=${istToday()}&to=${istToday()}`).catch(() => null);
  const d = yesterday?.data?.delta;
  paintDelta('ov-pv-d', d?.pageviews);
  paintDelta('ov-uv-d', d?.uniques);

  setText('ov-pv', fmt(o.today?.pageviews));
  setText('ov-uv', fmt(o.today?.uniques));
  setText('ov-avg', fmt(Math.round(o.weekAvgDaily ?? 0)));
  setText('ov-live', fmt(o.last30minEvents ?? 0));
  setText('ov-last', o.lastEventTs ? agoLabel(o.lastEventTs) : '—');

  const chips = document.getElementById('ov-anomalies');
  if (o.anomalies?.length) {
    chips.innerHTML = '';
    for (const a of o.anomalies) {
      const chip = document.createElement('span');
      chip.className = 'chip';
      chip.textContent = `${a.site} ${a.pct > 0 ? '+' : ''}${a.pct}% (${a.kind})`;
      chips.appendChild(chip);
    }
  } else {
    chips.innerHTML = '<span class="card-label dimmer">none — all quiet</span>';
  }

  updateReactor(o);
}

const NS = 'http://www.w3.org/2000/svg';
const mk = (tag, attrs = {}) => {
  const e = document.createElementNS(NS, tag);
  for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, v);
  return e;
};

function buildReactor() {
  const svg = document.getElementById('big-reactor');
  svg.innerHTML = `<defs>
    <radialGradient id="br-core" cx="50%" cy="50%">
      <stop offset="0%" stop-color="rgba(234,252,255,0.95)"/>
      <stop offset="45%" stop-color="rgba(79,216,255,0.5)"/>
      <stop offset="100%" stop-color="rgba(30,200,255,0)"/>
    </radialGradient>
  </defs>`;
  const C = 210;
  // Structure rings (static, hairline).
  svg.append(
    mk('circle', { cx: C, cy: C, r: 200, fill: 'none', stroke: 'rgba(78,216,255,0.12)', 'stroke-width': 1 }),
    mk('circle', { cx: C, cy: C, r: 178, fill: 'none', stroke: 'rgba(78,216,255,0.2)', 'stroke-width': 1, 'stroke-dasharray': '1 7' }),
  );
  // Animated rings (live traffic pulse drives speed).
  const g1 = mk('g', { id: 'br-spin1', style: 'transform-origin:210px 210px; animation: spin 14s linear infinite' });
  g1.append(mk('circle', { cx: C, cy: C, r: 120, fill: 'none', stroke: 'rgba(79,216,255,0.5)', 'stroke-width': 2, 'stroke-dasharray': '90 28 14 28' }));
  const g2 = mk('g', { id: 'br-spin2', style: 'transform-origin:210px 210px; animation: spin 22s linear infinite reverse' });
  g2.append(mk('circle', { cx: C, cy: C, r: 96, fill: 'none', stroke: 'rgba(30,200,255,0.4)', 'stroke-width': 6, 'stroke-dasharray': '4 14' }));
  const core = mk('circle', { id: 'br-core-c', cx: C, cy: C, r: 72, fill: 'url(#br-core)', style: 'animation: pulse 2.8s ease-in-out infinite' });
  svg.append(g1, g2, core);

  // Center readout.
  const t1 = mk('text', { id: 'br-n', x: C, y: C + 2, 'text-anchor': 'middle', fill: '#eafcff', style: 'font:700 30px ui-monospace,Consolas,monospace; font-variant-numeric:tabular-nums' });
  t1.textContent = '—';
  const t2 = mk('text', { x: C, y: C + 24, 'text-anchor': 'middle', fill: '#6d93ab', style: 'font:600 9px ui-monospace,Consolas,monospace; letter-spacing:0.2em' });
  t2.textContent = 'VIEWS TODAY';
  svg.append(t1, t2);

  // 5 orbiting site nodes.
  SITES.forEach((id, i) => {
    const a = (-90 + i * 72) * Math.PI / 180;
    const x = C + 152 * Math.cos(a);
    const y = C + 152 * Math.sin(a);
    const g = mk('g', { id: `br-node-${id}`, class: 'br-node', style: 'cursor:pointer' });
    g.append(
      mk('circle', { cx: x, cy: y, r: 22, fill: 'transparent' }), // hit area
      mk('circle', { id: `br-dot-${id}`, cx: x, cy: y, r: 7, fill: 'rgba(79,216,255,0.25)' }),
    );
    const label = mk('text', {
      x, y: y + (Math.sin(a) > 0.5 ? 26 : -18), 'text-anchor': 'middle',
      fill: '#6d93ab', style: 'font:600 8px ui-monospace,Consolas,monospace; letter-spacing:0.12em',
      id: `br-label-${id}`,
    });
    label.textContent = id.toUpperCase();
    g.append(label);
    g.addEventListener('click', () => { site = id; setView('site'); });
    svg.append(g);
  });
}

function updateReactor(o) {
  const n = document.getElementById('br-n');
  if (n) n.textContent = fmt(o.today?.pageviews ?? 0);
  // Pulse rate follows live traffic.
  const ev = o.last30minEvents ?? 0;
  const coreDur = Math.max(0.9, 2.8 - ev / 40);
  const el = document.getElementById('br-core-c');
  if (el) el.style.animationDuration = `${coreDur.toFixed(2)}s`;
  const s1 = document.getElementById('br-spin1');
  if (s1) s1.style.animationDuration = `${Math.max(4, 14 - ev / 10).toFixed(1)}s`;

  for (const s of o.today?.sites ?? []) {
    const dot = document.getElementById(`br-dot-${s.id}`);
    const label = document.getElementById(`br-label-${s.id}`);
    if (!dot) continue;
    const share = s.share ?? 0;
    dot.setAttribute('r', String(6 + share * 10));
    dot.setAttribute('fill', `rgba(79,216,255,${(0.2 + share * 0.8).toFixed(2)})`);
    dot.style.filter = share > 0.04 ? `drop-shadow(0 0 ${Math.round(3 + share * 12)}px rgba(79,216,255,${(share).toFixed(2)}))` : '';
    if (label) label.textContent = `${s.id.toUpperCase()} · ${fmt(s.pv)}`;
  }
}

// ==========================================================================
// SITE drill-in — range picker, chart, breakdowns
// ==========================================================================

const RANGES = { today: 0, '7d': 6, '30d': 29, '90d': 89 };

async function renderSite() {
  if (!body.querySelector('.hud-site')) {
    body.innerHTML = `
      <div class="hud-site">
        <div class="hud-toolbar">
          <div class="seg" id="site-pick">${SITES.map((s) =>
            `<button data-site="${s}" class="${s === site ? 'active' : ''}">${SITE_LABEL[s]}</button>`).join('')}</div>
          <div class="seg" id="range-pick">${Object.keys(RANGES).map((r) =>
            `<button data-range="${r}" class="${r === range ? 'active' : ''}">${r}</button>`).join('')}</div>
        </div>
        <div class="card hud-panel chart-panel">
          <div class="card-head">
            <span class="card-label" id="site-title"></span>
            <span class="num" id="site-totals"></span>
          </div>
          <canvas id="site-chart"></canvas>
          <div class="chart-legend"><span class="lg pv">■ pageviews</span><span class="lg uv">■ uniques</span><span class="card-label dimmer" id="chart-hover"></span></div>
        </div>
        <div class="hud-breakdowns" id="breakdowns"></div>
      </div>`;
    body.querySelector('#site-pick').addEventListener('click', (e) => {
      const b = e.target.closest('[data-site]');
      if (b) { site = b.dataset.site; renderSite(); }
    });
    body.querySelector('#range-pick').addEventListener('click', (e) => {
      const b = e.target.closest('[data-range]');
      if (b) { range = b.dataset.range; renderSite(); }
    });
  }
  body.querySelectorAll('#site-pick [data-site]').forEach((b) => b.classList.toggle('active', b.dataset.site === site));
  body.querySelectorAll('#range-pick [data-range]').forEach((b) => b.classList.toggle('active', b.dataset.range === range));

  const to = istToday();
  const from = shiftDate(to, -RANGES[range]);
  const gran = range === 'today' ? 'hour' : 'day';
  document.getElementById('site-title').textContent = `${SITE_LABEL[site]} — ${from} → ${to}`;

  try {
    const [ts, summary] = await Promise.all([
      hudGet(`/api/timeseries?site=${site}&from=${from}&to=${to}&granularity=${gran}`),
      hudGet(`/api/summary?site=${site}&from=${from}&to=${to}`),
    ]);
    const cur = summary.data.current || {};
    const d = summary.data.delta || {};
    document.getElementById('site-totals').textContent =
      `${fmt(cur.pageviews)} pv · ${fmt(cur.uniques)} uniq` +
      (d.pageviews != null ? `  (${d.pageviews >= 0 ? '+' : ''}${d.pageviews}%)` : '');
    drawChart(document.getElementById('site-chart'), ts.data.points || [], gran);
  } catch (err) {
    document.getElementById('site-totals').textContent = String(err);
  }

  // Breakdown panels.
  const dims = [['page', 'Top Pages'], ['referrer', 'Referrers'], ['country', 'Countries'], ['device', 'Devices'], ['browser', 'Browsers']];
  const wrap = document.getElementById('breakdowns');
  if (!wrap.children.length) {
    wrap.innerHTML = dims.map(([dim, label]) => `
      <div class="card hud-panel bd-panel">
        <div class="card-head"><span class="card-label">${label}</span></div>
        <ul class="bd-list" id="bd-${dim}"><li class="card-label dimmer">…</li></ul>
      </div>`).join('');
  }
  await Promise.all(dims.map(async ([dim]) => {
    const ul = document.getElementById(`bd-${dim}`);
    try {
      const res = await hudGet(`/api/breakdown?site=${site}&from=${from}&to=${to}&dim=${dim}&limit=8`);
      const items = res.data.items || [];
      ul.innerHTML = items.length ? '' : '<li class="card-label dimmer">no data</li>';
      for (const it of items) {
        const li = document.createElement('li');
        li.innerHTML = `<span class="bd-k"></span><span class="bd-bar"><span style="width:${Math.min(100, it.pct)}%"></span></span><span class="bd-n num"></span>`;
        li.querySelector('.bd-k').textContent = it.k || '(none)';
        li.querySelector('.bd-k').title = it.k || '';
        li.querySelector('.bd-n').textContent = fmt(it.n);
        ul.appendChild(li);
      }
    } catch {
      ul.innerHTML = '<li class="card-label dimmer">unavailable</li>';
    }
  }));
}

// ---------- DPR-aware canvas chart ----------

function drawChart(canvas, points, gran) {
  const dpr = window.devicePixelRatio || 1;
  const cssW = canvas.parentElement.clientWidth - 32;
  const cssH = 220;
  canvas.style.width = `${cssW}px`;
  canvas.style.height = `${cssH}px`;
  canvas.width = Math.round(cssW * dpr);
  canvas.height = Math.round(cssH * dpr);
  const ctx = canvas.getContext('2d');
  ctx.scale(dpr, dpr);
  ctx.clearRect(0, 0, cssW, cssH);
  if (!points.length) return;

  const padL = 44, padR = 8, padT = 10, padB = 22;
  const W = cssW - padL - padR, H = cssH - padT - padB;
  const maxY = Math.max(1, ...points.map((p) => p.pv));
  const nice = niceMax(maxY);
  const x = (i) => padL + (points.length === 1 ? W / 2 : (i / (points.length - 1)) * W);
  const y = (v) => padT + H - (v / nice) * H;

  // grid
  ctx.strokeStyle = 'rgba(78,216,255,0.10)';
  ctx.fillStyle = '#44607a';
  ctx.font = `10px ui-monospace, Consolas, monospace`;
  ctx.lineWidth = 1;
  for (let g = 0; g <= 4; g++) {
    const v = (nice / 4) * g;
    const yy = Math.round(y(v)) + 0.5;
    ctx.beginPath(); ctx.moveTo(padL, yy); ctx.lineTo(cssW - padR, yy); ctx.stroke();
    ctx.fillText(compact(v), 4, yy + 3);
  }
  // x labels (≤6)
  const stepN = Math.max(1, Math.ceil(points.length / 6));
  ctx.textAlign = 'center';
  for (let i = 0; i < points.length; i += stepN) {
    const t = points[i].t;
    const label = gran === 'hour' ? t.slice(11, 16) : t.slice(5);
    ctx.fillText(label, x(i), cssH - 6);
  }
  ctx.textAlign = 'left';

  // pv area + line
  ctx.beginPath();
  points.forEach((p, i) => (i ? ctx.lineTo(x(i), y(p.pv)) : ctx.moveTo(x(i), y(p.pv))));
  ctx.strokeStyle = 'rgba(79,216,255,0.9)';
  ctx.lineWidth = 1.6;
  ctx.stroke();
  ctx.lineTo(x(points.length - 1), y(0));
  ctx.lineTo(x(0), y(0));
  ctx.closePath();
  const grad = ctx.createLinearGradient(0, padT, 0, padT + H);
  grad.addColorStop(0, 'rgba(79,216,255,0.22)');
  grad.addColorStop(1, 'rgba(79,216,255,0.0)');
  ctx.fillStyle = grad;
  ctx.fill();

  // uv line
  ctx.beginPath();
  points.forEach((p, i) => (i ? ctx.lineTo(x(i), y(p.uv)) : ctx.moveTo(x(i), y(p.uv))));
  ctx.strokeStyle = 'rgba(111,227,165,0.8)';
  ctx.lineWidth = 1.3;
  ctx.stroke();

  // hover readout
  canvas.onmousemove = (e) => {
    const r = canvas.getBoundingClientRect();
    const i = Math.round(((e.clientX - r.left - padL) / W) * (points.length - 1));
    const p = points[Math.max(0, Math.min(points.length - 1, i))];
    if (p) document.getElementById('chart-hover').textContent = `${p.t} — ${fmt(p.pv)} pv · ${fmt(p.uv)} uniq`;
  };
  canvas.onmouseleave = () => { document.getElementById('chart-hover').textContent = ''; };
}

function niceMax(v) {
  const mag = 10 ** Math.floor(Math.log10(v));
  for (const m of [1, 2, 2.5, 5, 10]) if (v <= m * mag) return m * mag;
  return 10 * mag;
}
const compact = (n) => (n >= 1000 ? `${(n / 1000).toFixed(n >= 10000 ? 0 : 1)}k` : String(Math.round(n)));

// ==========================================================================
// LIVE — rolling 30-min counters + event stream (10s)
// ==========================================================================

async function renderLive() {
  if (!body.querySelector('.hud-live')) {
    body.innerHTML = `
      <div class="hud-live">
        <div class="live-counters" id="live-counters"></div>
        <div class="card hud-panel stream-panel">
          <div class="card-head"><span class="card-label">Event stream — last 30 min</span><span class="live-now"><span class="live-dot"></span>LIVE</span></div>
          <ul class="stream" id="live-stream"></ul>
        </div>
      </div>`;
  }
  let res;
  try { res = await hudGet('/api/live'); } catch (err) { setStale(null); staleEl.textContent = String(err); return; }
  const data = res.data;

  const counters = document.getElementById('live-counters');
  counters.innerHTML = '';
  for (const c of data.counters || []) {
    const div = document.createElement('div');
    div.className = 'card hud-panel live-counter';
    div.innerHTML = `<span class="card-label">${c.label || c.id}</span><span class="num big ${c.n > 0 ? 'glow-live' : ''}">${fmt(c.n)}</span>`;
    div.style.cursor = 'pointer';
    div.addEventListener('click', () => { site = c.id; setView('site'); });
    counters.appendChild(div);
  }

  const ul = document.getElementById('live-stream');
  ul.innerHTML = (data.events || []).length ? '' : '<li class="card-label dimmer" style="padding:8px">no events in the last 30 minutes</li>';
  for (const e of (data.events || []).slice(0, 40)) {
    const li = document.createElement('li');
    const t = new Date(e.ts).toLocaleTimeString('en-GB', { hour: '2-digit', minute: '2-digit', second: '2-digit' });
    li.innerHTML = `<span class="num st-t"></span><span class="st-site"></span><span class="st-path"></span><span class="st-meta"></span>`;
    li.querySelector('.st-t').textContent = t;
    li.querySelector('.st-site').textContent = e.site;
    li.querySelector('.st-path').textContent = e.path || '/';
    li.querySelector('.st-path').title = e.path || '/';
    li.querySelector('.st-meta').textContent = `${e.country || '—'} · ${e.device || '—'}`;
    ul.appendChild(li);
  }
}

// ==========================================================================
// REVENUE — view + add/edit monthly entries (Worker POST/DELETE, Access-gated)
// ==========================================================================

const SOURCES = ['adsense', 'ezoic', 'affiliate', 'other'];

async function renderRevenue() {
  if (!body.querySelector('.hud-revenue')) {
    body.innerHTML = `
      <div class="hud-revenue">
        <div class="hud-toolbar">
          <label class="card-label" style="display:flex;align-items:center;gap:8px">Month
            <input type="month" id="rev-month" value="${istMonth()}" />
          </label>
          <div class="fx-box card-label">FX <input type="text" id="rev-fx" style="width:64px" /> ₹/$
            <button class="btn" id="rev-fx-save">Set</button>
          </div>
        </div>
        <div class="card hud-panel">
          <div class="card-head"><span class="card-label">By site</span><span class="num" id="rev-total"></span></div>
          <table class="rev-table">
            <thead><tr><th>Site</th><th>USD</th><th>INR</th><th>Pageviews</th><th>RPM $</th></tr></thead>
            <tbody id="rev-rows"></tbody>
          </table>
        </div>
        <div class="card hud-panel">
          <div class="card-head"><span class="card-label">Entries</span></div>
          <div class="rev-add" id="rev-add">
            <select id="ra-site">${SITES.map((s) => `<option value="${s}">${s}</option>`).join('')}</select>
            <select id="ra-source">${SOURCES.map((s) => `<option value="${s}">${s}</option>`).join('')}</select>
            <input type="text" id="ra-amount" placeholder="amount" style="width:88px" />
            <select id="ra-currency"><option>USD</option><option>INR</option></select>
            <button class="btn" id="ra-add">Add</button>
            <span class="card-label dimmer" id="ra-state"></span>
          </div>
          <ul class="rev-entries" id="rev-entries"></ul>
        </div>
      </div>`;
    document.getElementById('rev-month').addEventListener('change', renderRevenue);
    document.getElementById('rev-fx-save').addEventListener('click', async () => {
      const month = document.getElementById('rev-month').value;
      const rate = Number(document.getElementById('rev-fx').value);
      if (!(rate > 0)) return;
      await inv('worker_mutate', { method: 'POST', path: '/api/revenue/rate', body: { month, rate } });
      renderRevenue();
    });
    document.getElementById('ra-add').addEventListener('click', async () => {
      const state = document.getElementById('ra-state');
      const entry = {
        month: document.getElementById('rev-month').value,
        site: document.getElementById('ra-site').value,
        source: document.getElementById('ra-source').value,
        amount: Number(document.getElementById('ra-amount').value),
        currency: document.getElementById('ra-currency').value,
      };
      if (!(entry.amount >= 0)) { state.textContent = 'bad amount'; return; }
      state.textContent = 'saving…';
      try {
        await inv('worker_mutate', { method: 'POST', path: '/api/revenue', body: entry });
        document.getElementById('ra-amount').value = '';
        state.textContent = '✓';
        setTimeout(() => (state.textContent = ''), 1500);
        renderRevenue();
      } catch (e) {
        state.textContent = String(e);
      }
    });
  }

  const month = document.getElementById('rev-month').value || istMonth();
  let res;
  try { res = await hudGet(`/api/revenue?month=${month}`); } catch (err) { staleEl.textContent = String(err); return; }
  const r = res.data;

  document.getElementById('rev-fx').placeholder = String(r.fx?.rate ?? 84);
  document.getElementById('rev-total').textContent = `$${fmt(r.total?.usd)} · ₹${fmt(r.total?.inr)}`;

  const rows = document.getElementById('rev-rows');
  rows.innerHTML = '';
  for (const s of r.sites || []) {
    const tr = document.createElement('tr');
    tr.innerHTML = `<td>${s.label || s.id}</td><td class="num">$${fmt(s.usd)}</td><td class="num">₹${fmt(s.inr)}</td><td class="num">${fmt(s.pageviews)}</td><td class="num">${s.rpmUsd != null ? `$${s.rpmUsd}` : '—'}</td>`;
    rows.appendChild(tr);
  }

  const ul = document.getElementById('rev-entries');
  ul.innerHTML = (r.entries || []).length ? '' : '<li class="card-label dimmer" style="padding:8px 0">no entries this month</li>';
  for (const e of r.entries || []) {
    const li = document.createElement('li');
    li.innerHTML = `<span>${e.site}</span><span>${e.source}</span><span class="num">${e.currency === 'USD' ? '$' : '₹'}${fmt(e.amount)}</span><button class="todo-mini" title="Delete">✕</button>`;
    li.querySelector('button').addEventListener('click', async () => {
      await inv('worker_mutate', { method: 'DELETE', path: `/api/revenue?id=${e.id}` });
      renderRevenue();
    });
    ul.appendChild(li);
  }
}

// ---------- shared tiny helpers ----------

function setText(id, v) { const el = document.getElementById(id); if (el) el.textContent = v; }
function paintDelta(id, pct) {
  const el = document.getElementById(id);
  if (!el) return;
  if (pct == null) { el.textContent = ''; el.className = 'delta flat'; return; }
  el.textContent = `${pct >= 0 ? '▲' : '▼'} ${Math.abs(pct).toFixed(1)}%`;
  el.className = `delta ${pct >= 0 ? 'up' : 'down'}`;
}
