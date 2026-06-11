// Voice I/O — the natural-voice upgrade.
//
// TTS, two engines:
//   native — Rust WinRT Windows.Media.SpeechSynthesis: sees every voice pack
//            installed via Windows Settings → Speech, including the modern
//            "(Natural)" neural voices on Windows 11. Synthesis returns a WAV
//            (base64) played here, so mute/interrupt stays in one place.
//   web    — WebView2 speechSynthesis: legacy SAPI voices only (the Edge
//            "Online (Natural)" voices are browser-exclusive and never appear
//            in a WebView). Kept as fallback and for zero-latency starts.
// Preference: "Ryan (Natural) en-GB" → any en-GB Natural male → any Natural →
// legacy en-GB male → en — across BOTH engines (native checked first, since
// that's where Natural voices actually live).
//
// Delivery polish: URLs/long ids are never read aloud ("link on screen"),
// big numbers are rounded in speech (the screen has the exact figure), a
// slight pause follows "sir", and JARVIS never speaks over push-to-talk.
//
// STT: push-to-talk via the Rust WinRT recognizer (WebView2 lacks Web Speech
// recognition); auto-stops on silence, hotkey-release stops early.

import { prefs, send, setThinking } from './app.js';

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const wave = document.getElementById('wave');
const canvas = document.getElementById('wave-canvas');
const ctx2d = canvas.getContext('2d');
const micBtn = document.getElementById('mic-btn');

// ---------- voice inventory (both engines) ----------

export let voiceInventory = []; // { engine: 'native'|'web', name, lang, natural, male }

async function buildInventory() {
  const natRe = /natural/i;
  const maleRe = /ryan|guy|thomas|george|daniel|male|mark|james/i;
  const inv = [];
  try {
    for (const v of await invoke('tts_voices')) {
      inv.push({
        engine: 'native', name: v.name, lang: v.lang || '',
        natural: natRe.test(v.name),
        male: v.gender === 'male' || maleRe.test(v.name),
      });
    }
  } catch { /* WinRT unavailable — web only */ }
  for (const v of speechSynthesis.getVoices()) {
    inv.push({
      engine: 'web', name: v.name, lang: v.lang || '',
      natural: natRe.test(v.name),
      male: maleRe.test(v.name),
    });
  }
  voiceInventory = inv;
  document.dispatchEvent(new CustomEvent('voices-ready'));
  return inv;
}
speechSynthesis.addEventListener?.('voiceschanged', buildInventory);
buildInventory();

export function pickPreferred() {
  const inv = voiceInventory;
  const gb = (v) => /en-GB/i.test(v.lang);
  const en = (v) => /^en/i.test(v.lang);
  // Explicit user choice first.
  if (prefs.voiceName) {
    const [engine, ...rest] = prefs.voiceName.split(':');
    const name = rest.join(':');
    const v = inv.find((x) => x.engine === engine && x.name === name);
    if (v) return v;
  }
  return (
    inv.find((v) => gb(v) && v.natural && /ryan/i.test(v.name)) ||
    inv.find((v) => gb(v) && v.natural && v.male) ||
    inv.find((v) => gb(v) && v.natural) ||
    inv.find((v) => v.natural && en(v)) ||
    inv.find((v) => v.natural) ||
    inv.find((v) => gb(v) && v.male) ||
    inv.find((v) => gb(v)) ||
    inv.find((v) => en(v)) ||
    inv[0] || null
  );
}

export function hasNaturalVoice() {
  return voiceInventory.some((v) => v.natural);
}

// ---------- delivery polish ----------

function roundSpoken(nStr) {
  const n = Number(nStr.replace(/,/g, ''));
  if (!Number.isFinite(n) || n < 10_000) return nStr;
  if (n >= 1_000_000) return `about ${(n / 1_000_000).toFixed(1).replace(/\.0$/, '')} million`;
  if (n >= 100_000) return `about ${Math.round(n / 1000)} thousand`;
  return `about ${(n / 1000).toFixed(1).replace(/\.0$/, '')} thousand`;
}

export function polishForSpeech(text) {
  return text
    .replace(/https?:\/\/\S+|www\.\S+/gi, 'link on screen')
    .replace(/\b[a-f0-9]{12,}\b/gi, 'the id on screen')           // hex ids / hashes
    .replace(/\b[A-Za-z0-9_-]{20,}\b/g, 'the id on screen')        // tokens / message ids
    .replace(/\b\d{1,3}(?:,\d{3})+\b|\b\d{5,}\b/g, roundSpoken)    // big numbers
    .replace(/[#*_`>|]+/g, ' ')                                     // markdown debris
    .replace(/[▲▼◷●▸]/g, '')
    .replace(/\s{2,}/g, ' ')
    .trim();
}

const xmlEscape = (s) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');

/** SSML with a slight pause after "sir" (native engine only). */
function toSsml(text, lang) {
  const withBreaks = xmlEscape(text)
    .replace(/\bsir([,.!?;])/gi, 'sir$1<break time="240ms"/>');
  return `<speak version="1.0" xmlns="http://www.w3.org/2001/10/synthesis" xml:lang="${lang || 'en-GB'}">${withBreaks}</speak>`;
}

// ---------- speaking ----------

let currentAudio = null;
let speakSeq = 0;

export function stopSpeaking() {
  speakSeq++;
  speechSynthesis.cancel();
  if (currentAudio) {
    currentAudio.pause();
    currentAudio = null;
  }
}

export async function speak(text) {
  if (!text) return;
  if (listening) return; // never speak over sir mid-push-to-talk
  if (await invoke('is_muted')) return;
  stopSpeaking();
  const seq = speakSeq;

  const polished = polishForSpeech(text);
  if (!polished) return;
  const v = pickPreferred();

  if (v?.engine === 'native') {
    try {
      const b64 = await invoke('tts_synthesize', {
        text: toSsml(polished, v.lang),
        voice: v.name,
        rate: prefs.rate,
        ssml: true,
      });
      if (seq !== speakSeq || listening) return; // superseded while synthesizing
      currentAudio = new Audio(`data:audio/wav;base64,${b64}`);
      currentAudio.play().catch(() => {});
      return;
    } catch { /* fall through to web engine */ }
  }

  if ('speechSynthesis' in window) {
    const u = new SpeechSynthesisUtterance(polished);
    const wv = speechSynthesis.getVoices().find((x) => x.name === v?.name) || null;
    if (wv) u.voice = wv;
    u.rate = prefs.rate;
    u.pitch = prefs.pitch;
    speechSynthesis.speak(u);
  }
}

document.addEventListener('jarvis-said', (e) => speak(e.detail));
listen('mute-changed', ({ payload }) => {
  if (payload) stopSpeaking();
});

// ---------- waveform (live element — the only place it animates) ----------

let audioCtx = null;
let analyser = null;
let micStream = null;
let rafId = null;

function sizeCanvas() {
  // Device-pixel-ratio-aware: crisp bars on any display.
  const dpr = window.devicePixelRatio || 1;
  const w = canvas.clientWidth || 480;
  canvas.width = Math.round(w * dpr);
  canvas.height = Math.round(36 * dpr);
}

async function startWave() {
  wave.hidden = false;
  sizeCanvas();
  try {
    micStream = await navigator.mediaDevices.getUserMedia({ audio: true });
    audioCtx = new AudioContext();
    analyser = audioCtx.createAnalyser();
    analyser.fftSize = 256;
    audioCtx.createMediaStreamSource(micStream).connect(analyser);
  } catch {
    analyser = null; // animated fallback
  }
  const data = analyser ? new Uint8Array(analyser.frequencyBinCount) : null;
  const draw = () => {
    rafId = requestAnimationFrame(draw);
    ctx2d.clearRect(0, 0, canvas.width, canvas.height);
    ctx2d.fillStyle = 'rgba(79,216,255,0.85)';
    const bars = 64;
    const bw = canvas.width / bars;
    for (let i = 0; i < bars; i++) {
      let v;
      if (analyser) {
        analyser.getByteFrequencyData(data);
        v = data[Math.floor((i / bars) * data.length)] / 255;
      } else {
        v = 0.2 + 0.8 * Math.abs(Math.sin(Date.now() / 130 + i * 0.7)) * Math.random();
      }
      const h = Math.max(2, v * canvas.height * 0.9);
      ctx2d.fillRect(i * bw + 1, (canvas.height - h) / 2, bw - 2, h);
    }
  };
  draw();
}

function stopWave() {
  wave.hidden = true;
  cancelAnimationFrame(rafId);
  micStream?.getTracks().forEach((t) => t.stop());
  audioCtx?.close();
  micStream = audioCtx = analyser = null;
}

// ---------- push-to-talk ----------

const webSpeech = window.SpeechRecognition || window.webkitSpeechRecognition || null;
let listening = false;

async function startListening() {
  if (listening) return;
  listening = true;
  micBtn.classList.add('listening');
  stopSpeaking(); // don't transcribe our own voice

  if (webSpeech) {
    const rec = new webSpeech();
    rec.lang = 'en-GB';
    rec.interimResults = false;
    rec.onresult = (e) => gotTranscript(e.results[0][0].transcript);
    rec.onerror = (e) => gotError(e.error);
    rec.onend = () => endListening();
    rec.start();
    startWave();
  } else {
    await startWave();
    invoke('stt_listen').catch((err) => gotError(String(err)));
  }
}

function endListening() {
  listening = false;
  micBtn.classList.remove('listening');
  stopWave();
}

function gotTranscript(text) {
  endListening();
  if (text?.trim()) send(text.trim());
}

function gotError(error) {
  endListening();
  setThinking(false);
  const el = document.createElement('div');
  el.className = 'msg jarvis error';
  el.innerHTML = '<span class="who">J.A.R.V.I.S.</span><span class="body"></span>';
  el.querySelector('.body').textContent = error;
  document.getElementById('chat').appendChild(el);
}

listen('stt-result', ({ payload }) => gotTranscript(payload.text));
listen('stt-error', ({ payload }) => gotError(payload.error));

document.addEventListener('ptt-start', startListening);
document.addEventListener('ptt-stop', () => {
  if (listening && !webSpeech) invoke('stt_stop');
});
