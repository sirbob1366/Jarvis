// TAB 4 — Settings. All credentials (stored in Windows Credential Manager via
// the Rust side), voice, briefing window, startup prefs, and a connection-
// status row per integration with the last error visible.

import { inv } from './data.js';
import { prefs } from './app.js';
import { voiceInventory, hasNaturalVoice } from './voice.js';

const root = document.getElementById('settings-root');

// Last-error memory per integration so failures are diagnosable at a glance.
const connErrors = {
  get: (k) => localStorage.getItem(`conn-err:${k}`) || '',
  set: (k, v) => localStorage.setItem(`conn-err:${k}`, v || ''),
};

function secretField(id, label, placeholder, hintId) {
  return `<label>${label}
    <input id="${id}" type="password" placeholder="${placeholder}" autocomplete="off" />
    ${hintId ? `<span class="hint" id="${hintId}"></span>` : ''}
  </label>`;
}

function render() {
  root.innerHTML = `
  <div class="card settings-card" id="sc-status">
    <h3>Connections</h3>
    <div id="conn-rows"></div>
    <button class="btn" id="conn-recheck">Re-check</button>
  </div>

  <div class="card settings-card">
    <h3>Brain</h3>
    <label>Mode</label>
    <div class="seg" id="brain-mode">
      <button data-mode="cli">Claude Code (subscription)</button>
      <button data-mode="api">Anthropic API</button>
    </div>
    <span class="hint" id="brain-state">checking…</span>
    <span class="hint" id="brain-latency"></span>
    ${secretField('set-api-key', 'Anthropic API key (API mode / fallback)', 'sk-ant-…', 'api-key-state')}
    <span class="hint">Stored in Windows Credential Manager — never a file.</span>
  </div>

  <div class="card settings-card">
    <h3>Portfolio analytics</h3>
    ${secretField('set-cf-id', 'CF Access — Client ID', 'xxxx.access')}
    ${secretField('set-cf-secret', 'CF Access — Client Secret', '••••••••')}
  </div>

  <div class="card settings-card">
    <h3>Personal Google</h3>
    ${secretField('set-g-id', 'OAuth Client ID', '…apps.googleusercontent.com')}
    ${secretField('set-g-secret', 'OAuth Client Secret', '••••••••')}
    <button class="btn wide" id="g-connect">Connect Calendar</button>
    <span class="hint" id="g-state"></span>
  </div>

  <div class="card settings-card">
    <h3>Work accounts (read-only)</h3>
    ${secretField('set-slack', 'Slack user token (xoxp-…)', 'scopes: search:read, *history, users:read')}
    <button class="btn wide" id="work-g-connect">Connect Work Google</button>
    <span class="hint" id="work-g-state"></span>
    <span class="hint">Gmail + work calendar are read-only by design — JARVIS can never send or modify anything.</span>
  </div>

  <div class="card settings-card">
    <h3>Voice</h3>
    <label>Voice
      <select id="set-voice"></select>
    </label>
    <label>Rate <span class="hint num" id="rate-label"></span>
      <input id="set-rate" type="range" min="0.7" max="1.5" step="0.05" />
    </label>
    <button class="btn wide" id="voice-test">Test voice</button>
    <div id="natural-hint" hidden>
      <span class="hint">No natural neural voice detected. Windows Settings → Time &amp; Language → Speech → Add voices — install one (e.g. Ryan, en-GB) and JARVIS will use it.</span>
      <button class="btn wide" id="open-voice-settings" style="margin-top:8px">Open Windows voice settings</button>
    </div>
  </div>

  <div class="card settings-card">
    <h3>Briefing</h3>
    <label>Morning window (IST hours)
      <div style="display:flex;gap:8px;margin-top:4px">
        <input id="set-brief-start" type="text" style="width:64px" />
        <span style="align-self:center;color:var(--dim)">to</span>
        <input id="set-brief-end" type="text" style="width:64px" />
      </div>
    </label>
    <label>City (weather) <input id="set-city" type="text" placeholder="Pune" /></label>
  </div>

  <div class="card settings-card" id="sc-agents">
    <h3>Agents</h3>
    <div class="toggle-row"><span>Agents enabled</span><input type="checkbox" id="set-agents-enabled" /></div>
    <span class="hint">Agents edit your allowlisted projects with Claude Code. They commit locally and <b>never deploy</b> — every job waits for your approval on the Agents tab.</span>
    <div class="conn-row" id="agents-cli-row" style="margin-top:8px">
      <span class="conn-dot unset"></span>
      <span class="conn-name">Claude Code</span>
      <span class="conn-err" id="agents-cli-state">checking…</span>
    </div>
    <label style="margin-top:8px">Allowlisted directories</label>
    <div id="agents-allowlist"></div>
    <div class="ag-allow-actions">
      <button class="btn" id="agents-add-row">+ Add directory</button>
      <button class="btn" id="agents-save-allow">Save allowlist</button>
      <span class="hint" id="agents-allow-msg"></span>
    </div>
    ${secretField('set-cf-api', 'Cloudflare API token (deploy status, optional)', 'read-scoped token', 'cf-api-state')}
    <label>Cloudflare account ID (for deploy status)
      <input id="set-cf-account" type="text" placeholder="32-char account id" autocomplete="off" />
    </label>
  </div>

  <div class="card settings-card">
    <h3>JARVIS-OS vault</h3>
    <div class="conn-row" id="vault-row">
      <span class="conn-dot unset"></span>
      <span class="conn-name">Vault</span>
      <span class="conn-err" id="vault-state">checking…</span>
    </div>
    <label>Default domain pin
      <select id="set-domain-pin">
        <option value="all">All</option><option value="work">Work</option>
        <option value="business">Business</option><option value="personal">Personal</option>
      </select>
    </label>
    <div class="toggle-row"><span>Ask before writing to the vault</span><input type="checkbox" id="set-writeback-ask" checked /></div>
    <button class="btn wide" id="vault-init-btn" hidden>Initialize vault</button>
  </div>

  <div class="card settings-card">
    <h3>System</h3>
    <div class="toggle-row"><span>Start with Windows</span><input type="checkbox" id="set-autostart" /></div>
    <div class="toggle-row"><span>Mute voice</span><input type="checkbox" id="set-mute" /></div>
  </div>

  <div class="card settings-card">
    <h3>&nbsp;</h3>
    <button class="btn wide" id="settings-save">Save</button>
    <span class="hint" id="save-state"></span>
  </div>`;

  wire();
  refreshState();
  refreshConnections();
}

// ---------- connection status ----------

const CONNECTIONS = [
  { key: 'brain', name: 'Claude Code CLI', check: checkBrain },
  { key: 'anthropic', name: 'Anthropic API', check: checkAnthropic },
  { key: 'cf', name: 'Analytics Worker', check: checkWorker },
  { key: 'gcal', name: 'Google Calendar', check: checkCalendar },
  { key: 'workg', name: 'Work Google', check: checkWorkGoogle },
  { key: 'slack', name: 'Slack', check: checkSlack },
];

async function checkWorkGoogle() {
  const has = await inv('secret_exists', { key: 'work_google_oauth_token' });
  if (!has) return { state: 'unset', note: 'not connected' };
  try {
    await inv('work_calendar_today');
    return { state: 'ok', note: 'linked (read-only)' };
  } catch (e) {
    return { state: 'fail', note: String(e) };
  }
}

async function checkSlack() {
  const has = await inv('secret_exists', { key: 'slack_token' });
  if (!has) return { state: 'unset', note: 'no token' };
  try {
    await inv('work_slack_overview');
    return { state: 'ok', note: 'reachable' };
  } catch (e) {
    return { state: 'fail', note: String(e) };
  }
}

async function checkBrain() {
  const s = await inv('brain_status');
  if (!s.cli_available) return { state: 'unset', note: 'not installed — API mode will be used' };
  return { state: 'ok', note: `${s.cli_version || 'available'} · brain in ${s.mode} mode` };
}

export async function refreshBrain() {
  const seg = document.getElementById('brain-mode');
  if (!seg) return;
  try {
    const s = await inv('brain_status');
    seg.querySelectorAll('button').forEach((b) => b.classList.toggle('active', b.dataset.mode === s.mode));
    const state = document.getElementById('brain-state');
    state.textContent = s.cli_available
      ? `Claude Code ${s.cli_version} · sandbox: ${s.sandbox}`
      : 'Claude Code CLI not found — conversational turns use the API.';
    const lat = [];
    if (s.cli_first_ms) lat.push(`CLI: first word ${s.cli_first_ms} ms, turn ${s.cli_total_ms} ms`);
    if (s.api_total_ms) lat.push(`API: turn ${s.api_total_ms} ms`);
    document.getElementById('brain-latency').textContent = lat.length ? `Last measured — ${lat.join(' · ')}` : '';
  } catch (e) {
    document.getElementById('brain-state').textContent = String(e);
  }
}

async function checkAnthropic() {
  const has = await inv('secret_exists', { key: 'anthropic_api_key' });
  return has ? { state: 'ok', note: 'key stored' } : { state: 'unset', note: 'no key' };
}

async function checkWorker() {
  const has = await inv('secret_exists', { key: 'cf_access_client_id' });
  if (!has) return { state: 'unset', note: 'no service token' };
  try {
    await inv('worker_api', { path: '/api/sites' });
    return { state: 'ok', note: 'reachable' };
  } catch (e) {
    return { state: 'fail', note: String(e) };
  }
}

async function checkCalendar() {
  const has = await inv('secret_exists', { key: 'google_oauth_token' });
  if (!has) return { state: 'unset', note: 'not connected' };
  try {
    await inv('calendar_today');
    return { state: 'ok', note: 'linked' };
  } catch (e) {
    return { state: 'fail', note: String(e) };
  }
}

export async function refreshConnections() {
  const rows = document.getElementById('conn-rows');
  if (!rows) return;
  rows.innerHTML = CONNECTIONS.map((c) => `
    <div class="conn-row" id="conn-${c.key}">
      <span class="conn-dot unset"></span>
      <span class="conn-name">${c.name}</span>
      <span class="conn-err">checking…</span>
    </div>`).join('');

  await Promise.all(CONNECTIONS.map(async (c) => {
    let res;
    try { res = await c.check(); } catch (e) { res = { state: 'fail', note: String(e) }; }
    connErrors.set(c.key, res.state === 'fail' ? res.note : '');
    const row = document.getElementById(`conn-${c.key}`);
    if (!row) return;
    row.querySelector('.conn-dot').className = `conn-dot ${res.state === 'ok' ? 'ok' : res.state === 'fail' ? 'fail' : 'unset'}`;
    row.querySelector('.conn-err').textContent = res.note;
    row.querySelector('.conn-err').title = res.note;
  }));
}

// ---------- field state + save ----------

async function refreshState() {
  const [hasKey] = await Promise.all([inv('secret_exists', { key: 'anthropic_api_key' })]);
  document.getElementById('api-key-state').textContent = hasKey ? '✓ key stored' : 'no key stored yet';
  document.getElementById('set-city').value = (await inv('setting_get', { key: 'city' })) || 'Pune';
  document.getElementById('set-brief-start').value = (await inv('setting_get', { key: 'briefing_window_start' })) || '6';
  document.getElementById('set-brief-end').value = (await inv('setting_get', { key: 'briefing_window_end' })) || '12';
  document.getElementById('set-rate').value = prefs.rate;
  document.getElementById('rate-label').textContent = `${prefs.rate.toFixed(2)}×`;
  document.getElementById('set-mute').checked = await inv('is_muted');
  try {
    document.getElementById('set-autostart').checked = await inv('plugin:autostart|is_enabled');
  } catch { /* plugin command unavailable */ }
  document.getElementById('set-domain-pin').value = (await inv('setting_get', { key: 'domain_pin' })) || 'all';
  document.getElementById('set-writeback-ask').checked = (await inv('setting_get', { key: 'vault_writeback_ask' })) !== '0';
  refreshVault();
  refreshAgents();
  refreshVoices();
}

async function refreshVault() {
  const row = document.getElementById('vault-row');
  if (!row) return;
  try {
    const st = await inv('vault_status');
    const dot = row.querySelector('.conn-dot');
    const state = document.getElementById('vault-state');
    const initBtn = document.getElementById('vault-init-btn');
    if (!st.exists) {
      dot.className = 'conn-dot unset';
      state.textContent = 'not initialized';
      initBtn.hidden = false;
    } else {
      dot.className = `conn-dot ${st.git_dirty ? 'fail' : 'ok'}`;
      state.textContent = `${st.root} · git ${st.git_dirty ? 'dirty' : 'clean'}${st.last_audit_date ? ` · last audit ${st.last_audit_date}` : ' · no audit yet'}`;
      initBtn.hidden = true;
    }
  } catch (e) {
    document.getElementById('vault-state').textContent = String(e);
  }
}

// ---------- agents ----------

function allowlistRow(entry = { key: '', path: '', cf_project: '' }) {
  const row = document.createElement('div');
  row.className = 'ag-allow-row';
  row.innerHTML = `
    <input class="ag-allow-key" placeholder="key" value="${(entry.key || '').replace(/"/g, '&quot;')}" />
    <input class="ag-allow-path" placeholder="C:\\path\\to\\repo" value="${(entry.path || '').replace(/"/g, '&quot;')}" />
    <input class="ag-allow-cf" placeholder="cf project (opt)" value="${(entry.cf_project || '').replace(/"/g, '&quot;')}" />
    <button class="ag-allow-del" title="Remove">✕</button>`;
  row.querySelector('.ag-allow-del').addEventListener('click', () => row.remove());
  return row;
}

async function refreshAgents() {
  const wrap = document.getElementById('agents-allowlist');
  if (!wrap) return;
  let cfg;
  try { cfg = await inv('agents_config'); } catch (e) {
    document.getElementById('agents-cli-state').textContent = String(e);
    return;
  }
  document.getElementById('set-agents-enabled').checked = !!cfg.enabled;
  const row = document.getElementById('agents-cli-row');
  row.querySelector('.conn-dot').className = `conn-dot ${cfg.cli_available ? 'ok' : 'unset'}`;
  document.getElementById('agents-cli-state').textContent = cfg.cli_available
    ? `${cfg.cli_version || 'installed'} · acceptEdits mode, up to ${cfg.max_concurrency} concurrent`
    : 'not installed — agents cannot run';

  wrap.innerHTML = '';
  for (const e of cfg.allowlist || []) wrap.appendChild(allowlistRow(e));

  document.getElementById('set-cf-account').value =
    (await inv('setting_get', { key: 'cloudflare_account_id' })) || '';
  document.getElementById('cf-api-state').textContent =
    (await inv('secret_exists', { key: 'cloudflare_api_token' })) ? '✓ token stored' : 'no token (deploy status optional)';
}

function refreshVoices() {
  const sel = document.getElementById('set-voice');
  if (!sel) return;
  // Both engines: native WinRT (where Natural voices live) + webview SAPI.
  const opts = voiceInventory.map((v) => {
    const key = `${v.engine}:${v.name}`;
    const flags = `${v.natural ? ' ★ Natural' : ''}${v.engine === 'native' ? '' : ' (legacy)'}`;
    return `<option value="${key.replace(/"/g, '&quot;')}">${v.name}${flags}</option>`;
  });
  sel.innerHTML = '<option value="">Auto (Natural en-GB male preferred)</option>' + opts.join('');
  sel.value = prefs.voiceName;
  const hint = document.getElementById('natural-hint');
  if (hint) hint.hidden = hasNaturalVoice();
}
document.addEventListener('voices-ready', refreshVoices);

function wire() {
  document.getElementById('conn-recheck').addEventListener('click', refreshConnections);

  document.getElementById('brain-mode').addEventListener('click', async (e) => {
    const b = e.target.closest('[data-mode]');
    if (!b) return;
    await inv('brain_set_mode', { mode: b.dataset.mode });
    refreshBrain();
    refreshConnections();
  });

  document.getElementById('set-rate').addEventListener('input', (e) => {
    document.getElementById('rate-label').textContent = `${Number(e.target.value).toFixed(2)}×`;
  });

  document.getElementById('open-voice-settings').addEventListener('click', () => {
    inv('open_voice_settings').catch(() => {});
  });

  document.getElementById('voice-test').addEventListener('click', () => {
    prefs.rate = Number(document.getElementById('set-rate').value);
    prefs.voiceName = document.getElementById('set-voice').value;
    document.dispatchEvent(new CustomEvent('jarvis-said', {
      detail: 'All systems nominal, sir. Shall I proceed?',
    }));
  });

  document.getElementById('set-mute').addEventListener('change', async () => {
    await inv('toggle_mute');
  });

  document.getElementById('vault-init-btn').addEventListener('click', async () => {
    document.getElementById('vault-state').textContent = 'initializing…';
    try {
      document.getElementById('vault-state').textContent = await inv('vault_init');
    } catch (e) {
      document.getElementById('vault-state').textContent = String(e);
    }
    refreshVault();
  });

  document.getElementById('set-domain-pin').addEventListener('change', async (e) => {
    await inv('setting_set', { key: 'domain_pin', value: e.target.value });
  });

  document.getElementById('set-writeback-ask').addEventListener('change', async (e) => {
    await inv('setting_set', { key: 'vault_writeback_ask', value: e.target.checked ? '1' : '0' });
  });

  document.getElementById('set-agents-enabled').addEventListener('change', async (e) => {
    await inv('agents_set_enabled', { enabled: e.target.checked });
  });

  document.getElementById('agents-add-row').addEventListener('click', () => {
    document.getElementById('agents-allowlist').appendChild(allowlistRow());
  });

  document.getElementById('agents-save-allow').addEventListener('click', async () => {
    const rows = [...document.querySelectorAll('#agents-allowlist .ag-allow-row')];
    const list = rows.map((r) => ({
      key: r.querySelector('.ag-allow-key').value.trim(),
      path: r.querySelector('.ag-allow-path').value.trim(),
      cf_project: r.querySelector('.ag-allow-cf').value.trim(),
    })).filter((e) => e.key && e.path);
    const msg = document.getElementById('agents-allow-msg');
    try {
      await inv('agents_set_allowlist', { allowlist: list });
      msg.textContent = `✓ saved ${list.length} ${list.length === 1 ? 'directory' : 'directories'}`;
      setTimeout(() => { msg.textContent = ''; }, 3000);
    } catch (e) { msg.textContent = String(e); }
  });

  document.getElementById('set-autostart').addEventListener('change', async (e) => {
    try {
      await inv(e.target.checked ? 'plugin:autostart|enable' : 'plugin:autostart|disable');
    } catch { /* tray menu still controls it */ }
  });

  document.getElementById('g-connect').addEventListener('click', async () => {
    const state = document.getElementById('g-state');
    await saveSecret('set-g-id', 'google_client_id');
    await saveSecret('set-g-secret', 'google_client_secret');
    state.textContent = 'waiting for Google consent…';
    try {
      state.textContent = await inv('calendar_connect');
    } catch (err) {
      state.textContent = String(err);
    }
    refreshConnections();
  });

  document.getElementById('work-g-connect').addEventListener('click', async () => {
    const state = document.getElementById('work-g-state');
    await saveSecret('set-g-id', 'google_client_id');
    await saveSecret('set-g-secret', 'google_client_secret');
    state.textContent = 'waiting for Google consent (pick the work account)…';
    try {
      state.textContent = await inv('work_google_connect');
    } catch (err) {
      state.textContent = String(err);
    }
    refreshConnections();
  });

  document.getElementById('settings-save').addEventListener('click', async () => {
    await saveSecret('set-api-key', 'anthropic_api_key');
    await saveSecret('set-cf-id', 'cf_access_client_id');
    await saveSecret('set-cf-secret', 'cf_access_client_secret');
    await saveSecret('set-g-id', 'google_client_id');
    await saveSecret('set-g-secret', 'google_client_secret');
    await saveSecret('set-slack', 'slack_token');
    await saveSecret('set-cf-api', 'cloudflare_api_token');
    await inv('setting_set', { key: 'cloudflare_account_id', value: document.getElementById('set-cf-account').value.trim() });
    prefs.rate = Number(document.getElementById('set-rate').value);
    prefs.voiceName = document.getElementById('set-voice').value;
    await inv('setting_set', { key: 'city', value: document.getElementById('set-city').value.trim() || 'Pune' });
    await inv('setting_set', { key: 'briefing_window_start', value: document.getElementById('set-brief-start').value.trim() || '6' });
    await inv('setting_set', { key: 'briefing_window_end', value: document.getElementById('set-brief-end').value.trim() || '12' });
    document.getElementById('save-state').textContent = '✓ saved';
    setTimeout(() => { const el = document.getElementById('save-state'); if (el) el.textContent = ''; }, 2500);
    refreshState();
    refreshConnections();
  });
}

async function saveSecret(inputId, key) {
  const el = document.getElementById(inputId);
  const v = el?.value.trim();
  if (v) {
    await inv('secret_set', { key, value: v });
    el.value = '';
  }
}

render();
refreshBrain();
document.addEventListener('tab-shown', ({ detail }) => {
  if (detail.tab === 'settings') { refreshState(); refreshConnections(); refreshBrain(); refreshAgents(); }
});
