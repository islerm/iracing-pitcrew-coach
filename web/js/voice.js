import { api, html, request, store, stored, useEffect, useState } from "./lib.js";

// ---------- Voice ----------

// One shared player: a single clip at a time, observed by every SpeakButton.
const voiceState = { info: null, infoPromise: null, key: null, status: "idle", radio: stored("pcc.voiceRadio", "on") === "on", error: null };

const voiceCache = new Map(); // key -> AudioBuffer[], most recent last

const VOICE_CACHE_MAX = 6;

const voiceListeners = new Set();

const voiceSources = new Set(); // scheduled AudioBufferSourceNodes not yet ended

let voiceCtx = null;

let voiceAbort = null;

let voiceToken = 0;

function voiceEmit() {
  voiceListeners.forEach((fn) => fn());
}

function voiceContext() {
  if (!voiceCtx) voiceCtx = new (window.AudioContext || window.webkitAudioContext)();
  return voiceCtx;
}

/** Identifies one clip (same text, same radio setting) so buttons can tell whether theirs is playing. */
const voiceKey = (radio, text) => `${radio ? "r" : "c"}|${text}`;

function voiceLoadInfo() {
  if (!voiceState.infoPromise) {
    voiceState.infoPromise = api("/api/voice")
      .catch(() => ({ engine: "none", voice: "", hint: "Voice is unavailable (restart the server to enable it)" }))
      .then((info) => {
        voiceState.info = info;
        voiceEmit();
        return info;
      });
  }
  return voiceState.infoPromise;
}

function voiceStop() {
  voiceToken += 1;
  if (voiceAbort) {
    voiceAbort.abort();
    voiceAbort = null;
  }
  voiceSources.forEach((source) => {
    source.onended = null;
    try {
      source.stop();
    } catch (_err) {
      /* already stopped */
    }
  });
  voiceSources.clear();
  voiceState.key = null;
  voiceState.status = "idle";
  voiceEmit();
}

/** Speak `text` sentence by sentence: audio starts after the first part, the rest is queued gaplessly. */
async function voicePlay(text) {
  voiceStop();
  const radio = voiceState.radio;
  const key = voiceKey(radio, text);
  const token = voiceToken;
  const abort = new AbortController();
  voiceAbort = abort;
  voiceState.key = key;
  voiceState.error = null;
  voiceState.status = "loading";
  voiceEmit();

  const ctx = voiceContext();
  ctx.resume().catch(() => {});
  let nextAt = 0;
  let fetching = true; // more parts may still arrive
  const idleIfDrained = () => {
    if (token !== voiceToken || fetching || voiceSources.size > 0) return;
    voiceAbort = null;
    voiceState.key = null;
    voiceState.status = "idle";
    voiceEmit();
  };
  const schedule = (buffer) => {
    const source = ctx.createBufferSource();
    source.buffer = buffer;
    source.connect(ctx.destination);
    const at = Math.max(ctx.currentTime + 0.03, nextAt);
    nextAt = at + buffer.duration;
    voiceSources.add(source);
    source.onended = () => {
      voiceSources.delete(source);
      idleIfDrained();
    };
    source.start(at);
    if (voiceState.status !== "playing") {
      voiceState.status = "playing";
      voiceEmit();
    }
  };

  const cached = voiceCache.get(key);
  if (cached) {
    voiceCache.delete(key);
    voiceCache.set(key, cached);
    cached.forEach(schedule);
    fetching = false;
    return;
  }

  const buffers = [];
  let scheduled = 0;
  try {
    const plan = await (await request("/api/speak/plan", { method: "POST", body: { text }, signal: abort.signal })).json();
    const parts = plan && Array.isArray(plan.parts) ? plan.parts : [];
    if (!parts.length) throw new Error("Nothing to say");
    for (let i = 0; i < parts.length; i += 1) {
      if (token !== voiceToken) return;
      const body = { text: parts[i], radio, first: i === 0, last: i === parts.length - 1 };
      const data = await (await request("/api/speak", { method: "POST", body, signal: abort.signal })).arrayBuffer();
      if (token !== voiceToken) return;
      const buffer = await ctx.decodeAudioData(data);
      if (token !== voiceToken) return; // superseded or stopped while generating
      buffers.push(buffer);
      schedule(buffer);
      scheduled += 1;
    }
    voiceCache.set(key, buffers);
    while (voiceCache.size > VOICE_CACHE_MAX) voiceCache.delete(voiceCache.keys().next().value);
  } catch (err) {
    if (token !== voiceToken) return;
    voiceState.error = err.message;
    if (!scheduled) voiceEmit(); // otherwise whatever is queued finishes first
  }
  fetching = false;
  idleIfDrained();
}

function voiceSetRadio(on) {
  voiceState.radio = on;
  store("pcc.voiceRadio", on ? "on" : "off");
  voiceEmit();
}

/** Subscribe to the shared voice player: engine info, radio flag, and play/stop. */
function useVoice() {
  const [, setTick] = useState(0);
  useEffect(() => {
    const fn = () => setTick((n) => n + 1);
    voiceListeners.add(fn);
    voiceLoadInfo();
    return () => voiceListeners.delete(fn);
  }, []);
  return {
    info: voiceState.info,
    radio: voiceState.radio,
    setRadio: voiceSetRadio,
    error: voiceState.error,
    key: voiceState.key,
    status: voiceState.status,
    play: voicePlay,
    stop: voiceStop,
  };
}

// Esc stops whatever is speaking.
window.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && voiceState.status !== "idle") voiceStop();
});

/** Ghost button that reads `text` aloud as the pit-lane engineer; click again to stop. */
export function SpeakButton({ text, label = "Hear this" }) {
  const v = useVoice();
  const disabled = !v.info || v.info.engine === "none" || !text;
  const mine = v.key === voiceKey(v.radio, text);
  const status = mine ? v.status : "idle";
  const title = !v.info
    ? "Checking voice engine…"
    : v.info.engine === "none"
      ? v.info.hint || "No voice engine available"
      : v.error
        ? `Voice failed: ${v.error}`
        : status === "playing" || status === "loading"
          ? "Stop (Esc)"
          : `${label} (${v.info.engine}${v.info.voice ? ` · ${v.info.voice}` : ""})`;
  return html`<button
    className="btn btn-ghost btn-icon btn-sm speak"
    disabled=${disabled}
    title=${title}
    aria-label=${status === "idle" ? label : "Stop speaking"}
    onClick=${() => (status === "idle" ? v.play(text) : v.stop())}
  >
    ${status === "loading" ? html`<span className="spinner"></span>` : status === "playing" ? "■" : "🔊"}
  </button>`;
}

/** Tiny checkbox-style toggle for the team-radio effect. */
export function RadioToggle() {
  const v = useVoice();
  if (!v.info || v.info.engine === "none") return null;
  return html`<label className="radio-toggle" title="Add a crackly team-radio effect to the voice">
    <input type="checkbox" checked=${v.radio} onChange=${(e) => v.setRadio(e.target.checked)} /> Radio
  </label>`;
}
