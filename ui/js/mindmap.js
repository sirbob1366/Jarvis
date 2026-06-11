// TAB 5 — Mind Map: a visual, navigable map of the JARVIS-OS vault.
// Tidy-tree layout (root left → leaves right; stays readable at ~100 nodes),
// collapsible branches, pan/zoom, search-to-highlight. Click a file → side
// panel with rendered markdown, edit-in-place (saves + git-commits), and an
// "Ask JARVIS" hand-off. Domain tints: work amber / business cyan / personal
// green — glow only on the active path. Live: vault writes pulse the node;
// the weekly audit renders as an overlay scorecard.

import { inv } from './data.js';
import { send } from './app.js';

const { listen } = window.__TAURI__.event;

const root = document.getElementById('mm-root');
const DOMAIN_COLOR = {
  work: '#ffb84d',
  business: '#4fd8ff',
  personal: '#6fe3a5',
};

let tree = null;
let collapsed = new Set(JSON.parse(localStorage.getItem('mm-collapsed') || '[]'));
let selected = null;
let view = { x: 40, y: 0, k: 1 };
let mmVisible = false;

// ---------- skeleton ----------

function skeleton() {
  root.innerHTML = `
    <div class="mm-toolbar">
      <input id="mm-search" type="text" placeholder="search the vault…" autocomplete="off" />
      <button class="btn" id="mm-audit-btn">Audit</button>
      <button class="btn" id="mm-onboard-btn">Onboard</button>
      <span class="card-label dimmer" id="mm-status"></span>
    </div>
    <div class="mm-stage">
      <svg id="mm-svg"></svg>
      <aside class="mm-panel card" id="mm-panel" hidden>
        <div class="card-head">
          <span class="card-label" id="mm-panel-title"></span>
          <div>
            <button class="win-btn" id="mm-edit-btn" title="Edit">✎</button>
            <button class="win-btn" id="mm-ask-btn" title="Ask JARVIS about this">⌬</button>
            <button class="win-btn" id="mm-panel-close" title="Close">✕</button>
          </div>
        </div>
        <div class="mm-md" id="mm-md"></div>
        <div class="mm-edit" id="mm-edit" hidden>
          <textarea id="mm-edit-text" spellcheck="false"></textarea>
          <div style="display:flex;gap:8px;margin-top:8px">
            <button class="btn" id="mm-save-btn">Save + commit</button>
            <button class="btn" id="mm-cancel-btn">Cancel</button>
          </div>
        </div>
      </aside>
      <div class="mm-overlay card" id="mm-audit" hidden>
        <div class="card-head"><span class="card-label">Context audit — Four-Cs</span><button class="win-btn" id="mm-audit-close">✕</button></div>
        <pre id="mm-audit-text"></pre>
        <button class="btn" id="mm-audit-run">Run audit now</button>
      </div>
      <div class="mm-init card" id="mm-init" hidden>
        <p>JARVIS-OS is not initialized. One tap creates <span class="num">~/JARVIS-OS</span> —
        a git-versioned markdown vault (AIS-OS skeleton, adapted to work / business / personal) —
        and the brain starts working from it.</p>
        <button class="btn wide" id="mm-init-btn">Initialize the vault</button>
        <span class="hint" id="mm-init-state"></span>
      </div>
    </div>`;

  document.getElementById('mm-search').addEventListener('input', () => draw());
  document.getElementById('mm-panel-close').addEventListener('click', () => { selected = null; panel(false); draw(); });
  document.getElementById('mm-audit-btn').addEventListener('click', showAudit);
  document.getElementById('mm-audit-close').addEventListener('click', () => (document.getElementById('mm-audit').hidden = true));
  document.getElementById('mm-audit-run').addEventListener('click', () => {
    document.getElementById('mm-audit').hidden = true;
    send('Run the vault /audit skill: read CLAUDE.md, the three domains, decisions and connections, then give me the Four-Cs gap report and the one-line spoken summary.');
  });
  document.getElementById('mm-onboard-btn').addEventListener('click', () =>
    send('Run the vault /onboard interview: the seven questions, three-domain version. Pre-fill what you already know and keep it short, sir will answer by voice or text.'));
  document.getElementById('mm-init-btn').addEventListener('click', async () => {
    const state = document.getElementById('mm-init-state');
    state.textContent = 'initializing…';
    try {
      state.textContent = await inv('vault_init');
      await refresh();
    } catch (e) {
      state.textContent = String(e);
    }
  });
  document.getElementById('mm-ask-btn').addEventListener('click', () => {
    if (selected) send(`Read ${selected} from the vault (vault_read) and brief me on it — anything stale or contradictory, flag it.`);
  });
  document.getElementById('mm-edit-btn').addEventListener('click', async () => {
    if (!selected) return;
    document.getElementById('mm-edit-text').value = await inv('vault_read_file', { path: selected });
    document.getElementById('mm-md').hidden = true;
    document.getElementById('mm-edit').hidden = false;
  });
  document.getElementById('mm-cancel-btn').addEventListener('click', () => {
    document.getElementById('mm-edit').hidden = true;
    document.getElementById('mm-md').hidden = false;
  });
  document.getElementById('mm-save-btn').addEventListener('click', async () => {
    await inv('vault_write_file', { path: selected, content: document.getElementById('mm-edit-text').value });
    document.getElementById('mm-edit').hidden = true;
    document.getElementById('mm-md').hidden = false;
    openFile(selected);
  });

  wireSvgNav();
}

// ---------- data ----------

async function refresh() {
  if (!root.querySelector('.mm-stage')) skeleton();
  try {
    const res = await inv('vault_tree');
    tree = { name: 'JARVIS-OS', path: '', dir: true, children: res.tree };
    document.getElementById('mm-init').hidden = true;
    document.getElementById('mm-status').textContent = res.root;
    draw();
  } catch {
    tree = null;
    document.getElementById('mm-init').hidden = false;
    document.getElementById('mm-svg').innerHTML = '';
  }
  try {
    const st = await inv('vault_status');
    if (st.exists) {
      const bits = [st.git_dirty ? 'git: dirty' : 'git: clean'];
      if (st.last_audit_date) bits.push(`audit ${st.last_audit_date}`);
      document.getElementById('mm-status').textContent = `${st.root} · ${bits.join(' · ')}`;
    }
  } catch { /* status is decoration */ }
}

// ---------- tidy-tree layout ----------

const ROW = 26, COL = 200;

function layout(node, depth, slot, out, domain) {
  const id = node.path || 'ROOT';
  const myDomain = depth === 1 && DOMAIN_COLOR[node.name] ? node.name : domain;
  const kids = node.dir && !collapsed.has(id) ? node.children || [] : [];
  let y;
  if (!kids.length) {
    y = slot.next++;
  } else {
    const ys = kids.map((k) => layout(k, depth + 1, slot, out, myDomain).y);
    y = (Math.min(...ys) + Math.max(...ys)) / 2;
  }
  const n = { id, node, depth, y, domain: myDomain, kids: kids.map((k) => k.path || 'ROOT') };
  out.push(n);
  return n;
}

function draw() {
  const svg = document.getElementById('mm-svg');
  if (!svg || !tree) return;
  const q = (document.getElementById('mm-search')?.value || '').toLowerCase().trim();

  const nodes = [];
  layout(tree, 0, { next: 0 }, nodes, null);
  const byId = new Map(nodes.map((n) => [n.id, n]));

  // Ancestors of the selection form the active (glowing) path.
  const active = new Set();
  if (selected != null) {
    let parts = selected.split('/');
    while (parts.length) {
      active.add(parts.join('/'));
      parts.pop();
    }
    active.add('ROOT');
  }

  const H = (Math.max(...nodes.map((n) => n.y)) + 2) * ROW;
  const W = (Math.max(...nodes.map((n) => n.depth)) + 1) * COL + 240;
  svg.setAttribute('viewBox', `${-view.x / view.k} ${-view.y / view.k} ${svg.clientWidth / view.k} ${svg.clientHeight / view.k}`);

  let paths = '';
  let dots = '';
  for (const n of nodes) {
    const x = n.depth * COL + 16;
    const y = n.y * ROW + 24;
    const color = DOMAIN_COLOR[n.domain] || '#6d93ab';
    const isActive = active.has(n.id);
    const hit = q && (n.node.name.toLowerCase().includes(q) || (n.node.preview || '').toLowerCase().includes(q));

    for (const kid of n.kids) {
      const c = byId.get(kid);
      if (!c) continue;
      const cx = c.depth * COL + 16;
      const cy = c.y * ROW + 24;
      const stroke = active.has(c.id) ? color : 'rgba(78,216,255,0.16)';
      paths += `<path d="M${x + 8} ${y} C ${x + COL / 2} ${y}, ${x + COL / 2} ${cy}, ${cx - 8} ${cy}" fill="none" stroke="${stroke}" stroke-width="1"/>`;
    }

    const r = n.depth === 0 ? 9 : n.node.dir ? 6 : 4;
    const fillA = n.node.dir ? 0.85 : 0.55;
    const glow = isActive || hit ? `filter="url(#mm-glow)"` : '';
    dots += `<g class="mm-node" data-id="${n.id}" data-dir="${n.node.dir ? 1 : 0}" transform="translate(${x},${y})" style="cursor:pointer">
      <circle r="${r}" fill="${color}" fill-opacity="${hit ? 1 : fillA}" ${glow}/>
      ${n.node.dir && (n.node.children || []).length ? `<text x="0" y="3" text-anchor="middle" style="font:700 8px monospace;fill:#04222e;pointer-events:none">${collapsed.has(n.id) ? '+' : ''}</text>` : ''}
      <text x="${r + 6}" y="4" style="font:${isActive ? '600' : '400'} 11px 'Segoe UI',sans-serif;fill:${isActive || hit ? '#d6e9f4' : '#6d93ab'};pointer-events:none">${esc(n.node.name.replace(/\.md$/, ''))}</text>
      ${n.node.preview ? `<title>${esc(n.node.preview)}</title>` : ''}
    </g>`;
  }

  svg.innerHTML = `<defs><filter id="mm-glow" x="-80%" y="-80%" width="260%" height="260%">
      <feDropShadow dx="0" dy="0" stdDeviation="3.2" flood-color="#4fd8ff" flood-opacity="0.8"/>
    </filter></defs><g id="mm-world">${paths}${dots}</g>`;
  svg.dataset.w = W;
  svg.dataset.h = H;

  svg.querySelectorAll('.mm-node').forEach((g) => {
    g.addEventListener('click', (e) => {
      e.stopPropagation();
      const id = g.dataset.id;
      if (g.dataset.dir === '1') {
        if (collapsed.has(id)) collapsed.delete(id);
        else if (id !== 'ROOT') collapsed.add(id);
        localStorage.setItem('mm-collapsed', JSON.stringify([...collapsed]));
        draw();
      } else {
        openFile(id);
      }
    });
  });
}

const esc = (s) => String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/"/g, '&quot;');

// ---------- pan / zoom ----------

function wireSvgNav() {
  const svg = document.getElementById('mm-svg');
  let dragging = null;
  svg.addEventListener('mousedown', (e) => { dragging = { x: e.clientX, y: e.clientY, vx: view.x, vy: view.y }; });
  window.addEventListener('mousemove', (e) => {
    if (!dragging) return;
    view.x = dragging.vx + (e.clientX - dragging.x);
    view.y = dragging.vy + (e.clientY - dragging.y);
    draw();
  });
  window.addEventListener('mouseup', () => (dragging = null));
  svg.addEventListener('wheel', (e) => {
    e.preventDefault();
    const k0 = view.k;
    view.k = Math.max(0.4, Math.min(2.5, view.k * (e.deltaY < 0 ? 1.12 : 0.89)));
    // zoom around cursor
    const rect = svg.getBoundingClientRect();
    const mx = e.clientX - rect.left, my = e.clientY - rect.top;
    view.x = mx - ((mx - view.x) / k0) * view.k;
    view.y = my - ((my - view.y) / k0) * view.k;
    draw();
  }, { passive: false });
}

// ---------- side panel + tiny markdown renderer ----------

function panel(show) {
  document.getElementById('mm-panel').hidden = !show;
}

async function openFile(path) {
  selected = path;
  panel(true);
  document.getElementById('mm-panel-title').textContent = path;
  document.getElementById('mm-edit').hidden = true;
  document.getElementById('mm-md').hidden = false;
  try {
    const body = await inv('vault_read_file', { path });
    document.getElementById('mm-md').innerHTML = renderMd(body);
  } catch (e) {
    document.getElementById('mm-md').textContent = String(e);
  }
  draw();
}

function renderMd(src) {
  const lines = esc(src).split('\n');
  let html = '';
  let inList = false, inCode = false;
  for (const line of lines) {
    if (line.startsWith('```')) { inCode = !inCode; html += inCode ? '<pre>' : '</pre>'; continue; }
    if (inCode) { html += `${line}\n`; continue; }
    const inline = (s) => s
      .replace(/\*\*(.+?)\*\*/g, '<b>$1</b>')
      .replace(/\*(.+?)\*/g, '<i>$1</i>')
      .replace(/`(.+?)`/g, '<code>$1</code>');
    if (/^#{1,4} /.test(line)) {
      const level = line.match(/^#+/)[0].length;
      if (inList) { html += '</ul>'; inList = false; }
      html += `<h${level + 1}>${inline(line.replace(/^#+ /, ''))}</h${level + 1}>`;
    } else if (/^[-*] /.test(line.trim())) {
      if (!inList) { html += '<ul>'; inList = true; }
      html += `<li>${inline(line.trim().slice(2))}</li>`;
    } else if (line.trim().startsWith('|')) {
      html += `<div class="md-row">${inline(line.trim())}</div>`;
    } else if (line.trim() === '') {
      if (inList) { html += '</ul>'; inList = false; }
      html += '<div class="md-gap"></div>';
    } else {
      if (inList) { html += '</ul>'; inList = false; }
      html += `<p>${inline(line)}</p>`;
    }
  }
  if (inList) html += '</ul>';
  if (inCode) html += '</pre>';
  return html;
}

// ---------- audit overlay ----------

async function showAudit() {
  const box = document.getElementById('mm-audit');
  box.hidden = false;
  try {
    const st = await inv('vault_status');
    document.getElementById('mm-audit-text').textContent =
      st.last_audit || 'No audit yet — JARVIS runs one every Sunday evening, or run it now.';
  } catch (e) {
    document.getElementById('mm-audit-text').textContent = String(e);
  }
}

// ---------- live updates ----------

listen('vault-changed', async ({ payload }) => {
  if (!mmVisible) return;
  await refresh();
  // Pulse the touched node.
  const g = document.querySelector(`.mm-node[data-id="${CSS.escape(payload.path)}"] circle`);
  if (g) {
    g.style.transition = 'r 200ms cubic-bezier(.25,.8,.4,1)';
    const r0 = g.getAttribute('r');
    g.setAttribute('r', String(Number(r0) * 2));
    setTimeout(() => g.setAttribute('r', r0), 260);
  }
});

listen('vault-audit', () => { if (mmVisible) showAudit(); });

document.addEventListener('tab-shown', ({ detail }) => {
  mmVisible = detail.tab === 'mindmap';
  if (mmVisible) refresh();
});
