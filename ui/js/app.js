// JARVIS Command Center — shell: tab router, conversation drawer, chat
// stream rendering, mute, hotkey wiring. The board/HUD/work/settings tabs
// each own their module; this file owns navigation and the conversation.

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

// ---------- prefs (non-secret UI prefs; secrets live in the OS vault) ----------

export const prefs = {
  get rate() { return Number(localStorage.getItem('voice-rate') || 1.05); },
  set rate(v) { localStorage.setItem('voice-rate', v); },
  get pitch() { return Number(localStorage.getItem('voice-pitch') || 0.92); },
  set pitch(v) { localStorage.setItem('voice-pitch', v); },
  get voiceName() { return localStorage.getItem('voice-name') || ''; },
  set voiceName(v) { localStorage.setItem('voice-name', v); },
};

// ---------- tab router ----------

const tabs = ['command', 'hud', 'work', 'settings'];
let activeTab = 'command';

export function navigate(tab, detail = null) {
  if (!tabs.includes(tab)) return;
  activeTab = tab;
  for (const t of tabs) {
    document.getElementById(`tab-${t}`)?.classList.toggle('active', t === tab);
  }
  document.querySelectorAll('.rail-btn[data-tab]').forEach((b) =>
    b.classList.toggle('active', b.dataset.tab === tab));
  // Tab modules listen for this to lazily render / drill in.
  document.dispatchEvent(new CustomEvent('tab-shown', { detail: { tab, detail } }));
}

export function currentTab() { return activeTab; }

document.querySelectorAll('.rail-btn[data-tab]').forEach((b) =>
  b.addEventListener('click', () => navigate(b.dataset.tab)));

// Card click-throughs (cards carry data-goto; inner interactive elements stop propagation).
document.addEventListener('click', (e) => {
  const gotoTab = e.target.closest('[data-goto-tab]');
  if (gotoTab) { e.stopPropagation(); navigate(gotoTab.dataset.gotoTab); return; }
  const card = e.target.closest('[data-goto]');
  if (card && !e.target.closest('input,button,a,form')) navigate(card.dataset.goto);
});

// ---------- conversation drawer ----------

const drawer = document.getElementById('drawer');
const scrim = document.getElementById('drawer-scrim');
const chat = document.getElementById('chat');
const input = document.getElementById('input');
const reactor = document.getElementById('reactor');

let drawerOpen = false;

export function openDrawer(focus = true) {
  if (drawerOpen) return;
  drawerOpen = true;
  drawer.hidden = false;
  scrim.hidden = false;
  requestAnimationFrame(() => {
    drawer.classList.add('open');
    scrim.classList.add('open');
  });
  if (focus) setTimeout(() => input.focus(), 120);
}

export function closeDrawer() {
  if (!drawerOpen) return;
  drawerOpen = false;
  drawer.classList.remove('open');
  scrim.classList.remove('open');
  setTimeout(() => { drawer.hidden = true; scrim.hidden = true; }, 220);
}

export function isDrawerOpen() { return drawerOpen; }

scrim.addEventListener('click', closeDrawer);
document.getElementById('drawer-close').addEventListener('click', closeDrawer);
document.getElementById('chat-clear').addEventListener('click', async () => {
  await invoke('clear_session');
  chat.innerHTML = '';
  emptyHint();
});

// ---------- chat rendering ----------

let streamingEl = null;
let hotkeyLabel = '';

function emptyHint() {
  if (!chat.children.length) {
    const voiceHint = hotkeyLabel ? `${hotkeyLabel} to speak · or type below` : 'type below';
    chat.innerHTML = `<div class="hint-empty">SYSTEMS ONLINE<br/>${voiceHint}</div>`;
  }
}

invoke('get_hotkey').then((hk) => {
  hotkeyLabel = hk;
  document.getElementById('strip-input').placeholder = hk ? `Ask JARVIS…  (${hk} to speak)` : 'Ask JARVIS…';
  emptyHint();
});

function addMsg(role, text, cls = '') {
  chat.querySelector('.hint-empty')?.remove();
  const el = document.createElement('div');
  el.className = `msg ${role} ${cls}`;
  el.innerHTML = `<span class="who">${role === 'user' ? 'SIR' : 'J.A.R.V.I.S.'}</span><span class="body"></span>`;
  el.querySelector('.body').textContent = text;
  chat.appendChild(el);
  chat.scrollTop = chat.scrollHeight;
  return el;
}

export function setThinking(on) {
  reactor.classList.toggle('thinking', on);
  document.getElementById('titlebar-status').textContent = on ? 'processing' : '';
}

export async function send(message) {
  message = message.trim();
  if (!message) return;
  openDrawer(false);
  addMsg('user', message);
  setThinking(true);
  streamingEl = null;
  try {
    await invoke('ask_jarvis', { message });
  } catch (err) {
    setThinking(false);
    addMsg('jarvis', String(err), 'error');
  }
}

listen('jarvis-delta', ({ payload }) => {
  if (!streamingEl) { openDrawer(false); streamingEl = addMsg('jarvis', ''); }
  streamingEl.querySelector('.body').textContent += payload.text;
  chat.scrollTop = chat.scrollHeight;
});

listen('jarvis-done', ({ payload }) => {
  setThinking(false);
  toolNote(null);
  if (!streamingEl) { openDrawer(false); streamingEl = addMsg('jarvis', payload.text); }
  else streamingEl.querySelector('.body').textContent = payload.text;
  streamingEl = null;
  document.dispatchEvent(new CustomEvent('jarvis-said', { detail: payload.text }));
});

listen('jarvis-error', ({ payload }) => {
  setThinking(false);
  streamingEl = null;
  toolNote(null);
  openDrawer(false);
  addMsg('jarvis', payload.error, 'error');
});

// ---------- tool activity indicator ----------

const TOOL_LABELS = {
  portfolio_stats: 'consulting portfolio sensors',
  weather: 'checking the skies',
  set_timer: 'arming a timer',
  list_timers: 'reviewing timers',
  system: 'reaching into the system',
  remember: 'committing to memory',
  recall: 'searching my notes',
  calendar: 'consulting the calendar',
  work_todos: 'reviewing the task ledger',
  work_email: 'scanning work mail',
  work_slack: 'scanning Slack',
  work_calendar: 'consulting the work calendar',
  navigate_app: 'bringing it on screen',
  hud_data: 'pulling analytics',
};

let toolEl = null;
function toolNote(name) {
  if (!name) { toolEl?.remove(); toolEl = null; return; }
  if (!toolEl) {
    toolEl = document.createElement('div');
    toolEl.className = 'hint-empty';
    toolEl.style.margin = '0';
    chat.appendChild(toolEl);
  }
  toolEl.textContent = `· ${TOOL_LABELS[name] || name.replace(/_/g, ' ')} ·`;
  chat.scrollTop = chat.scrollHeight;
}
listen('jarvis-tool', ({ payload }) => toolNote(payload.name));

// ---------- proactive messages land in the drawer ----------

listen('timer-fired', ({ payload }) => {
  openDrawer(false);
  addMsg('jarvis', `⏱ ${payload.label}`);
  document.dispatchEvent(new CustomEvent('jarvis-said', { detail: payload.label }));
});

listen('anomaly-alert', ({ payload }) => {
  openDrawer(false);
  addMsg('jarvis', `⚠ ${payload.text}`);
  document.dispatchEvent(new CustomEvent('jarvis-said', { detail: payload.text }));
  document.dispatchEvent(new CustomEvent('board-refresh')); // anomaly chips
});

listen('todos-changed', () => document.dispatchEvent(new CustomEvent('board-refresh')));

// navigate_app tool — the model switches what the app is showing.
listen('navigate', ({ payload }) => {
  navigate(payload.tab, { view: payload.view || null, site: payload.site || null });
});

// ---------- composers ----------

document.getElementById('composer').addEventListener('submit', (e) => {
  e.preventDefault();
  const text = input.value;
  input.value = '';
  send(text);
});

const stripInput = document.getElementById('strip-input');
stripInput.closest('.greet-ask').addEventListener('submit', (e) => e.preventDefault());
stripInput.addEventListener('keydown', (e) => {
  if (e.key === 'Enter' && stripInput.value.trim()) {
    send(stripInput.value);
    stripInput.value = '';
  }
});

document.getElementById('mic-btn').addEventListener('click', () => {
  openDrawer(false);
  document.dispatchEvent(new CustomEvent('ptt-start'));
});

document.getElementById('hide-btn').addEventListener('click', () => invoke('hide_window'));

document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape') {
    if (drawerOpen) closeDrawer();
    else invoke('hide_window');
  }
});

// ---------- mute ----------

const muteBtn = document.getElementById('mute-btn');
function renderMute(muted) {
  muteBtn.classList.toggle('muted', muted);
  muteBtn.querySelector('.snd-mute').style.display = muted ? '' : 'none';
  muteBtn.querySelectorAll('.snd-wave').forEach((w) => (w.style.display = muted ? 'none' : ''));
}
muteBtn.addEventListener('click', async () => renderMute(await invoke('toggle_mute')));
listen('mute-changed', ({ payload }) => renderMute(payload));
invoke('is_muted').then(renderMute);

// ---------- global hotkey: summon + push-to-talk ----------

listen('hotkey-summon', () => {
  openDrawer(false);
  document.dispatchEvent(new CustomEvent('ptt-start'));
});
listen('hotkey-released', () => {
  document.dispatchEvent(new CustomEvent('ptt-stop'));
});

emptyHint();
