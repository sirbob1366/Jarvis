// TAB 4 — Agents: JARVIS operates the portfolio. DISPATCH (site + instruction,
// typed or dictated, with templates) → ACTIVE JOBS (live log, cancel) → REVIEW
// QUEUE (per-file syntax-highlighted diff; Approve & Deploy / Request changes /
// Discard) → HISTORY (commit hashes, one-tap rollback). The deploy gate is
// enforced in Rust: nothing pushes without sir's explicit approval here.

import { inv } from './data.js';
import { dictate, isListening } from './voice.js';

const { listen } = window.__TAURI__.event;
const root = document.getElementById('agents-root');

let config = { enabled: false, allowlist: [], cli_available: false };
let jobs = []; // newest first
let history = [];
let agentsVisible = false;
let tick = null;

const TEMPLATES = {
  '': '',
  copy: 'Update the copy on <page>: <what to change>. Keep tone and formatting consistent with the rest of the site.',
  blog: 'Add a new blog post titled "<title>". Topic: <topic>. Match the structure, front-matter, and style of the existing posts.',
  bug: 'Fix this bug: <describe what is wrong and how to reproduce>. Expected behaviour: <what should happen>.',
  faq: 'Update the FAQ: <add/edit/remove which question>. Answer: <the answer>.',
};

// ---------- helpers ----------

const esc = (s) => String(s ?? '')
  .replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');

function elapsed(job) {
  const end = job.ended_at || Date.now();
  const s = Math.max(0, Math.round((end - job.started_at) / 1000));
  if (s < 60) return `${s}s`;
  return `${Math.floor(s / 60)}m ${s % 60}s`;
}

const STATUS_LABEL = {
  queued: 'queued', running: 'working', review: 'awaiting review',
  deploying: 'deploying', done: 'done', failed: 'failed',
  cancelled: 'cancelled', discarded: 'discarded', superseded: 'revised',
};

function upsert(job) {
  const i = jobs.findIndex((j) => j.id === job.id);
  if (i >= 0) jobs[i] = job;
  else jobs.unshift(job);
}

// ---------- skeleton ----------

function skeleton() {
  root.innerHTML = `
    <div class="agents-head">
      <span class="card-label">Agents</span>
      <span class="card-label dimmer" id="ag-cli"></span>
      <span class="card-label dimmer" id="ag-stale" style="margin-left:auto"></span>
    </div>

    <div class="agents-grid">
      <div class="card hud-panel ag-dispatch" id="ag-dispatch">
        <div class="card-head"><span class="card-label">Dispatch</span></div>
        <div class="ag-disabled-banner" id="ag-disabled" hidden>
          <span>Agents are disabled.</span>
          <button class="link-btn" data-goto-tab="settings">Enable in Settings →</button>
        </div>
        <div class="ag-form" id="ag-form">
          <div class="ag-row">
            <select id="ag-site" class="ag-select"></select>
            <select id="ag-template" class="ag-select" title="Instruction templates">
              <option value="">Templates…</option>
              <option value="copy">Update copy</option>
              <option value="blog">Add blog post</option>
              <option value="bug">Fix described bug</option>
              <option value="faq">Update FAQ</option>
            </select>
          </div>
          <div class="ag-instruction">
            <textarea id="ag-instruction" rows="3" placeholder="What should the agent do? (type or dictate)" autocomplete="off"></textarea>
            <button class="mic-btn ag-mic" id="ag-mic" title="Dictate">
              <svg viewBox="0 0 24 24"><rect x="9" y="3" width="6" height="11" rx="3"/><path d="M5 11a7 7 0 0 0 14 0"/><path d="M12 18v3"/></svg>
            </button>
          </div>
          <div class="ag-dispatch-actions">
            <button class="btn" id="ag-dispatch-btn">Dispatch agent</button>
            <span class="hint" id="ag-dispatch-msg"></span>
          </div>
        </div>
      </div>

      <div class="card hud-panel ag-col" id="ag-active-panel">
        <div class="card-head"><span class="card-label">Active jobs</span><span class="count-badge num" id="ag-active-n" hidden></span></div>
        <div class="ag-list" id="ag-active"></div>
        <div class="card-empty" id="ag-active-empty">no agents running</div>
      </div>

      <div class="card hud-panel ag-col ag-review-col" id="ag-review-panel">
        <div class="card-head"><span class="card-label">Review queue</span><span class="count-badge num" id="ag-review-n" hidden></span></div>
        <div class="ag-list" id="ag-review"></div>
        <div class="card-empty" id="ag-review-empty">nothing awaiting review</div>
      </div>

      <div class="card hud-panel ag-col" id="ag-history-panel">
        <div class="card-head"><span class="card-label">History</span></div>
        <div class="ag-list" id="ag-history"></div>
        <div class="card-empty" id="ag-history-empty">no deployments yet</div>
      </div>
    </div>`;

  wireForm();
}

function wireForm() {
  document.getElementById('ag-template').addEventListener('change', (e) => {
    const t = TEMPLATES[e.target.value];
    const box = document.getElementById('ag-instruction');
    if (t) { box.value = t; box.focus(); }
    e.target.value = '';
  });

  document.getElementById('ag-mic').addEventListener('click', () => {
    const mic = document.getElementById('ag-mic');
    const box = document.getElementById('ag-instruction');
    mic.classList.toggle('listening', !isListening());
    dictate((text) => {
      mic.classList.remove('listening');
      if (text) box.value = (box.value ? box.value.trim() + ' ' : '') + text;
      box.focus();
    });
  });

  document.getElementById('ag-dispatch-btn').addEventListener('click', dispatch);
}

// ---------- dispatch ----------

async function dispatch() {
  const site = document.getElementById('ag-site').value;
  const instruction = document.getElementById('ag-instruction').value.trim();
  const msg = document.getElementById('ag-dispatch-msg');
  if (!instruction) { msg.textContent = 'An instruction is required.'; return; }

  let target = site;
  if (site === '__auto') {
    target = (config.allowlist.find((e) => instruction.toLowerCase().includes(e.key.toLowerCase())) || {}).key;
    if (!target) { msg.textContent = 'Could not auto-detect a site — pick one explicitly.'; return; }
  }

  msg.textContent = 'dispatching…';
  try {
    await inv('agents_dispatch', { site: target, instruction });
    document.getElementById('ag-instruction').value = '';
    msg.textContent = `dispatched to ${target}`;
    setTimeout(() => { msg.textContent = ''; }, 4000);
  } catch (e) {
    msg.textContent = String(e);
  }
}

// ---------- render ----------

function renderDispatch() {
  const cli = document.getElementById('ag-cli');
  if (cli) {
    cli.textContent = config.cli_available
      ? `Claude Code ${config.cli_version || 'ready'}`
      : 'Claude Code CLI not found — install it to dispatch agents';
  }
  document.getElementById('ag-disabled').hidden = config.enabled;
  document.getElementById('ag-form').classList.toggle('locked', !config.enabled);

  const sel = document.getElementById('ag-site');
  if (sel && sel.dataset.n !== String(config.allowlist.length)) {
    sel.dataset.n = String(config.allowlist.length);
    sel.innerHTML = '<option value="__auto">Auto-detect</option>' +
      config.allowlist.map((e) => `<option value="${esc(e.key)}">${esc(e.key)}</option>`).join('');
  }
}

function jobCard(job) {
  const card = document.createElement('div');
  card.className = `ag-job ag-${job.status}`;
  card.dataset.id = job.id;
  card.innerHTML = `
    <div class="ag-job-head">
      <span class="ag-site-tag">${esc(job.site)}</span>
      <span class="ag-status ${job.status}">${STATUS_LABEL[job.status] || job.status}</span>
      <span class="ag-elapsed num"></span>
    </div>
    <div class="ag-instr">${esc(job.instruction)}</div>`;

  // streaming log (active jobs)
  if (job.status === 'running' || job.status === 'queued' || job.status === 'deploying') {
    const log = document.createElement('pre');
    log.className = 'ag-log';
    log.textContent = (job.log || []).slice(-40).join('\n');
    card.appendChild(log);
    if (job.status === 'running' || job.status === 'queued') {
      const cancel = mkBtn('Cancel', 'ag-btn-ghost', async () => {
        try { await inv('agents_cancel', { id: job.id }); } catch (e) { flash(card, e); }
      });
      card.appendChild(rowOf(cancel));
    }
  }

  if (job.error) {
    const err = document.createElement('div');
    err.className = 'ag-err';
    err.textContent = job.error;
    card.appendChild(err);
  }
  return card;
}

function reviewCard(job) {
  const card = document.createElement('div');
  card.className = 'ag-job ag-review';
  card.dataset.id = job.id;

  const summary = job.summary ? `<div class="ag-summary">${esc(job.summary)}</div>` : '';
  card.innerHTML = `
    <div class="ag-job-head">
      <span class="ag-site-tag">${esc(job.site)}</span>
      <span class="ag-status review">awaiting review</span>
      <span class="card-label dimmer">${job.files_changed} file${job.files_changed === 1 ? '' : 's'}</span>
    </div>
    <div class="ag-instr">${esc(job.instruction)}</div>
    ${summary}
    <div class="ag-files"></div>
    <div class="ag-review-actions"></div>
    <span class="hint ag-flash"></span>`;

  // per-file collapsible diffs
  const filesEl = card.querySelector('.ag-files');
  for (const f of job.files || []) {
    filesEl.appendChild(fileDiff(f));
  }

  // actions
  const actions = card.querySelector('.ag-review-actions');
  actions.appendChild(mkBtn('✅ Approve & Deploy', 'ag-btn-go', async () => {
    flash(card, 'deploying…');
    try {
      const r = await inv('agents_approve', { id: job.id });
      flash(card, `deployed ${String(r.commit).slice(0, 8)} · ${r.cloudflare || ''}`);
    } catch (e) { flash(card, e); }
  }));
  actions.appendChild(mkBtn('✏️ Request changes', 'ag-btn-ghost', () => askChanges(card, job)));
  actions.appendChild(mkBtn('🗑️ Discard', 'ag-btn-ghost', async () => {
    try { await inv('agents_discard', { id: job.id }); } catch (e) { flash(card, e); }
  }));
  return card;
}

function fileDiff(f) {
  const wrap = document.createElement('details');
  wrap.className = 'ag-file';
  const ins = (f.diff.match(/^\+(?!\+\+)/gm) || []).length;
  const del = (f.diff.match(/^-(?!--)/gm) || []).length;
  wrap.innerHTML = `<summary>
      <span class="ag-file-status s-${f.status[0]}">${esc(f.status)}</span>
      <span class="ag-file-path">${esc(f.path)}</span>
      <span class="ag-file-stat"><span class="add">+${ins}</span> <span class="del">−${del}</span></span>
    </summary>`;
  const pre = document.createElement('pre');
  pre.className = 'ag-diff';
  pre.innerHTML = highlightDiff(f.diff);
  wrap.appendChild(pre);
  return wrap;
}

function highlightDiff(diff) {
  return diff.split('\n').map((line) => {
    let cls = 'ctx';
    if (line.startsWith('+') && !line.startsWith('+++')) cls = 'add';
    else if (line.startsWith('-') && !line.startsWith('---')) cls = 'del';
    else if (line.startsWith('@@')) cls = 'hunk';
    else if (line.startsWith('diff ') || line.startsWith('index ') || line.startsWith('+++') || line.startsWith('---')) cls = 'meta';
    return `<span class="dl ${cls}">${esc(line) || ' '}</span>`;
  }).join('\n');
}

function askChanges(card, job) {
  const box = card.querySelector('.ag-followup');
  if (box) { box.remove(); return; }
  const wrap = document.createElement('div');
  wrap.className = 'ag-followup';
  wrap.innerHTML = `<textarea rows="2" placeholder="What should the agent change?"></textarea>`;
  const send = mkBtn('Send to agent', 'ag-btn-go', async () => {
    const follow = wrap.querySelector('textarea').value.trim();
    if (!follow) return;
    flash(card, 'sending follow-up…');
    try { await inv('agents_request_changes', { id: job.id, followUp: follow }); }
    catch (e) { flash(card, e); }
  });
  wrap.appendChild(rowOf(send));
  card.querySelector('.ag-review-actions').after(wrap);
  wrap.querySelector('textarea').focus();
}

function historyRow(h) {
  const row = document.createElement('div');
  row.className = 'ag-hist-row';
  const when = new Date(h.ts).toLocaleString('en-GB', { day: '2-digit', month: 'short', hour: '2-digit', minute: '2-digit' });
  const isRollback = h.action === 'rollback';
  row.innerHTML = `
    <div class="ag-hist-main">
      <span class="ag-site-tag">${esc(h.site)}</span>
      <span class="ag-hist-instr">${esc(h.instruction)}</span>
    </div>
    <div class="ag-hist-meta">
      <span class="num ag-hash" title="${esc(h.commit_hash)}">${esc(String(h.commit_hash).slice(0, 8))}</span>
      <span class="card-label dimmer">${when}${isRollback ? ' · revert' : ''}</span>
    </div>`;
  if (!isRollback && h.commit_hash) {
    const rb = mkBtn('Rollback', 'ag-btn-ghost ag-rollback', async () => {
      if (rb.dataset.armed !== '1') { rb.dataset.armed = '1'; rb.textContent = 'Confirm rollback?'; return; }
      rb.textContent = 'reverting…';
      try {
        const r = await inv('agents_rollback', { site: h.site, hash: h.commit_hash });
        rb.textContent = `reverted ${String(r.revert).slice(0, 8)}`;
      } catch (e) { rb.textContent = 'failed'; rb.title = String(e); }
    });
    row.querySelector('.ag-hist-meta').appendChild(rb);
  }
  return row;
}

function renderJobs() {
  if (!root.querySelector('.agents-grid')) return;
  const active = jobs.filter((j) => ['running', 'queued', 'deploying'].includes(j.status));
  const reviews = jobs.filter((j) => j.status === 'review');

  fill('ag-active', 'ag-active-empty', active, jobCard);
  fill('ag-review', 'ag-review-empty', reviews, reviewCard);

  badge('ag-active-n', active.length);
  badge('ag-review-n', reviews.length);
  const railBadge = document.getElementById('rail-agents-badge');
  if (railBadge) { railBadge.hidden = reviews.length === 0; railBadge.textContent = reviews.length; }
}

function renderHistory() {
  fill('ag-history', 'ag-history-empty', history, historyRow);
}

function fill(listId, emptyId, items, make) {
  const list = document.getElementById(listId);
  if (!list) return;
  list.innerHTML = '';
  for (const it of items) list.appendChild(make(it));
  document.getElementById(emptyId).hidden = items.length > 0;
}

function badge(id, n) {
  const el = document.getElementById(id);
  if (el) { el.hidden = !n; el.textContent = n; }
}

// ---------- small DOM helpers ----------

function mkBtn(label, cls, onClick) {
  const b = document.createElement('button');
  b.className = `ag-act ${cls}`;
  b.textContent = label;
  b.addEventListener('click', onClick);
  return b;
}
function rowOf(...els) {
  const d = document.createElement('div');
  d.className = 'ag-btn-row';
  d.append(...els);
  return d;
}
function flash(card, msg) {
  const el = card.querySelector('.ag-flash');
  if (el) el.textContent = String(msg);
}

// ---------- data ----------

async function refresh() {
  if (!root.querySelector('.agents-grid')) skeleton();
  try {
    config = await inv('agents_config');
  } catch { /* keep last */ }
  renderDispatch();
  try {
    const data = await inv('agents_jobs');
    jobs = data.jobs || [];
    history = data.history || [];
  } catch { /* keep last */ }
  renderJobs();
  renderHistory();
}

// ---------- live events ----------

listen('agent-update', ({ payload }) => {
  upsert(payload);
  if (agentsVisible) renderJobs();
  else {
    const railBadge = document.getElementById('rail-agents-badge');
    const n = jobs.filter((j) => j.status === 'review').length;
    if (railBadge) { railBadge.hidden = n === 0; railBadge.textContent = n; }
  }
});

listen('agent-log', ({ payload }) => {
  const job = jobs.find((j) => j.id === payload.id);
  if (job) { (job.log = job.log || []).push(payload.line); }
  if (!agentsVisible) return;
  const card = root.querySelector(`.ag-job[data-id="${payload.id}"] .ag-log`);
  if (card) { card.textContent += (card.textContent ? '\n' : '') + payload.line; card.scrollTop = card.scrollHeight; }
});

listen('agent-review-ready', () => {
  // refresh history mapping isn't needed; jobs already updated via agent-update
  if (agentsVisible) renderJobs();
});

// elapsed-time ticker for active cards
function startTick() {
  clearInterval(tick);
  tick = setInterval(() => {
    if (!agentsVisible) return;
    for (const j of jobs) {
      if (!['running', 'queued', 'deploying'].includes(j.status)) continue;
      const el = root.querySelector(`.ag-job[data-id="${j.id}"] .ag-elapsed`);
      if (el) el.textContent = elapsed(j);
    }
  }, 1000);
}

document.addEventListener('tab-shown', ({ detail }) => {
  agentsVisible = detail.tab === 'agents';
  if (agentsVisible) { refresh(); startTick(); }
  else clearInterval(tick);
});

// Prime the rail badge once at startup so pending reviews show before first visit.
inv('agents_jobs').then((d) => {
  jobs = d.jobs || [];
  const n = jobs.filter((j) => j.status === 'review').length;
  const railBadge = document.getElementById('rail-agents-badge');
  if (railBadge) { railBadge.hidden = n === 0; railBadge.textContent = n; }
}).catch(() => {});
