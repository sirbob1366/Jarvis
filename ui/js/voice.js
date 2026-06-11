// Voice I/O.
// TTS: webview speechSynthesis (WebView2 exposes the installed Windows voices) —
//      prefers a UK male voice, rate/pitch from Settings, respects Mute.
// STT: WebView2 lacks Web Speech recognition, so push-to-talk invokes the Rust
//      WinRT recognizer (stt_listen); we still probe for SpeechRecognition and
//      use it if the webview ever gains support.
// Waveform: getUserMedia analyser drives the strip; mic is only open while
//      listening. If mic access fails, an animated fallback renders instead.

import { prefs, send, setThinking } from './app.js';

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const wave = document.getElementById('wave');
const canvas = document.getElementById('wave-canvas');
const ctx2d = canvas.getContext('2d');

// ---------- text-to-speech ----------

let voice = null;

function pickVoice() {
  const voices = speechSynthesis.getVoices();
  if (!voices.length) return;
  voice =
    voices.find((v) => /en-GB/i.test(v.lang) && /ryan|george|thomas|daniel|male/i.test(v.name)) ||
    voices.find((v) => /en-GB/i.test(v.lang)) ||
    voices.find((v) => /^en/i.test(v.lang)) ||
    voices[0];
}
speechSynthesis.addEventListener('voiceschanged', pickVoice);
pickVoice();

export async function speak(text) {
  if (!('speechSynthesis' in window) || !text) return;
  if (await invoke('is_muted')) return;
  speechSynthesis.cancel();
  const u = new SpeechSynthesisUtterance(text);
  u.voice = voice;
  u.rate = prefs.rate;
  u.pitch = prefs.pitch;
  speechSynthesis.speak(u);
}

// Every reply is spoken (and already rendered as text by app.js).
document.addEventListener('jarvis-said', (e) => speak(e.detail));
listen('mute-changed', ({ payload }) => {
  if (payload) speechSynthesis.cancel();
});

// ---------- waveform ----------

let audioCtx = null;
let analyser = null;
let micStream = null;
let rafId = null;

async function startWave() {
  wave.hidden = false;
  try {
    micStream = await navigator.mediaDevices.getUserMedia({ audio: true });
    audioCtx = new AudioContext();
    analyser = audioCtx.createAnalyser();
    analyser.fftSize = 256;
    audioCtx.createMediaStreamSource(micStream).connect(analyser);
  } catch {
    analyser = null; // fallback animation
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
  speechSynthesis.cancel(); // don't transcribe our own voice

  if (webSpeech) {
    // Rare: webview gained Web Speech support — prefer it.
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

// Hotkey wiring from app.js
document.addEventListener('ptt-start', startListening);
document.addEventListener('ptt-stop', () => {
  // "Hold" behaviour: releasing the hotkey ends the capture; the recognizer
  // also auto-stops on silence, so tap-and-speak works too.
  if (listening && !webSpeech) invoke('stt_stop');
});
