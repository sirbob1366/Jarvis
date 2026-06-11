// TAB 3 — Work: Today (work calendar + gaps) / Inbox (Gmail, read-only,
// deep links) / Slack (mentions + DMs, permalinks) / unified To-Do front and
// center with confirm-complete-snooze. Also fills the Command board's
// inbox/slack strip. Refreshes every 5 min while visible.

import { inv, cached, store, agoLabel } from './data.js';
import { send } from './app.js';

const root = document.getElementById('work-root');
let workVisible = false;
let timer = null;

function openLink(url) {
  if (!url) return;
  const opener = window.__TAURI__?.opener;
  if (opener?.openUrl) opener.openUrl(url);
  else inv('plugin:opener|open_url', { url });
}

function skeleton() {
  root.innerHTML = `
    <div class="work-head">
      <button class="btn" id="work-catchup">Catch me up</button>
      <button class="btn" id="work-scan">Scan for action items</button>
      <span class="card-label dimmer" id="work-stale"></span>
    </div>
    <div class="work-grid">
      <div class="card hud-panel" id="wk-today">
        <div class="card-head"><span class="card-label">Today</span><span class="card-label dimmer" id="wk-gaps-note"></span></div>
        <div id="wk-next" class="next-meeting" hidden>
          <div class="nm-title" id="wk-next-title"></div>
          <div class="nm-when"><span class="num" id="wk-next-time"></span><span class="nm-in" id="wk-next-in"></span></div>
        </div>
        <ul class="event-list" id="wk-events"></ul>
        <div class="card-empty" id="wk-today-empty" hidden>no meetings today, sir</div>
        <div class="conn-nudge" id="wk-today-nudge" hidden></div>
      </div>

      <div class="card hud-panel" id="wk-todo">
        <div class="card-head"><span class="card-label">To-Do</span><span class="count-badge num" id="wk-todo-count" hidden></span></div>
        <ul class="work-todo-list" id="wk-todo-list"></ul>
        <div class="card-empty" id="wk-todo-empty" hidden>all clear, sir</div>
        <form class="todo-add" id="wk-todo-form"><input id="wk-todo-input" type="text" placeholder="+ add" autocomplete="off" /></form>
      </div>

      <div class="card hud-panel" id="wk-inbox">
        <div class="card-head"><span class="card-label">Inbox</span><span class="num" id="wk-unread"></span></div>
        <ul class="mail-list" id="wk-mail"></ul>
        <div class="card-empty" id="wk-mail-empty" hidden>inbox is quiet</div>
        <div class="conn-nudge" id="wk-inbox-nudge" hidden></div>
      </div>

      <div class="card hud-panel" id="wk-slack">
        <div class="card-head"><span class="card-label">Slack</span><span class="num" id="wk-slack-n"></span></div>
        <div class="card-label dimmer" style="margin:8px 0 4px">Mentions</div>
        <ul class="slack-list" id="wk-mentions"></ul>
        <div class="card-label dimmer" style="margin:8px 0 4px">Unread DMs</div>
        <ul class="slack-list" id="wk-dms"></ul>
        <div class="conn-nudge" id="wk-slack-nudge" hidden></div>
      </div>
    </div>`;

  document.getElementById('work-catchup').addEventListener('click', () =>
    send('Catch me up on work: the last 4 hours of email, Slack mentions, and what is next on the work calendar — in under 20 spoken seconds.'));
  document.getElementById('work-scan').addEventListener('click', async () => {
    const note = document.getElementById('work-stale');
    note.textContent = 'scanning…';
    try {
      note.textContent = await inv('work_scan');
      refreshTodos();
    } catch (e) {
      note.textContent = String(e);
    }
  });
  document.getElementById('wk-todo-form').addEventListener('submit', async (e) => {
    e.preventDefault();
    const inp = document.getElementById('wk-todo-input');
    if (!inp.value.trim()) return;
    await inv('todo_add', { text: inp.value.trim() });
    inp.value = '';
    refreshTodos();
  });
}

// ---------- today (work calendar) ----------

function fmtTime(iso) {
  if (!iso) return '';
  if (!iso.includes('T')) return 'all day';
  return new Date(iso).toLocaleTimeString('en-GB', { hour: '2-digit', minute: '2-digit' });
}

function paintToday(data) {
  document.getElementById('wk-today-nudge').hidden = true;
  const events = (data.events || []).filter((e) => e.start);
  const now = Date.now();
  const next = events.find((e) => e.start.includes('T') && Date.parse(e.start) > now);

  const nm = document.getElementById('wk-next');
  if (next) {
    nm.hidden = false;
    document.getElementById('wk-next-title').textContent = next.title || '(untitled)';
    document.getElementById('wk-next-time').textContent = fmtTime(next.start);
    const mins = Math.round((Date.parse(next.start) - now) / 60000);
    document.getElementById('wk-next-in').textContent = mins < 60 ? `in ${mins} min` : `in ${Math.floor(mins / 60)} h ${mins % 60} min`;
  } else nm.hidden = true;

  const ul = document.getElementById('wk-events');
  ul.innerHTML = '';
  for (const e of events.filter((x) => x !== next)) {
    const li = document.createElement('li');
    if (e.end && Date.parse(e.end) < now) li.className = 'past';
    li.innerHTML = `<span class="ev-time num"></span><span class="ev-title"></span><span class="badge-w">W</span>`;
    li.querySelector('.ev-time').textContent = fmtTime(e.start);
    li.querySelector('.ev-title').textContent = e.title || '(untitled)';
    ul.appendChild(li);
  }
  document.getElementById('wk-today-empty').hidden = events.length > 0;

  const gaps = data.gaps || [];
  document.getElementById('wk-gaps-note').textContent = gaps.length
    ? `free: ${gaps.map((g) => `${g.from}–${g.to}`).join(' · ')}` : '';
}

function nudge(id, msg) {
  const el = document.getElementById(id);
  if (!el) return;
  el.hidden = false;
  el.innerHTML = '';
  const span = document.createElement('span');
  span.textContent = msg;
  const btn = document.createElement('button');
  btn.className = 'link-btn';
  btn.textContent = 'Settings →';
  btn.dataset.gotoTab = 'settings';
  el.append(span, btn);
}

// ---------- inbox ----------

function paintInbox(data) {
  document.getElementById('wk-inbox-nudge').hidden = true;
  document.getElementById('wk-unread').textContent = data.unread ? `${data.unread} unread` : '';
  const ul = document.getElementById('wk-mail');
  ul.innerHTML = '';
  const msgs = data.messages || [];
  document.getElementById('wk-mail-empty').hidden = msgs.length > 0;
  for (const m of msgs) {
    const li = document.createElement('li');
    li.className = m.unread ? 'unread' : '';
    li.innerHTML = `<span class="ml-from"></span><span class="ml-subject"></span><span class="ml-snippet"></span>`;
    li.querySelector('.ml-from').textContent = (m.from || '').replace(/<.*>/, '').trim();
    li.querySelector('.ml-subject').textContent = m.subject || '(no subject)';
    li.querySelector('.ml-snippet').textContent = m.snippet || '';
    li.title = 'Open in Gmail';
    li.addEventListener('click', () => openLink(m.link));
    ul.appendChild(li);
  }
  // Board strip.
  const strip = document.getElementById('inbox-gmail');
  if (strip) {
    strip.innerHTML = `<span class="card-label">Gmail</span><span class="num">${data.unread ?? 0}</span> unread`;
    const btn = document.createElement('button');
    btn.className = 'link-btn';
    btn.textContent = 'open ↗';
    btn.addEventListener('click', (e) => { e.stopPropagation(); openLink('https://mail.google.com'); });
    strip.appendChild(btn);
  }
}

// ---------- slack ----------

function paintSlack(data) {
  document.getElementById('wk-slack-nudge').hidden = true;
  const mentions = data.mentions || [];
  const dms = data.dms || [];
  document.getElementById('wk-slack-n').textContent =
    `${mentions.length} mention${mentions.length === 1 ? '' : 's'}`;

  const mUl = document.getElementById('wk-mentions');
  mUl.innerHTML = mentions.length ? '' : '<li class="card-label dimmer">none in the last day</li>';
  for (const m of mentions) {
    const li = document.createElement('li');
    li.innerHTML = `<span class="sl-from"></span><span class="sl-text"></span>`;
    li.querySelector('.sl-from').textContent = `${m.from || '?'} · #${m.channel || 'dm'}`;
    li.querySelector('.sl-text').textContent = m.text || '';
    li.title = 'Open in Slack';
    li.addEventListener('click', () => openLink(m.permalink));
    mUl.appendChild(li);
  }

  const dUl = document.getElementById('wk-dms');
  dUl.innerHTML = dms.length ? '' : '<li class="card-label dimmer">no unread DMs</li>';
  for (const d of dms) {
    const li = document.createElement('li');
    li.innerHTML = `<span class="sl-from"></span><span class="sl-text"></span>`;
    li.querySelector('.sl-from').textContent = d.with;
    li.querySelector('.sl-text').textContent = `${d.unread} unread`;
    dUl.appendChild(li);
  }

  const strip = document.getElementById('inbox-slack');
  if (strip) {
    strip.innerHTML = `<span class="card-label">Slack</span><span class="num">${mentions.length}</span> mentions · <span class="num">${dms.length}</span> unread DMs`;
  }
}

// ---------- todos ----------

async function refreshTodos() {
  const data = await inv('todo_list');
  const list = document.getElementById('wk-todo-list');
  if (!list) return;
  const items = data.items || [];
  list.innerHTML = '';
  for (const t of items) {
    const li = document.createElement('li');
    const check = document.createElement('input');
    check.type = 'checkbox';
    check.className = 'todo-check';
    check.addEventListener('change', async () => {
      li.classList.add('done');
      await inv('todo_complete', { id: t.id });
      setTimeout(refreshTodos, 350);
    });
    const text = document.createElement('span');
    text.className = 'todo-text';
    text.textContent = t.text;
    text.title = t.text;
    if (t.link) {
      text.style.cursor = 'pointer';
      text.addEventListener('click', () => openLink(t.link));
    }
    const src = document.createElement('span');
    src.className = 'tag-suggested';
    src.textContent = t.source;
    li.append(check, text, src);
    if (t.status === 'suggested') {
      const ok = document.createElement('button');
      ok.className = 'todo-mini'; ok.textContent = '✓'; ok.title = 'Confirm';
      ok.addEventListener('click', async () => { await inv('todo_confirm', { id: t.id }); refreshTodos(); });
      const zz = document.createElement('button');
      zz.className = 'todo-mini'; zz.textContent = '⏰'; zz.title = 'Snooze 24h';
      zz.addEventListener('click', async () => { await inv('todo_snooze', { id: t.id, untilTs: Date.now() + 86400_000 }); refreshTodos(); });
      const no = document.createElement('button');
      no.className = 'todo-mini'; no.textContent = '✕'; no.title = 'Dismiss';
      no.addEventListener('click', async () => { await inv('todo_dismiss', { id: t.id }); refreshTodos(); });
      li.append(ok, zz, no);
    } else {
      const zz = document.createElement('button');
      zz.className = 'todo-mini'; zz.textContent = '⏰'; zz.title = 'Snooze 24h';
      zz.addEventListener('click', async () => { await inv('todo_snooze', { id: t.id, untilTs: Date.now() + 86400_000 }); refreshTodos(); });
      li.append(zz);
    }
    list.appendChild(li);
  }
  const badge = document.getElementById('wk-todo-count');
  badge.hidden = !data.open_count;
  badge.textContent = data.open_count;
  document.getElementById('wk-todo-empty').hidden = items.length > 0;
}

// ---------- refresh orchestration ----------

async function refresh() {
  if (!root.querySelector('.work-grid')) skeleton();
  const stale = document.getElementById('work-stale');

  const jobs = [
    ['work_calendar_today', 'wk-cal', paintToday, 'wk-today-nudge', 'work calendar not connected'],
    ['work_email_overview', 'wk-mail', paintInbox, 'wk-inbox-nudge', 'work Gmail not connected'],
    ['work_slack_overview', 'wk-slack', paintSlack, 'wk-slack-nudge', 'Slack token not configured'],
  ];
  await Promise.all(jobs.map(async ([cmd, key, paint, nudgeId, nudgeMsg]) => {
    try {
      const data = await inv(cmd);
      store(key, data);
      paint(data);
    } catch (err) {
      const c = cached(key);
      if (c) { paint(c.data); stale.textContent = `cached ${agoLabel(c.ts)}`; }
      else nudge(nudgeId, String(err).includes('not co') || String(err).includes('Settings') ? nudgeMsg : String(err));
    }
  }));
  refreshTodos();
}

document.addEventListener('tab-shown', ({ detail }) => {
  workVisible = detail.tab === 'work';
  clearInterval(timer);
  if (workVisible) {
    refresh();
    timer = setInterval(() => { if (!document.hidden) refresh(); }, 5 * 60 * 1000);
  }
});

// The Command board's strip asks for a fill when work creds exist.
document.addEventListener('work-strip-refresh', async () => {
  if (!root.querySelector('.work-grid')) skeleton(); // paint targets must exist
  try { paintInbox(await inv('work_email_overview')); } catch { /* strip stays nudge */ }
  try { paintSlack(await inv('work_slack_overview')); } catch { /* partial is fine */ }
});

document.addEventListener('todos-changed-ui', refreshTodos);
