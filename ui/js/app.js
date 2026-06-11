// JARVIS Desktop UI — conversation view + settings.
// Tauri global API (withGlobalTauri): window.__TAURI__.

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const chat = document.getElementById('chat');
const input = document.getElementById('input');
const reactor = document.getElementById('reactor');
const muteBtn = document.getElementById('mute-btn');

let streamingEl = null; // the in-flight jarvis bubble

// ---------- prefs (non-secret UI prefs only; secrets live in the OS vault) ----------

export const prefs = {
  get rate() { return Number(localStorage.getItem('voice-rate') || 1.02); },
  set rate(v) { localStorage.setItem('voice-rate', v); },
  get pitch() { return Number(localStorage.getItem('voice-pitch') || 0.85); },
  set pitch(v) { localStorage.setItem('voice-pitch', v); },
  get city() { return localStorage.getItem('city') || 'Pune'; },
  set city(v) { localStorage.setItem('city', v); },
};

// ---------- chat rendering ----------

function emptyHint() {
  if (!chat.children.length) {
    chat.innerHTML = `<div class="hint-empty">SYSTEMS ONLINE<br/><br/>Ctrl+Shift+J to speak · or type below</div>`;
  }
}

function addMsg(role, text, cls = '') {
  const hint = chat.querySelector('.hint-empty');
  if (hint) hint.remove();
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
}

export async function send(message) {
  message = message.trim();
  if (!message) return;
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
  if (!streamingEl) streamingEl = addMsg('jarvis', '');
  streamingEl.querySelector('.body').textContent += payload.text;
  chat.scrollTop = chat.scrollHeight;
});

listen('jarvis-done', async ({ payload }) => {
  setThinking(false);
  toolNote(null);
  if (!streamingEl) streamingEl = addMsg('jarvis', payload.text);
  else streamingEl.querySelector('.body').textContent = payload.text;
  streamingEl = null;
  // Stage 2: speak the reply (voice.js hooks this event too).
  document.dispatchEvent(new CustomEvent('jarvis-said', { detail: payload.text }));
});

listen('jarvis-error', ({ payload }) => {
  setThinking(false);
  streamingEl = null;
  toolNote(null);
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
};

let toolEl = null;
function toolNote(name) {
  if (!name) {
    toolEl?.remove();
    toolEl = null;
    return;
  }
  if (!toolEl) {
    toolEl = document.createElement('div');
    toolEl.className = 'hint-empty';
    toolEl.style.margin = '0';
    chat.appendChild(toolEl);
  }
  toolEl.textContent = `· ${TOOL_LABELS[name] || name} ·`;
  chat.scrollTop = chat.scrollHeight;
}

listen('jarvis-tool', ({ payload }) => toolNote(payload.name));

// ---------- timers ----------

listen('timer-fired', ({ payload }) => {
  addMsg('jarvis', `⏱ ${payload.label}`);
  document.dispatchEvent(new CustomEvent('jarvis-said', { detail: payload.label }));
});

// ---------- proactive anomaly alerts ----------

listen('anomaly-alert', ({ payload }) => {
  addMsg('jarvis', `⚠ ${payload.text}`);
  document.dispatchEvent(new CustomEvent('jarvis-said', { detail: payload.text }));
});

// ---------- composer ----------

document.getElementById('composer').addEventListener('submit', (e) => {
  e.preventDefault();
  const text = input.value;
  input.value = '';
  send(text);
});

document.getElementById('hide-btn').addEventListener('click', () => invoke('hide_window'));

document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape') invoke('hide_window');
});

// ---------- mute ----------

async function renderMute(muted) {
  muteBtn.textContent = muted ? '🔇' : '🔊';
  muteBtn.classList.toggle('muted', muted);
}
muteBtn.addEventListener('click', async () => renderMute(await invoke('toggle_mute')));
listen('mute-changed', ({ payload }) => renderMute(payload));
invoke('is_muted').then(renderMute);

// ---------- hotkey summon ----------

listen('hotkey-summon', () => {
  input.focus();
  // Stage 2: voice.js starts push-to-talk listening on this event.
  document.dispatchEvent(new CustomEvent('ptt-start'));
});
listen('hotkey-released', () => {
  document.dispatchEvent(new CustomEvent('ptt-stop'));
});

// ---------- settings ----------

const dlg = document.getElementById('settings');
const apiKeyInput = document.getElementById('set-api-key');
const apiKeyState = document.getElementById('api-key-state');

document.getElementById('settings-btn').addEventListener('click', async () => {
  const [hasKey, hasCf] = await Promise.all([
    invoke('secret_exists', { key: 'anthropic_api_key' }),
    invoke('secret_exists', { key: 'cf_access_client_id' }),
  ]);
  apiKeyState.textContent = hasKey ? '✓ key stored in Credential Manager' : 'no key stored yet';
  document.getElementById('cf-state').textContent = hasCf ? '✓ service token stored' : 'not configured (portfolio tool offline)';
  apiKeyInput.value = '';
  document.getElementById('set-cf-id').value = '';
  document.getElementById('set-cf-secret').value = '';
  document.getElementById('set-rate').value = prefs.rate;
  document.getElementById('set-pitch').value = prefs.pitch;
  document.getElementById('set-city').value = (await invoke('setting_get', { key: 'city' })) || 'Pune';
  dlg.showModal();
});

document.getElementById('settings-save').addEventListener('click', async (e) => {
  e.preventDefault();
  const key = apiKeyInput.value.trim();
  if (key) await invoke('secret_set', { key: 'anthropic_api_key', value: key });
  const cfId = document.getElementById('set-cf-id').value.trim();
  const cfSecret = document.getElementById('set-cf-secret').value.trim();
  if (cfId) await invoke('secret_set', { key: 'cf_access_client_id', value: cfId });
  if (cfSecret) await invoke('secret_set', { key: 'cf_access_client_secret', value: cfSecret });
  prefs.rate = Number(document.getElementById('set-rate').value);
  prefs.pitch = Number(document.getElementById('set-pitch').value);
  await invoke('setting_set', { key: 'city', value: document.getElementById('set-city').value.trim() || 'Pune' });
  dlg.close();
});

emptyHint();
input.focus();
