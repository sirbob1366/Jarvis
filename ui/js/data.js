// Data plumbing — cache-first paint + visibility-aware polling.
// Every card paints instantly from localStorage on summon, then refreshes
// live. All pollers pause while the window is hidden (document.hidden flips
// when the Tauri window hides) and fire immediately on re-show if stale.

const { invoke } = window.__TAURI__.core;

export const inv = invoke;

// ---------- cache ----------

export function cached(key) {
  try {
    const raw = localStorage.getItem(`cache:${key}`);
    return raw ? JSON.parse(raw) : null; // { ts, data }
  } catch {
    return null;
  }
}

export function store(key, data) {
  try {
    localStorage.setItem(`cache:${key}`, JSON.stringify({ ts: Date.now(), data }));
  } catch { /* storage full — refresh still painted live */ }
}

export function agoLabel(ts) {
  const s = Math.max(0, Math.round((Date.now() - ts) / 1000));
  if (s < 60) return 'just now';
  if (s < 3600) return `${Math.round(s / 60)} min ago`;
  if (s < 86400) return `${Math.round(s / 3600)} h ago`;
  return `${Math.round(s / 86400)} d ago`;
}

// ---------- pollers ----------

const pollers = [];

export function poll(name, ms, fn) {
  const p = { name, ms, fn, last: 0, timer: null, running: false };
  pollers.push(p);

  const tick = async () => {
    if (document.hidden) return; // paused while minimized/hidden
    if (p.running) return;
    p.running = true;
    try { await fn(); p.last = Date.now(); } catch { /* card shows degraded state */ }
    p.running = false;
  };

  p.timer = setInterval(tick, ms);
  tick();
  p.kick = tick;
  return p;
}

document.addEventListener('visibilitychange', () => {
  if (document.hidden) return;
  // Window just re-shown: anything stale refreshes immediately.
  for (const p of pollers) {
    if (Date.now() - p.last > p.ms) p.kick?.();
  }
});

/** Refresh everything now (morning briefing sync). */
export function kickAll() {
  for (const p of pollers) p.kick?.();
}

// ---------- small DOM/format helpers ----------

export function fmt(n) {
  if (n == null || Number.isNaN(n)) return '—';
  return Number(n).toLocaleString('en-IN');
}

export function fmtCompact(n) {
  if (n == null) return '—';
  if (n >= 100000) return `${(n / 1000).toFixed(0)}k`;
  if (n >= 10000) return `${(n / 1000).toFixed(1)}k`;
  return fmt(n);
}

/** Tick a numeric element from its current value to `to` (~350ms). */
export function tickNumber(el, to, format = fmt) {
  const from = Number(String(el.dataset.v ?? '0')) || 0;
  el.dataset.v = to;
  if (from === to || document.hidden) { el.textContent = format(to); return; }
  const t0 = performance.now();
  const dur = 350;
  const step = (t) => {
    const k = Math.min(1, (t - t0) / dur);
    const eased = 1 - (1 - k) * (1 - k);
    el.textContent = format(Math.round(from + (to - from) * eased));
    if (k < 1) requestAnimationFrame(step);
  };
  requestAnimationFrame(step);
}

export function deltaEl(el, pct) {
  if (pct == null) { el.textContent = ''; el.className = 'delta flat'; return; }
  const up = pct >= 0;
  el.textContent = `${up ? '▲' : '▼'} ${Math.abs(pct).toFixed(1)}%`;
  el.className = `delta ${up ? 'up' : 'down'}`;
}
