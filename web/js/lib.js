export const { useCallback, useContext, useEffect, useMemo, useRef, useState } = React;

export const html = htm.bind(React.createElement);

// ---------- API ----------

/** fetch with an optional JSON body; resolves to the Response, throws the server's `error` message otherwise. */
export async function request(path, { method = "GET", body, signal } = {}) {
  const response = await fetch(path, {
    method,
    headers: { "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal,
  });
  if (!response.ok) {
    let message = `Request failed (${response.status})`;
    try {
      const payload = JSON.parse(await response.text());
      if (payload && payload.error) message = payload.error;
    } catch (_err) {
      /* not JSON */
    }
    throw new Error(message);
  }
  return response;
}

async function readJson(response) {
  const text = await response.text();
  try {
    return text ? JSON.parse(text) : null;
  } catch (_err) {
    return null;
  }
}

export const api = (path) => request(path).then(readJson);

export const post = (path, body) => request(path, { method: "POST", body }).then(readJson);

export function stored(key, fallback) {
  try {
    const value = localStorage.getItem(key);
    return value === null ? fallback : value;
  } catch (_err) {
    return fallback;
  }
}

export function store(key, value) {
  try {
    localStorage.setItem(key, value);
  } catch (_err) {
    /* storage unavailable */
  }
}

/** useState persisted in localStorage under `key`. */
export function useStored(key, fallback) {
  const [value, setValue] = useState(() => stored(key, fallback));
  const update = useCallback(
    (next) => {
      setValue(next);
      store(key, next);
    },
    [key]
  );
  return [value, update];
}

export function useElementWidth() {
  const ref = useRef(null);
  const [width, setWidth] = useState(0);
  useEffect(() => {
    if (!ref.current) return undefined;
    const observer = new ResizeObserver(([entry]) => setWidth(Math.floor(entry.contentRect.width)));
    observer.observe(ref.current);
    return () => observer.disconnect();
  }, []);
  return [ref, width];
}

// ---------- Formatting ----------

export const isNum = (v) => typeof v === "number" && Number.isFinite(v);

export function fmtLap(seconds) {
  if (!isNum(seconds) || seconds <= 0) return "—";
  const m = Math.floor(seconds / 60);
  const s = seconds - m * 60;
  const sStr = s.toFixed(3).padStart(6, "0");
  return m > 0 ? `${m}:${sStr}` : s.toFixed(3);
}

/** Signed difference, e.g. "+0.123"; "±0.000" when it rounds to nothing. */
export function fmtDelta(v, digits = 3) {
  if (!isNum(v)) return "—";
  const mag = Math.abs(v).toFixed(digits);
  return Number(mag) === 0 ? "±" + mag : (v > 0 ? "+" : "−") + mag;
}

export const deltaClass = (d) => (!isNum(d) || Math.abs(d) < 0.0005 ? "even" : d < 0 ? "good" : "bad");

export const fmtNum = (v, digits = 1, unit = "") => (isNum(v) ? v.toFixed(digits) + unit : "—");

export function fmtClock(ms) {
  const total = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const mm = String(m).padStart(2, "0");
  const ss = String(s).padStart(2, "0");
  return h > 0 ? `${h}:${mm}:${ss}` : `${mm}:${ss}`;
}

export function fmtTimeOfDay(ms) {
  if (!ms) return "";
  return new Date(Number(ms)).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

export function fmtAgo(ms) {
  const diff = Date.now() - Number(ms);
  const min = Math.round(diff / 60000);
  if (min < 1) return "just now";
  if (min < 60) return `${min} min ago`;
  const h = Math.round(min / 60);
  if (h < 24) return `${h} h ago`;
  const d = Math.round(h / 24);
  return d === 1 ? "yesterday" : `${d} days ago`;
}

/** "porsche992rgt3_roadamerica 2026 full 2026-09-17 14-14-41.ibt" → readable parts. */
export function parseTelemetryName(name) {
  const stem = name.replace(/\.ibt$/i, "");
  const m = stem.match(/^(.+?)_(.+?) (\d{4}-\d{2}-\d{2}) (\d{2})-(\d{2})-\d{2}$/);
  if (!m) return { title: stem, sub: "" };
  return { title: m[2], sub: `${m[1]} · ${m[3]} ${m[4]}:${m[5]}` };
}

// ---------- Stats ----------

export const mean = (xs) => (xs.length ? xs.reduce((a, b) => a + b, 0) / xs.length : null);

export function stdDev(xs) {
  if (xs.length < 2) return null;
  const m = mean(xs);
  return Math.sqrt(xs.reduce((acc, x) => acc + (x - m) ** 2, 0) / (xs.length - 1));
}

export const fastest = (laps) => (laps.length ? laps.reduce((a, b) => (b.lap_time_s < a.lap_time_s ? b : a)) : null);

const sectorCount = (laps) => Math.max(0, ...laps.map((lap) => (lap.sectors || []).length));

export const sectorOf = (lap, i) => (lap.sectors || [])[i];

export const range = (n) => Array.from({ length: n }, (_, i) => i);

/** Stats over "counted" laps: complete laps the driver hasn't excluded. */
export function computeStats(laps, excluded) {
  const counted = laps.filter((lap) => lap.is_complete && !excluded.has(lap.lap_number));
  if (counted.length === 0) return null;

  const times = counted.map((lap) => lap.lap_time_s);
  const best = fastest(counted);
  const avg = mean(times);

  const nSectors = sectorCount(counted);
  const bestSectors = range(nSectors).map((i) => {
    const values = counted.map((lap) => sectorOf(lap, i)).filter(isNum);
    return values.length ? Math.min(...values) : null;
  });
  const optimal = nSectors && bestSectors.every(isNum) ? bestSectors.reduce((a, b) => a + b, 0) : null;

  // Where time goes on a typical lap: average gap to your own best in each sector.
  let focus = null;
  range(nSectors).forEach((i) => {
    const values = counted.map((lap) => sectorOf(lap, i)).filter(isNum);
    if (!values.length) return;
    const loss = mean(values) - bestSectors[i];
    if (focus === null || loss > focus.loss) focus = { sector: i + 1, loss };
  });

  let trend = null;
  if (counted.length >= 4) {
    const n = Math.min(3, Math.floor(counted.length / 2));
    trend = mean(times.slice(-n)) - mean(times.slice(0, n));
  }

  let bestRun = null;
  if (counted.length >= 3) {
    for (let i = 0; i + 3 <= counted.length; i++) {
      const runAvg = mean(times.slice(i, i + 3));
      if (bestRun === null || runAvg < bestRun.avg) {
        bestRun = { avg: runAvg, from: counted[i].lap_number, to: counted[i + 2].lap_number };
      }
    }
  }

  const speeds = counted.map((lap) => lap.avg_speed_kph).filter(isNum);

  return {
    counted,
    best,
    avg,
    sd: stdDev(times),
    nSectors,
    bestSectors,
    optimal,
    focus,
    trend,
    bestRun,
    nearBest: times.filter((t) => t - best.lap_time_s <= 0.5).length,
    avgSpeed: mean(speeds),
  };
}

// ---------- Small components ----------

export function Kpi({ label, value, sub, highlight, title }) {
  return html`<div className=${"kpi" + (highlight ? " highlight" : "")} title=${title || ""}>
    <div className="kpi-label">${label}</div>
    <div className="kpi-value">${value}</div>
    <div className="kpi-sub">${sub || " "}</div>
  </div>`;
}

export function Segmented({ value, options, onChange, label }) {
  return html`<div className="segmented" role="radiogroup" aria-label=${label}>
    ${options.map(
      (opt) => html`<button
        key=${opt.value}
        role="radio"
        aria-checked=${value === opt.value}
        className=${value === opt.value ? "on" : ""}
        onClick=${() => onChange(opt.value)}
      >${opt.label}</button>`
    )}
  </div>`;
}

// ---------- Off track ----------

export const offTracks = (lap) => (lap && lap.off_track_pcts ? lap.off_track_pcts.length : 0);

export function offTrackTitle(lap) {
  const n = offTracks(lap);
  if (!n) return "";
  const inc = isNum(lap.incidents) && lap.incidents > 0 ? `, ${lap.incidents}x incident${lap.incidents === 1 ? "" : "s"}` : "";
  return `Went off track ${n === 1 ? "once" : n + " times"}${inc}`;
}

export function OffTrackMark({ lap }) {
  if (!offTracks(lap)) return null;
  const n = offTracks(lap);
  return html`<span className="off-mark" title=${offTrackTitle(lap)} aria-label=${offTrackTitle(lap)}>!${n > 1 ? html`<small>${n}</small>` : null}</span>`;
}

// ---------- Weather ----------

export const WETNESS = [null, "Dry", "Mostly dry", "Very lightly wet", "Lightly wet", "Moderately wet", "Very wet", "Extremely wet"];

export const SKY_ICON = { clear: "☀️", "partly cloudy": "⛅", "mostly cloudy": "🌥️", overcast: "☁️" };

export function fmtTempRange(r) {
  return Math.abs(r.end - r.start) >= 0.5 ? `${r.start.toFixed(0)}→${r.end.toFixed(0)}°C` : `${r.start.toFixed(0)}°C`;
}

export const lapWeatherTitle = (lap) =>
  [isNum(lap.air_temp_c) ? `Air ${lap.air_temp_c.toFixed(1)}°C` : null, lap.track_wetness ? `Track ${(WETNESS[lap.track_wetness] || "").toLowerCase()}` : null].filter(Boolean).join(" · ");

/** Min and max track temperature over the timed laps, when it moved enough to be worth charting. */
export function trackTempSpan(laps) {
  const temps = laps.filter((l) => l.is_complete && isNum(l.track_temp_c)).map((l) => l.track_temp_c);
  if (temps.length < 2) return null;
  const lo = Math.min(...temps);
  const hi = Math.max(...temps);
  return hi - lo >= 0.5 ? { lo, hi } : null;
}

export const rangeTitle = (what, r) => `${what}: ${r.start.toFixed(1)}°C at the start, ${r.end.toFixed(1)}°C at the end (${r.min.toFixed(1)}–${r.max.toFixed(1)}°C)`;
