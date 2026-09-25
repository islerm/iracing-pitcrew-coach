(function () {
  const { useCallback, useEffect, useMemo, useRef, useState } = React;
  const html = htm.bind(React.createElement);

  const SAMPLE_PATH = "data/practice_session.csv";

  // ---------- API ----------

  async function api(path, options) {
    const response = await fetch(path, {
      headers: { "Content-Type": "application/json" },
      ...options,
    });
    const text = await response.text();
    let payload = null;
    if (text) {
      try {
        payload = JSON.parse(text);
      } catch (_err) {
        payload = null;
      }
    }
    if (!response.ok) {
      throw new Error((payload && payload.error) || `Request failed (${response.status})`);
    }
    return payload;
  }

  function stored(key, fallback) {
    try {
      const value = localStorage.getItem(key);
      return value === null ? fallback : value;
    } catch (_err) {
      return fallback;
    }
  }

  function store(key, value) {
    try {
      localStorage.setItem(key, value);
    } catch (_err) {
      /* storage unavailable */
    }
  }

  function useElementWidth() {
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

  const isNum = (v) => typeof v === "number" && Number.isFinite(v);

  function fmtLap(seconds) {
    if (!isNum(seconds) || seconds <= 0) return "—";
    const m = Math.floor(seconds / 60);
    const s = seconds - m * 60;
    const sStr = s.toFixed(3).padStart(6, "0");
    return m > 0 ? `${m}:${sStr}` : s.toFixed(3);
  }

  function fmtDelta(seconds, digits = 3) {
    if (!isNum(seconds)) return "—";
    if (Math.abs(seconds) < 0.5 * 10 ** -digits) return "±" + (0).toFixed(digits);
    return (seconds > 0 ? "+" : "−") + Math.abs(seconds).toFixed(digits);
  }

  const deltaClass = (d) => (!isNum(d) || Math.abs(d) < 0.0005 ? "even" : d < 0 ? "good" : "bad");
  const fmtNum = (v, digits = 1, unit = "") => (isNum(v) ? v.toFixed(digits) + unit : "—");
  const fmtSigned = (v, digits = 1) => {
    if (!isNum(v)) return "—";
    const r = v.toFixed(digits);
    return Number(r) === 0 ? "±" + Math.abs(Number(r)).toFixed(digits) : (v > 0 ? "+" : "−") + Math.abs(v).toFixed(digits);
  };

  function fmtClock(ms) {
    const total = Math.max(0, Math.floor(ms / 1000));
    const h = Math.floor(total / 3600);
    const m = Math.floor((total % 3600) / 60);
    const s = total % 60;
    const mm = String(m).padStart(2, "0");
    const ss = String(s).padStart(2, "0");
    return h > 0 ? `${h}:${mm}:${ss}` : `${mm}:${ss}`;
  }

  function fmtTimeOfDay(ms) {
    if (!ms) return "";
    return new Date(Number(ms)).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  }

  function fmtAgo(ms) {
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
  function parseTelemetryName(name) {
    const stem = name.replace(/\.(ibt|csv)$/i, "");
    const m = stem.match(/^(.+?)_(.+?) (\d{4}-\d{2}-\d{2}) (\d{2})-(\d{2})-\d{2}$/);
    if (!m) return { title: stem, sub: "" };
    return { title: m[2], sub: `${m[1]} · ${m[3]} ${m[4]}:${m[5]}` };
  }

  // ---------- Stats ----------

  const mean = (xs) => (xs.length ? xs.reduce((a, b) => a + b, 0) / xs.length : null);

  function stdDev(xs) {
    if (xs.length < 2) return null;
    const m = mean(xs);
    return Math.sqrt(xs.reduce((acc, x) => acc + (x - m) ** 2, 0) / (xs.length - 1));
  }

  const sectorCount = (laps) => Math.max(0, ...laps.map((lap) => (lap.sectors || []).length));
  const sectorOf = (lap, i) => (lap.sectors || [])[i];
  const range = (n) => Array.from({ length: n }, (_, i) => i);

  /** Stats over "counted" laps: complete laps the driver hasn't excluded. */
  function computeStats(laps, excluded) {
    const counted = laps.filter((lap) => lap.is_complete && !excluded.has(lap.lap_number));
    if (counted.length === 0) return null;

    const times = counted.map((lap) => lap.lap_time_s);
    const best = counted.reduce((a, b) => (b.lap_time_s < a.lap_time_s ? b : a));
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

  function Toast({ toast, onClose }) {
    useEffect(() => {
      if (!toast || toast.kind === "error") return undefined;
      const id = setTimeout(onClose, 3500);
      return () => clearTimeout(id);
    }, [toast]);
    if (!toast) return null;
    return html`<div className=${"toast " + toast.kind} role="status">
      <span>${toast.text}</span>
      <button onClick=${onClose} aria-label="Dismiss">✕</button>
    </div>`;
  }

  function Kpi({ label, value, sub, highlight, title }) {
    return html`<div className=${"kpi" + (highlight ? " highlight" : "")} title=${title || ""}>
      <div className="kpi-label">${label}</div>
      <div className="kpi-value">${value}</div>
      <div className="kpi-sub">${sub || " "}</div>
    </div>`;
  }

  function Segmented({ value, options, onChange, label }) {
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

  // ---------- Import ----------

  function ImportModal({ busy, onCancel, onImport }) {
    const [dir, setDir] = useState(() => stored("pcc.importDir", ""));
    const [listing, setListing] = useState(null);
    const [loading, setLoading] = useState(false);
    const [path, setPath] = useState("");
    const [pending, setPending] = useState(null);

    const load = useCallback((folder) => {
      setLoading(true);
      api("/api/telemetry-files" + (folder ? `?dir=${encodeURIComponent(folder)}` : ""))
        .then((result) => {
          setListing(result);
          if (result.dir) setDir(result.dir);
        })
        .catch(() => setListing({ dir: folder, files: [] }))
        .finally(() => setLoading(false));
    }, []);

    useEffect(() => load(dir), []);

    const pick = (filePath) => {
      setPending(filePath);
      if (listing && listing.dir) store("pcc.importDir", listing.dir);
      onImport(filePath);
    };

    return html`<div className="overlay" onClick=${(e) => e.target === e.currentTarget && !busy && onCancel()}>
      <div className="modal modal-wide">
        <div className="modal-head">
          <h2>Open a session</h2>
          <button className="btn btn-ghost btn-icon" onClick=${onCancel} disabled=${busy} aria-label="Close">✕</button>
        </div>
        <p className="muted">
          Pick an iRacing <span className="mono">.ibt</span> recording to get lap times, sectors, a track map and
          telemetry overlays. Turn on telemetry logging in iRacing with <span className="kbd">Alt</span>+<span className="kbd">L</span>.
        </p>
        <form
          className="row-form"
          onSubmit=${(e) => {
            e.preventDefault();
            load(dir);
          }}
        >
          <input className="input mono" value=${dir} onInput=${(e) => setDir(e.target.value)} placeholder="Telemetry folder" aria-label="Telemetry folder" />
          <button className="btn" type="submit" disabled=${loading}>Refresh</button>
        </form>
        <div className="file-list">
          ${loading && !listing
            ? html`<p className="muted pad">Looking for sessions…</p>`
            : listing && listing.files.length === 0
              ? html`<p className="muted pad">No .ibt or .csv files in this folder.</p>`
              : (listing ? listing.files : []).map((file) => {
                  const { title, sub } = parseTelemetryName(file.name);
                  const isPending = busy && pending === file.path;
                  return html`<button key=${file.path} className="file-row" disabled=${busy} onClick=${() => pick(file.path)} title=${file.path}>
                    <span className="file-main">
                      <span className="file-title">${title}</span>
                      <span className="file-sub">${sub || file.name}</span>
                    </span>
                    <span className="file-meta">
                      ${isPending
                        ? html`<span className="spinner"></span>`
                        : html`<span>${fmtAgo(file.modified_ms)}</span><span className="faint">${(file.size_bytes / 1e6).toFixed(1)} MB</span>`}
                    </span>
                  </button>`;
                })}
        </div>
        <details className="manual">
          <summary>Open a file by path</summary>
          <form
            className="row-form"
            onSubmit=${(e) => {
              e.preventDefault();
              pick(path);
            }}
          >
            <input className="input mono" value=${path} onInput=${(e) => setPath(e.target.value)} placeholder=${SAMPLE_PATH} aria-label="File path" />
            <button type="button" className="btn btn-ghost" onClick=${() => setPath(SAMPLE_PATH)}>Sample</button>
            <button type="submit" className="btn btn-primary" disabled=${busy || !path.trim()}>Open</button>
          </form>
        </details>
        ${busy ? html`<p className="muted"><span className="spinner inline"></span> Reading telemetry and writing coach notes…</p>` : null}
      </div>
    </div>`;
  }

  // ---------- Top bar ----------

  function TopBar({ isRecording, recordingStartedAt, replayFile, busy, model, setModel, onStart, onStop, onImport }) {
    const [now, setNow] = useState(Date.now());
    const [settingsOpen, setSettingsOpen] = useState(false);

    useEffect(() => {
      if (!isRecording) return undefined;
      const id = setInterval(() => setNow(Date.now()), 500);
      return () => clearInterval(id);
    }, [isRecording]);

    return html`<header className="topbar">
      <div className="brand">
        <div className="brand-mark">PC</div>
        <div>Pit Crew Coach<small>iRacing practice</small></div>
      </div>
      <div className="topbar-spacer"></div>
      ${isRecording && recordingStartedAt
        ? html`<span className="rec-pill" title=${replayFile ? "Replaying " + replayFile : ""}>
            <span className="rec-dot"></span>${replayFile ? "REPLAY" : "REC"} <span className="mono">${fmtClock(now - recordingStartedAt)}</span>
          </span>`
        : null}
      <button className="btn btn-ghost" onClick=${onImport} disabled=${busy}>Open session</button>
      <div className="rel">
        <button className="btn btn-ghost btn-icon" title="Settings" aria-label="Settings" onClick=${() => setSettingsOpen(!settingsOpen)}>⚙</button>
        ${settingsOpen
          ? html`<div className="popover">
              <div className="field">
                <label htmlFor="model">Coach model (Ollama)</label>
                <input id="model" className="input mono" value=${model} disabled=${isRecording} onInput=${(e) => setModel(e.target.value)} placeholder="llama3.2" />
              </div>
              <p className="faint" style=${{ fontSize: "12px" }}>Used for the written coach feedback after each run.</p>
            </div>`
          : null}
      </div>
      ${isRecording
        ? html`<button className="btn btn-stop" onClick=${onStop} disabled=${busy}>
            ${busy ? html`<span className="spinner"></span> Analyzing…` : html`<span>■</span> Stop & analyze`}
          </button>`
        : html`<button className="btn btn-record" onClick=${onStart} disabled=${busy}>
            ${busy ? html`<span className="spinner"></span> Working…` : html`<span className="rec-dot"></span> Start recording`}
          </button>`}
    </header>`;
  }

  // ---------- Sidebar ----------

  function runTitle(split) {
    if (split.track_label) return split.track_label;
    return split.source === "Live" ? `Run ${split.id.replace("split-", "")}` : split.source;
  }

  function Sidebar({ splits, selectedId, onSelect, onDelete }) {
    const ordered = splits.slice().reverse();
    return html`<aside className="sidebar">
      <div className="section-label"><span>Runs</span><span>${splits.length}</span></div>
      ${ordered.length === 0
        ? html`<p className="faint" style=${{ fontSize: "13px" }}>No runs yet. Record a stint or open a session file.</p>`
        : ordered.map(
            (split) => html`<div
              key=${split.id}
              className=${"session" + (split.id === selectedId ? " active" : "")}
              role="button"
              tabIndex="0"
              onClick=${() => onSelect(split.id)}
              onKeyDown=${(e) => e.key === "Enter" && onSelect(split.id)}
            >
              <div className="session-title" title=${split.source}>
                ${split.source === "Live" ? html`<span className="tag tag-live">Live</span>` : null}
                <span>${runTitle(split)}</span>
              </div>
              <div className="session-best">${fmtLap(split.fastest_lap_time_s)}</div>
              <div className="session-meta">
                <span>${[split.car, fmtTimeOfDay(split.started_at_ms), `${split.complete_lap_count}/${split.lap_count} laps`].filter(Boolean).join(" · ")}</span>
                <button
                  className="session-delete"
                  title="Delete run"
                  aria-label="Delete run"
                  onClick=${(e) => {
                    e.stopPropagation();
                    onDelete(split.id);
                  }}
                >✕</button>
              </div>
            </div>`
          )}
    </aside>`;
  }

  // ---------- Live panel ----------

  function LivePanel({ live }) {
    const laps = (live && live.laps) || [];
    const complete = laps.filter((lap) => lap.is_complete);
    const best = complete.length ? complete.reduce((a, b) => (b.lap_time_s < a.lap_time_s ? b : a)) : null;
    const last = laps.length ? laps[laps.length - 1] : null;
    const lastDelta = best && last && last.is_complete ? last.lap_time_s - best.lap_time_s : null;
    const sd = stdDev(complete.map((lap) => lap.lap_time_s));

    return html`<section className="card live">
      <div className="card-head">
        <div className="card-title">Live run</div>
        <span className="faint" style=${{ fontSize: "12px" }}>Laps appear as you cross the line</span>
      </div>
      <div className="live-grid">
        <div>
          <div className="kpi-label">Last lap</div>
          <div className="live-big">${last ? fmtLap(last.lap_time_s) : "—"}</div>
          <div className=${"mono " + deltaClass(lastDelta)} style=${{ fontSize: "13px" }}>
            ${last && !last.is_complete ? "not timed" : lastDelta !== null ? fmtDelta(lastDelta) + " to best" : " "}
          </div>
        </div>
        <div>
          <div className="kpi-label">Best this run</div>
          <div className="live-big purple">${best ? fmtLap(best.lap_time_s) : "—"}</div>
          <div className="faint" style=${{ fontSize: "13px" }}>${best ? "Lap " + best.lap_number : " "}</div>
        </div>
        <div>
          <div className="kpi-label">Consistency</div>
          <div className="live-big">${sd !== null ? "±" + sd.toFixed(3) : "—"}</div>
          <div className="faint" style=${{ fontSize: "13px" }}>std dev</div>
        </div>
        <div>
          <div className="kpi-label">Timed laps</div>
          <div className="live-big">${complete.length}</div>
          <div className="faint" style=${{ fontSize: "13px" }}>${laps.length - complete.length ? `+ ${laps.length - complete.length} untimed` : " "}</div>
        </div>
      </div>
      ${live && live.capture_ended
        ? live.is_replay
          ? html`<p className="muted" style=${{ marginBottom: "10px" }}>Replay finished — click <b>Stop & analyze</b>.</p>`
          : html`<p className="bad" style=${{ marginBottom: "10px" }}>
              Telemetry capture stopped — iRacing may not be running. Click <b>Stop & analyze</b> to see the details.
            </p>`
        : null}
      ${laps.length === 0
        ? live && live.capture_ended
          ? null
          : html`<p className="muted">Waiting for your first lap… make sure iRacing is running and you're on track.</p>`
        : html`<div className="live-laps">
            ${laps.map((lap) => {
              const d = best && lap.is_complete ? lap.lap_time_s - best.lap_time_s : null;
              const isBest = best && lap.lap_number === best.lap_number;
              return html`<div key=${lap.lap_number} className=${"lap-chip" + (isBest ? " is-best" : "")} style=${{ cursor: "default" }}>
                <span className="n">L${lap.lap_number} ${!lap.is_complete ? html`<span className="tag tag-out">untimed</span>` : null}</span>
                <span className="t">${fmtLap(lap.lap_time_s)}</span>
                <span className=${"d " + (isBest ? "purple" : deltaClass(d))}>${isBest ? "best" : d !== null ? fmtDelta(d) : " "}</span>
              </div>`;
            })}
          </div>`}
    </section>`;
  }

  // ---------- Pace chart ----------

  function PaceChart({ laps, stats, excluded, selected, compare, onPick }) {
    const W = 1000;
    const H = 200;
    const pad = { l: 64, r: 16, t: 14, b: 24 };
    const timed = laps.filter((lap) => lap.is_complete);
    if (timed.length < 2 || !stats) {
      return html`<p className="muted">Need at least two timed laps to draw a pace chart.</p>`;
    }

    // Scale to counted laps so an out lap or a spin doesn't flatten everything else.
    const countedTimes = stats.counted.map((lap) => lap.lap_time_s);
    let lo = Math.min(...countedTimes);
    let hi = Math.max(...countedTimes);
    const span = Math.max(hi - lo, 0.4);
    lo -= span * 0.12;
    hi += span * 0.12;

    const x = (i) => pad.l + (i / (timed.length - 1)) * (W - pad.l - pad.r);
    const y = (t) => pad.t + ((Math.min(Math.max(t, lo), hi) - lo) / (hi - lo)) * (H - pad.t - pad.b);

    const ticks = [0, 0.25, 0.5, 0.75, 1].map((f) => lo + f * (hi - lo));
    const path = timed.map((lap, i) => `${i ? "L" : "M"}${x(i).toFixed(1)},${y(lap.lap_time_s).toFixed(1)}`).join(" ");
    const labelEvery = Math.ceil(timed.length / 16);

    return html`<svg className="chart" viewBox=${`0 0 ${W} ${H}`} role="img" aria-label="Lap time by lap">
      <g className="grid">
        ${ticks.map(
          (t, i) => html`<g key=${i}>
            <line x1=${pad.l} x2=${W - pad.r} y1=${y(t)} y2=${y(t)} />
            <text x=${pad.l - 8} y=${y(t) + 4} textAnchor="end">${fmtLap(t)}</text>
          </g>`
        )}
      </g>
      <line className="avg-line" x1=${pad.l} x2=${W - pad.r} y1=${y(stats.avg)} y2=${y(stats.avg)} />
      <line className="best-line" x1=${pad.l} x2=${W - pad.r} y1=${y(stats.best.lap_time_s)} y2=${y(stats.best.lap_time_s)} />
      <path className="pace" d=${path} />
      ${timed.map((lap, i) => {
        const cls = [
          "pt",
          lap.lap_number === stats.best.lap_number ? "is-best" : "",
          excluded.has(lap.lap_number) ? "excluded" : "",
          lap.lap_number === compare ? "compare" : "",
          lap.lap_number === selected ? "selected" : "",
        ].join(" ");
        return html`<g key=${lap.lap_number} className=${cls} onClick=${(e) => onPick(lap.lap_number, e.shiftKey)}>
          <title>Lap ${lap.lap_number}: ${fmtLap(lap.lap_time_s)}</title>
          <circle cx=${x(i)} cy=${y(lap.lap_time_s)} r="5" />
          ${i % labelEvery === 0 ? html`<text x=${x(i)} y=${H - 6}>${lap.lap_number}</text>` : null}
        </g>`;
      })}
    </svg>`;
  }

  // ---------- Track analysis helpers ----------

  const idxOf = (pct, n) => Math.max(0, Math.min(n, Math.round(pct * n)));

  /** Each turn owns the stretch of track from halfway after the previous turn to halfway to the next. */
  function turnSegments(turns) {
    return turns.map((turn, i) => ({
      ...turn,
      start: i === 0 ? 0 : (turns[i - 1].pct + turn.pct) / 2,
      end: i === turns.length - 1 ? 1 : (turn.pct + turns[i + 1].pct) / 2,
    }));
  }

  function brakePoint(trace, a, apex) {
    for (let j = a; j <= apex; j++) {
      if (trace.brake[j] >= 10) return j;
    }
    return null;
  }

  /**
   * Handling in one corner, matching the coach's numbers (src/handling.rs):
   * grip = mean combined g while braking or cornering, as % of the session peak;
   * balance = mean steering excess over the most laterally loaded third (+ understeer);
   * abs = % of braking samples with ABS active.
   */
  function handling(trace, a, b, peakG) {
    if (!trace) return {};
    const idx = [];
    for (let j = a; j <= b; j++) idx.push(j);
    const hasG = trace.lat_g && trace.lat_g.length;
    let grip = null;
    if (hasG && isNum(peakG)) {
      const working = idx.filter((j) => trace.brake[j] >= 5 || Math.abs(trace.lat_g[j]) > 0.3 * peakG);
      if (working.length) grip = (working.reduce((s, j) => s + Math.hypot(trace.lat_g[j], trace.long_g[j]), 0) / working.length / peakG) * 100;
    }
    let balance = null;
    if (trace.balance_deg && trace.balance_deg.length) {
      const load = (j) => (hasG ? Math.abs(trace.lat_g[j]) : Math.abs(trace.yaw_rate_dps[j] * trace.speed_kph[j]));
      const loaded = idx.slice().sort((x, y) => load(y) - load(x)).slice(0, Math.max(1, Math.floor(idx.length / 3)));
      balance = loaded.reduce((s, j) => s + trace.balance_deg[j], 0) / loaded.length;
    }
    let abs = null;
    if (trace.abs && trace.abs.length) {
      const braking = idx.filter((j) => trace.brake[j] >= 10);
      abs = braking.length ? (braking.filter((j) => trace.abs[j]).length / braking.length) * 100 : 0;
    }
    return { grip, balance, abs };
  }

  function cornerStats(segments, sel, ref, lengthM, peakG) {
    if (!sel) return [];
    const n = sel.time_s.length - 1;
    return segments.map((seg) => {
      const a = idxOf(seg.start, n);
      const b = idxOf(seg.end, n);
      const apex = idxOf(seg.pct, n);
      let minSel = Infinity;
      let minRef = Infinity;
      for (let j = a; j <= b; j++) {
        minSel = Math.min(minSel, sel.speed_kph[j]);
        if (ref) minRef = Math.min(minRef, ref.speed_kph[j]);
      }
      const timeSel = sel.time_s[b] - sel.time_s[a];
      const timeRef = ref ? ref.time_s[b] - ref.time_s[a] : null;
      const bpSel = brakePoint(sel, a, apex);
      const bpRef = ref ? brakePoint(ref, a, apex) : null;
      const hSel = handling(sel, a, b, peakG);
      const hRef = handling(ref, a, b, peakG);
      return {
        ...seg,
        gripSel: hSel.grip,
        gripRef: isNum(hRef.grip) ? hRef.grip : null,
        balSel: hSel.balance,
        balRef: isNum(hRef.balance) ? hRef.balance : null,
        absSel: hSel.abs,
        minSel,
        minRef: ref ? minRef : null,
        delta: ref ? timeSel - timeRef : null,
        // Positive = the selected lap braked later (closer to the corner).
        brakeLaterM: bpSel !== null && bpRef !== null ? ((bpSel - bpRef) / n) * lengthM : null,
      };
    });
  }

  function lerpColor(stops, t) {
    const x = Math.max(0, Math.min(1, t)) * (stops.length - 1);
    const i = Math.min(stops.length - 2, Math.floor(x));
    const f = x - i;
    const [a, b] = [stops[i], stops[i + 1]];
    return `rgb(${Math.round(a[0] + (b[0] - a[0]) * f)},${Math.round(a[1] + (b[1] - a[1]) * f)},${Math.round(a[2] + (b[2] - a[2]) * f)})`;
  }
  const SPEED_STOPS = [
    [255, 95, 95],
    [255, 181, 71],
    [61, 220, 132],
  ];

  // ---------- Track map ----------

  function TrackMapView({ map, sel, refTrace: ref, mode, cursorPct, onHover, activeTurn, onTurnClick }) {
    const svgRef = useRef(null);
    const points = map.points;
    const n = points.length - 1;

    const geometry = useMemo(() => {
      const xs = points.map((p) => p[0]);
      const ys = points.map((p) => p[1]);
      const minX = Math.min(...xs);
      const maxX = Math.max(...xs);
      const minY = Math.min(...ys);
      const maxY = Math.max(...ys);
      const size = Math.max(maxX - minX, maxY - minY);
      const s = size / 440; // metres per screen pixel at the default map size
      const pad = 30 * s;
      return {
        s,
        viewBox: `${minX - pad} ${-maxY - pad} ${maxX - minX + 2 * pad} ${maxY - minY + 2 * pad}`,
        path: points.map((p, i) => `${i ? "L" : "M"}${p[0].toFixed(1)},${(-p[1]).toFixed(1)}`).join(" ") + "Z",
      };
    }, [map]);
    const { s } = geometry;

    // Coloured overlay, drawn in short chunks.
    const overlay = useMemo(() => {
      if (!sel) return null;
      const chunk = Math.max(2, Math.round(n / 300));
      const out = [];
      let maxSlope = 0;
      const slopes = [];
      if (mode === "delta" && ref) {
        for (let k = 0; k < n; k += chunk) {
          const e = Math.min(n, k + chunk);
          const slope = sel.time_s[e] - ref.time_s[e] - (sel.time_s[k] - ref.time_s[k]);
          slopes.push(slope);
        }
        const sorted = slopes.map(Math.abs).sort((a, b) => a - b);
        maxSlope = sorted[Math.floor(sorted.length * 0.95)] || 1e-6;
      }
      const vMin = Math.min(...sel.speed_kph);
      const vMax = Math.max(...sel.speed_kph);
      let c = 0;
      for (let k = 0; k < n; k += chunk, c++) {
        const e = Math.min(n, k + chunk);
        let color;
        if (mode === "delta") {
          if (!ref) {
            color = "var(--text-3)";
          } else {
            const slope = slopes[c];
            const strength = Math.min(1, Math.abs(slope) / maxSlope);
            if (strength < 0.08) color = "rgba(154,163,178,0.55)";
            else color = slope < 0 ? `rgba(61,220,132,${0.35 + 0.65 * strength})` : `rgba(255,95,95,${0.35 + 0.65 * strength})`;
          }
        } else if (mode === "speed") {
          let v = 0;
          for (let j = k; j <= e; j++) v += sel.speed_kph[j];
          v /= e - k + 1;
          color = lerpColor(SPEED_STOPS, (v - vMin) / Math.max(1, vMax - vMin));
        } else {
          let thr = 0;
          let brk = 0;
          for (let j = k; j <= e; j++) {
            thr += sel.throttle[j];
            brk += sel.brake[j];
          }
          thr /= e - k + 1;
          brk /= e - k + 1;
          color = brk > 8 ? "#ff5f5f" : thr > 90 ? "#3ddc84" : thr > 15 ? "#e8c547" : "#6b7486";
        }
        const d = [];
        for (let j = k; j <= e; j++) d.push(`${j === k ? "M" : "L"}${points[j][0].toFixed(1)},${(-points[j][1]).toFixed(1)}`);
        out.push(html`<path key=${k} d=${d.join(" ")} stroke=${color} className="map-overlay" />`);
      }
      return out;
    }, [map, sel, ref, mode]);

    // Turn badges sit just outside each corner.
    const badges = useMemo(() => {
      return map.turns.map((turn, ti) => {
        const i = idxOf(turn.pct, n);
        const k = Math.max(3, Math.round(n / 400));
        const p0 = points[Math.max(0, i - k)];
        const p = points[i];
        const p1 = points[Math.min(n, i + k)];
        const t1 = [p[0] - p0[0], p[1] - p0[1]];
        const t2 = [p1[0] - p[0], p1[1] - p[1]];
        const cross = t1[0] * t2[1] - t1[1] * t2[0]; // > 0: turning left
        const tx = p1[0] - p0[0];
        const ty = p1[1] - p0[1];
        const len = Math.hypot(tx, ty) || 1;
        const side = cross > 0 ? -1 : 1; // outside of the corner
        const nx = (side * ty) / len;
        const ny = (side * -tx) / len;
        return { ...turn, i: ti, x: p[0] + nx * 20 * s, y: -(p[1] + ny * 20 * s) };
      });
    }, [map]);

    const startLine = useMemo(() => {
      const p = points[0];
      const q = points[Math.min(n, 5)];
      const len = Math.hypot(q[0] - p[0], q[1] - p[1]) || 1;
      const nx = -(q[1] - p[1]) / len;
      const ny = (q[0] - p[0]) / len;
      return { x1: p[0] + nx * 11 * s, y1: -(p[1] + ny * 11 * s), x2: p[0] - nx * 11 * s, y2: -(p[1] - ny * 11 * s) };
    }, [map]);

    function onMove(e) {
      const svg = svgRef.current;
      if (!svg) return;
      const pt = svg.createSVGPoint();
      pt.x = e.clientX;
      pt.y = e.clientY;
      const local = pt.matrixTransform(svg.getScreenCTM().inverse());
      let best = -1;
      let bestD = Infinity;
      for (let j = 0; j <= n; j += 2) {
        const d = (points[j][0] - local.x) ** 2 + (-points[j][1] - local.y) ** 2;
        if (d < bestD) {
          bestD = d;
          best = j;
        }
      }
      if (best >= 0 && Math.sqrt(bestD) < 40 * s) onHover(best / n);
    }

    const cursor = isNum(cursorPct) ? points[idxOf(cursorPct, n)] : null;
    const activeSeg = activeTurn !== null && activeTurn !== undefined ? turnSegments(map.turns)[activeTurn] : null;
    const activePath = activeSeg
      ? (() => {
          const a = idxOf(activeSeg.start, n);
          const b = idxOf(activeSeg.end, n);
          const d = [];
          for (let j = a; j <= b; j++) d.push(`${j === a ? "M" : "L"}${points[j][0].toFixed(1)},${(-points[j][1]).toFixed(1)}`);
          return d.join(" ");
        })()
      : null;

    return html`<svg ref=${svgRef} className="track-map" viewBox=${geometry.viewBox} onMouseMove=${onMove} onMouseLeave=${() => onHover(null)} role="img" aria-label="Track map">
      <path d=${geometry.path} className="map-base" />
      ${activePath ? html`<path d=${activePath} className="map-active" />` : null}
      ${overlay || html`<path d=${geometry.path} className="map-line" />`}
      <line ...${startLine} className="map-start" />
      ${badges.map(
        (b) => html`<g
          key=${b.i}
          className=${"turn-badge" + (activeTurn === b.i ? " active" : "")}
          transform=${`translate(${b.x} ${b.y})`}
          onClick=${() => onTurnClick(b.i)}
        >
          <title>Turn ${b.label}</title>
          <circle r=${9 * s} />
          <text fontSize=${(b.label.length > 2 ? 7.5 : 9.5) * s} dy=${3.3 * s}>${b.label}</text>
        </g>`
      )}
      ${cursor ? html`<circle cx=${cursor[0]} cy=${-cursor[1]} r=${6 * s} className="map-cursor" />` : null}
    </svg>`;
  }

  // ---------- Corner table ----------

  function CornerTable({ corners, hasRef, activeTurn, onSelectTurn, editing, labels, setLabels }) {
    if (!corners.length) {
      return html`<p className="muted">No corners detected on this track map.</p>`;
    }
    const losses = corners
      .filter((c) => isNum(c.delta))
      .slice()
      .sort((a, b) => b.delta - a.delta);
    const worst = new Set(losses.filter((c) => c.delta > 0.02).slice(0, 3).map((c) => c.label));
    const maxAbs = Math.max(0.05, ...corners.map((c) => Math.abs(c.delta || 0)));
    const hasGrip = corners.some((c) => isNum(c.gripSel));
    const hasBal = corners.some((c) => isNum(c.balSel));
    const hasAbs = corners.some((c) => isNum(c.absSel));
    const cmp = (sel, ref, digits, unit) =>
      isNum(ref) ? `${fmtNum(sel, digits)}${unit} on this lap, ${fmtNum(ref, digits)}${unit} on the reference` : `${fmtNum(sel, digits)}${unit}`;

    return html`<div className="corner-wrap">
      <table className="corners">
        <thead>
          <tr>
            <th className="l">Turn</th>
            <th>Time</th>
            <th className="l bar-col"></th>
            <th>Min kph</th>
            ${hasRef ? html`<th>vs ref</th><th>Brake</th>` : null}
            ${hasGrip ? html`<th title="Average grip used while braking and cornering, as % of the session's peak">Grip</th>` : null}
            ${hasBal ? html`<th title="Mid-corner steering beyond what the car's rotation needed. + understeer, − oversteer. Compare laps, not corners.">Balance</th>` : null}
            ${hasAbs ? html`<th title="Share of braking with ABS active">ABS</th>` : null}
          </tr>
        </thead>
        <tbody>
          ${corners.map((c, i) => {
            const speedDiff = isNum(c.minRef) ? c.minSel - c.minRef : null;
            return html`<tr key=${i} className=${activeTurn === i ? "active" : ""} onClick=${() => onSelectTurn(activeTurn === i ? null : i)}>
              <td className="l">
                ${editing
                  ? html`<input
                      className="input turn-input mono"
                      value=${labels[i]}
                      onClick=${(e) => e.stopPropagation()}
                      onInput=${(e) => {
                        const next = labels.slice();
                        next[i] = e.target.value;
                        setLabels(next);
                      }}
                      aria-label=${"Label for turn " + (i + 1)}
                    />`
                  : html`<span className="turn-pill">${c.label}</span>${worst.has(c.label) ? html`<span className="tag tag-loss">lost</span>` : null}`}
              </td>
              <td className=${deltaClass(c.delta)}>${hasRef ? fmtDelta(c.delta) : "—"}</td>
              <td className="l bar-col">
                ${isNum(c.delta)
                  ? html`<div className="bar small">
                      ${Math.abs(c.delta) >= 0.0005
                        ? html`<span className=${c.delta < 0 ? "good" : "bad"} style=${{ width: `${(Math.abs(c.delta) / maxAbs) * 50}%` }}></span>`
                        : null}
                    </div>`
                  : null}
              </td>
              <td>${fmtNum(c.minSel, 0)}</td>
              ${hasRef
                ? html`<td className=${speedDiff === null ? "" : speedDiff > 0.5 ? "good" : speedDiff < -0.5 ? "bad" : "faint"}>
                      ${speedDiff === null ? "—" : (speedDiff > 0 ? "+" : speedDiff < 0 ? "−" : "±") + Math.abs(speedDiff).toFixed(0)}
                    </td>
                    <td className="faint" title="Brake point vs reference lap">
                      ${isNum(c.brakeLaterM) ? (Math.abs(c.brakeLaterM) < 3 ? "same" : `${Math.abs(c.brakeLaterM).toFixed(0)} m ${c.brakeLaterM > 0 ? "later" : "earlier"}`) : "—"}
                    </td>`
                : null}
              ${hasGrip
                ? html`<td className=${isNum(c.gripRef) ? (c.gripSel - c.gripRef >= 3 ? "good" : c.gripSel - c.gripRef <= -3 ? "bad" : "") : ""} title=${cmp(c.gripSel, c.gripRef, 0, "%")}>
                    ${isNum(c.gripSel) ? `${c.gripSel.toFixed(0)}%` : "—"}
                  </td>`
                : null}
              ${hasBal
                ? html`<td className="faint" title=${cmp(c.balSel, c.balRef, 1, "°")}>
                    ${isNum(c.balSel) ? html`${fmtSigned(c.balSel, 0)}°${isNum(c.balRef) ? html`<span className="bal-ref"> (${fmtSigned(c.balSel - c.balRef, 0)})</span>` : null}` : "—"}
                  </td>`
                : null}
              ${hasAbs ? html`<td className=${c.absSel >= 40 ? "bad" : "faint"}>${isNum(c.absSel) ? `${c.absSel.toFixed(0)}%` : "—"}</td>` : null}
            </tr>`;
          })}
        </tbody>
      </table>
    </div>`;
  }

  // ---------- Telemetry traces ----------

  const PANELS = [
    { key: "delta", label: "Delta", h: 84 },
    { key: "speed_kph", label: "Speed", unit: "kph", h: 130 },
    { key: "throttle", label: "Throttle", unit: "%", h: 64, fixed: [0, 100] },
    { key: "brake", label: "Brake", unit: "%", h: 64, fixed: [0, 100] },
    { key: "gear", label: "Gear", h: 56, step: true },
    { key: "steer_deg", label: "Steering", unit: "°", h: 72, symmetric: true },
    { key: "lat_g", label: "Lateral g", unit: "g", h: 64, symmetric: true, minSpan: 0.5, digits: 2 },
    { key: "long_g", label: "Long g", unit: "g", h: 64, symmetric: true, minSpan: 0.5, digits: 2 },
    { key: "balance_deg", label: "Balance · + under / − over", unit: "°", h: 72, symmetric: true, signed: true },
  ];

  function TraceChart({ sel, refTrace: ref, lengthM, turns, sectorPcts, cursorPct, onCursor, view, setView }) {
    const [wrapRef, width] = useElementWidth();
    const [drag, setDrag] = useState(null);
    const n = sel.time_s.length - 1;
    const pad = { l: 12, r: 12, top: 18, gap: 8 };
    const W = Math.max(320, width);
    const plotW = W - pad.l - pad.r;
    const [r0, r1] = view;
    const i0 = Math.max(0, Math.floor(r0 * n));
    const i1 = Math.min(n, Math.ceil(r1 * n));
    const step = Math.max(1, Math.floor((i1 - i0) / plotW));
    const x = (j) => pad.l + ((j / n - r0) / (r1 - r0)) * plotW;

    const delta = useMemo(() => (ref ? sel.time_s.map((t, j) => t - ref.time_s[j]) : null), [sel, ref]);
    const channel = (trace, key) => (key === "delta" ? (trace === sel ? delta : null) : trace && trace[key]);

    let y = pad.top;
    const layout = PANELS.filter((p) => p.key === "delta" || (sel[p.key] && sel[p.key].length)).map((p) => {
      const top = y;
      y += p.h + pad.gap;
      return { ...p, top };
    });
    const H = y;

    function domain(p) {
      if (p.fixed) return p.fixed;
      const values = [];
      for (const trace of [sel, ref]) {
        const arr = channel(trace, p.key);
        if (!arr) continue;
        for (let j = i0; j <= i1; j += step) values.push(arr[j]);
      }
      if (!values.length) return [-1, 1];
      let lo = Math.min(...values);
      let hi = Math.max(...values);
      if (p.key === "delta" || p.symmetric) {
        const m = Math.max(Math.abs(lo), Math.abs(hi), p.key === "delta" ? 0.05 : p.minSpan || 5);
        return [-m * 1.1, m * 1.1];
      }
      if (p.step) return [Math.min(0, lo) - 0.5, hi + 0.5];
      const padV = (hi - lo) * 0.08 || 1;
      return [lo - padV, hi + padV];
    }

    function line(arr, p, dom, stepwise) {
      const [lo, hi] = dom;
      const yv = (v) => p.top + p.h - ((v - lo) / (hi - lo)) * p.h;
      let d = "";
      for (let j = i0; j <= i1; j += step) {
        const px = x(j).toFixed(1);
        const py = yv(arr[j]).toFixed(1);
        if (!d) d = `M${px},${py}`;
        else if (stepwise) d += `H${px}V${py}`;
        else d += `L${px},${py}`;
      }
      return d;
    }

    /** Strip along the top of the brake panel wherever ABS was active on the selected lap. */
    function absMarks(abs, p) {
      let d = "";
      let start = null;
      for (let j = i0; j <= i1 + 1; j++) {
        const on = j <= i1 && abs[j];
        if (on && start === null) start = j;
        if (!on && start !== null) {
          const x0 = x(start);
          d += `M${x0.toFixed(1)},${p.top + 1}H${Math.max(x0 + 1.5, x(j - 1)).toFixed(1)}v4H${x0.toFixed(1)}Z`;
          start = null;
        }
      }
      return d;
    }

    const cursorIdx = isNum(cursorPct) ? idxOf(cursorPct, n) : null;
    const pctFromEvent = (e) => {
      const rect = e.currentTarget.getBoundingClientRect();
      const px = ((e.clientX - rect.left) / rect.width) * W;
      return Math.max(0, Math.min(1, r0 + ((px - pad.l) / plotW) * (r1 - r0)));
    };

    const visibleTurns = turns.filter((t) => t.pct >= r0 && t.pct <= r1);
    const fmtReadout = (p, v) => {
      if (!isNum(v)) return "—";
      if (p.key === "delta") return fmtDelta(v);
      if (p.key === "gear") return v === 0 ? "N" : v < 0 ? "R" : String(v);
      if (p.signed) return fmtSigned(v, 0) + p.unit;
      return v.toFixed(p.digits || 0) + (p.unit === "%" || p.unit === "g" ? p.unit : "");
    };

    return html`<div ref=${wrapRef} className="trace-wrap">
      <svg
        className="traces"
        width=${W}
        height=${H}
        viewBox=${`0 0 ${W} ${H}`}
        onMouseMove=${(e) => {
          const pct = pctFromEvent(e);
          onCursor(pct);
          if (drag) setDrag({ ...drag, to: pct });
        }}
        onMouseLeave=${() => {
          onCursor(null);
          setDrag(null);
        }}
        onMouseDown=${(e) => {
          const pct = pctFromEvent(e);
          setDrag({ from: pct, to: pct });
        }}
        onMouseUp=${() => {
          if (drag && Math.abs(drag.to - drag.from) * plotW / (r1 - r0) > 8) {
            setView([Math.min(drag.from, drag.to), Math.max(drag.from, drag.to)]);
          }
          setDrag(null);
        }}
        onDoubleClick=${() => setView([0, 1])}
      >
        <defs>
          ${layout.map(
            (p) => html`<clipPath key=${p.key} id=${"clip-" + p.key}>
              <rect x=${pad.l} y=${p.top} width=${plotW} height=${p.h} />
            </clipPath>`
          )}
        </defs>
        ${sectorPcts
          .filter((sp) => sp > r0 && sp < r1 && sp > 0)
          .map((sp, i) => html`<line key=${"s" + i} className="sector-line" x1=${x(sp * n)} x2=${x(sp * n)} y1=${pad.top} y2=${H - pad.gap} />`)}
        ${visibleTurns.map(
          (t) => html`<g key=${"t" + t.label + t.pct}>
            <line className="turn-line" x1=${x(t.pct * n)} x2=${x(t.pct * n)} y1=${pad.top - 2} y2=${H - pad.gap} />
            <text className="turn-label" x=${x(t.pct * n)} y=${11}>T${t.label}</text>
          </g>`
        )}
        ${layout.map((p) => {
          const dom = domain(p);
          const [lo, hi] = dom;
          const yv = (v) => p.top + p.h - ((v - lo) / (hi - lo)) * p.h;
          const selArr = channel(sel, p.key);
          const refArr = p.key === "delta" ? null : channel(ref, p.key);
          const zeroY = yv(0);
          return html`<g key=${p.key}>
            <rect className="panel-bg" x=${pad.l} y=${p.top} width=${plotW} height=${p.h} />
            ${(p.key === "delta" || p.symmetric) ? html`<line className="zero-line" x1=${pad.l} x2=${pad.l + plotW} y1=${zeroY} y2=${zeroY} />` : null}
            ${p.key === "delta" && !selArr
              ? html`<text className="panel-note" x=${pad.l + plotW / 2} y=${p.top + p.h / 2 + 4}>Pick a different lap to compare against</text>`
              : null}
            <g clipPath=${`url(#clip-${p.key})`}>
              ${p.key === "delta" && selArr
                ? html`<${React.Fragment}>
                    <clipPath id="clip-delta-behind"><rect x=${pad.l} y=${p.top} width=${plotW} height=${Math.max(0, zeroY - p.top)} /></clipPath>
                    <clipPath id="clip-delta-ahead"><rect x=${pad.l} y=${zeroY} width=${plotW} height=${Math.max(0, p.top + p.h - zeroY)} /></clipPath>
                    <path className="delta-area loss" clipPath="url(#clip-delta-behind)" d=${`${line(selArr, p, dom)}V${zeroY}H${x(i0)}Z`} />
                    <path className="delta-area gain" clipPath="url(#clip-delta-ahead)" d=${`${line(selArr, p, dom)}V${zeroY}H${x(i0)}Z`} />
                    <path className="trace-line delta" d=${line(selArr, p, dom)} />
                  <//>`
                : null}
              ${p.key === "brake" && sel.abs && sel.abs.length ? html`<path className="abs-marks" d=${absMarks(sel.abs, p)} />` : null}
              ${refArr ? html`<path className="trace-line ref" d=${line(refArr, p, dom, p.step)} />` : null}
              ${selArr && p.key !== "delta" ? html`<path className="trace-line sel" d=${line(selArr, p, dom, p.step)} />` : null}
            </g>
            <text className="panel-label" x=${pad.l + 6} y=${p.top + 13}>${p.key === "brake" && sel.abs && sel.abs.length ? "Brake · ABS marked" : p.label}</text>
            ${cursorIdx !== null
              ? html`<text className="panel-readout" x=${pad.l + plotW - 6} y=${p.top + 13}>
                  ${p.key === "delta"
                    ? html`<tspan className=${delta ? deltaClass(delta[cursorIdx]) + "-fill" : ""}>${delta ? fmtReadout(p, delta[cursorIdx]) : "—"}</tspan>`
                    : html`<tspan className="sel-fill">${fmtReadout(p, selArr && selArr[cursorIdx])}</tspan>${refArr
                        ? html`<tspan className="faint-fill"> / </tspan><tspan className="ref-fill">${fmtReadout(p, refArr[cursorIdx])}</tspan>`
                        : null}`}
                </text>`
              : null}
          </g>`;
        })}
        ${cursorIdx !== null && cursorIdx >= i0 && cursorIdx <= i1
          ? html`<line className="cursor-line" x1=${x(cursorIdx)} x2=${x(cursorIdx)} y1=${pad.top} y2=${H - pad.gap} />`
          : null}
        ${drag && drag.to !== drag.from
          ? html`<rect
              className="drag-rect"
              x=${Math.min(x(drag.from * n), x(drag.to * n))}
              y=${pad.top}
              width=${Math.abs(x(drag.to * n) - x(drag.from * n))}
              height=${H - pad.top - pad.gap}
            />`
          : null}
      </svg>
      <div className="trace-foot">
        <span>${Math.round(r0 * lengthM)} m – ${Math.round(r1 * lengthM)} m</span>
        <span className="faint">Drag to zoom · double-click to reset</span>
      </div>
    </div>`;
  }

  // ---------- Grip circle ----------

  /**
   * Friction circle (g-g plot) for the part of the lap in view: every point is one sample of
   * lateral vs longitudinal g. A lap that fills the circle out to the session peak, including
   * the diagonals (trail braking into the corner, throttle on the way out), is using the grip.
   */
  function GripCircle({ sel, refTrace: ref, view, peakG, cursorPct, onCursor, selected, refNumber }) {
    const svgRef = useRef(null);
    const n = sel.time_s.length - 1;
    const i0 = Math.max(0, Math.floor(view[0] * n));
    const i1 = Math.min(n, Math.ceil(view[1] * n));
    const step = Math.max(1, Math.floor((i1 - i0) / 900));
    const size = 260;
    const c = size / 2;
    const extent = useMemo(() => {
      let m = isNum(peakG) ? peakG * 1.12 : 1;
      for (const t of [sel, ref]) {
        if (!t) continue;
        for (let j = 0; j <= n; j += 4) m = Math.max(m, Math.abs(t.lat_g[j]), Math.abs(t.long_g[j]));
      }
      return Math.ceil(m * 2) / 2;
    }, [sel, ref, peakG]);
    const r = (c - 18) / extent;
    const px = (g) => c + g * r;
    const py = (g) => c - g * r; // acceleration up, braking down

    const dots = (t) => {
      let d = "";
      for (let j = i0; j <= i1; j += step) d += `M${px(t.lat_g[j]).toFixed(1)},${py(t.long_g[j]).toFixed(1)}h0`;
      return d;
    };
    const rings = [];
    for (let g = 1; g <= extent; g += 1) rings.push(g);

    function onMove(e) {
      const svg = svgRef.current;
      if (!svg) return;
      const rect = svg.getBoundingClientRect();
      const mx = ((e.clientX - rect.left) / rect.width) * size;
      const my = ((e.clientY - rect.top) / rect.height) * size;
      let best = null;
      let bestD = 144; // within 12px
      for (let j = i0; j <= i1; j += step) {
        const d = (px(sel.lat_g[j]) - mx) ** 2 + (py(sel.long_g[j]) - my) ** 2;
        if (d < bestD) {
          bestD = d;
          best = j;
        }
      }
      if (best !== null) onCursor(best / n);
    }

    const ci = isNum(cursorPct) ? idxOf(cursorPct, n) : null;
    const cur = ci !== null ? { lat: sel.lat_g[ci], long: sel.long_g[ci] } : null;
    const curRef = ci !== null && ref ? { lat: ref.lat_g[ci], long: ref.long_g[ci] } : null;
    const pctOfPeak = (p) => (isNum(peakG) ? ` · ${((Math.hypot(p.lat, p.long) / peakG) * 100).toFixed(0)}% of peak` : "");

    return html`<div className="grip-circle">
      <div className="section-label">Grip circle</div>
      <svg ref=${svgRef} viewBox=${`0 0 ${size} ${size}`} role="img" aria-label="Lateral versus longitudinal g" onMouseMove=${onMove} onMouseLeave=${() => onCursor(null)}>
        <line className="gg-axis" x1=${px(-extent)} x2=${px(extent)} y1=${c} y2=${c} />
        <line className="gg-axis" x1=${c} x2=${c} y1=${py(extent)} y2=${py(-extent)} />
        ${rings.map((g) => html`<g key=${g}>
          <circle className="gg-ring" cx=${c} cy=${c} r=${g * r} />
          <text className="gg-tick" x=${c + 3} y=${py(g) - 3}>${g}g</text>
        </g>`)}
        ${isNum(peakG) ? html`<circle className="gg-peak" cx=${c} cy=${c} r=${peakG * r}><title>Session peak ${peakG.toFixed(2)} g</title></circle>` : null}
        <text className="gg-edge" x=${c} y=${10}>Accel</text>
        <text className="gg-edge" x=${c} y=${size - 3}>Brake</text>
        ${ref && ref.lat_g ? html`<path className="gg-dots ref" d=${dots(ref)} />` : null}
        <path className="gg-dots sel" d=${dots(sel)} />
        ${curRef ? html`<circle className="gg-cursor ref" cx=${px(curRef.lat)} cy=${py(curRef.long)} r="4.5" />` : null}
        ${cur ? html`<circle className="gg-cursor sel" cx=${px(cur.lat)} cy=${py(cur.long)} r="4.5" />` : null}
      </svg>
      <div className="gg-readout mono">
        ${cur
          ? html`<div><span className="sel-fill">Lap ${selected}</span> ${fmtSigned(cur.lat, 2)} lat ${fmtSigned(cur.long, 2)} long${pctOfPeak(cur)}</div>
              ${curRef ? html`<div><span className="ref-fill">Lap ${refNumber}</span> ${fmtSigned(curRef.lat, 2)} lat ${fmtSigned(curRef.long, 2)} long${pctOfPeak(curRef)}</div>` : null}`
          : html`<div className="faint">${view[0] > 0 || view[1] < 1 ? "Showing the zoomed section" : "Showing the whole lap"} · hover for values</div>`}
      </div>
      <p className="faint gg-note">The dashed ring is the session's peak grip. Points filling toward it, including the diagonals (trail braking in, throttle out), mean the tyres are being used.</p>
    </div>`;
  }

  // ---------- Track section ----------

  function TrackSection({ split, laps, selected, refNumber, traceCache }) {
    const [map, setMap] = useState(null);
    const [mapError, setMapError] = useState(null);
    const [traces, setTraces] = useState({});
    const [mode, setMode] = useState(() => stored("pcc.mapMode", "delta"));
    const [cursorPct, setCursorPct] = useState(null);
    const [activeTurn, setActiveTurn] = useState(null);
    const [view, setView] = useState([0, 1]);
    const [editing, setEditing] = useState(false);
    const [labels, setLabels] = useState([]);
    const traced = new Set(split.traced_laps || []);

    useEffect(() => {
      if (!split.has_track) return;
      api(`/api/splits/${split.id}/track`).then(setMap).catch((err) => setMapError(err.message));
    }, [split.id]);

    const selTraced = traced.has(selected);
    const refTraced = refNumber !== null && refNumber !== selected && traced.has(refNumber);

    useEffect(() => {
      const wanted = [selTraced ? selected : null, refTraced ? refNumber : null].filter((v) => v !== null);
      wanted.forEach((lap) => {
        const key = `${split.id}:${lap}`;
        if (traceCache.current.has(key)) {
          setTraces((t) => (t[lap] ? t : { ...t, [lap]: traceCache.current.get(key) }));
          return;
        }
        api(`/api/splits/${split.id}/laps/${lap}/trace`)
          .then((trace) => {
            traceCache.current.set(key, trace);
            setTraces((t) => ({ ...t, [lap]: trace }));
          })
          .catch(() => {});
      });
    }, [split.id, selected, refNumber]);

    const sel = selTraced ? traces[selected] : null;
    const ref = refTraced ? traces[refNumber] : null;
    const segments = useMemo(() => (map ? turnSegments(map.turns) : []), [map]);
    const corners = useMemo(() => (map && sel ? cornerStats(segments, sel, ref, map.length_m, split.peak_g) : []), [segments, sel, ref, split.peak_g]);

    const selectTurn = (i) => {
      setActiveTurn(i);
      if (i === null) {
        setView([0, 1]);
      } else {
        const seg = segments[i];
        const margin = (seg.end - seg.start) * 0.15;
        setView([Math.max(0, seg.start - margin), Math.min(1, seg.end + margin)]);
      }
    };

    async function saveLabels() {
      try {
        const turns = map.turns.map((t, i) => ({ pct: t.pct, label: (labels[i] || "").trim() || t.label }));
        const updated = await api(`/api/splits/${split.id}/track/turns`, { method: "POST", body: JSON.stringify({ turns }) });
        setMap(updated);
        setEditing(false);
      } catch (err) {
        window.alert(err.message);
      }
    }

    if (!split.has_track) {
      if (split.source === "Live" || split.source.toLowerCase().endsWith(".ibt")) {
        return html`<section className="card"><p className="muted">No complete laps with telemetry in this run, so there's no track map yet.</p></section>`;
      }
      return null;
    }
    if (mapError) return html`<section className="card"><p className="bad">${mapError}</p></section>`;
    if (!map) return html`<section className="card"><p className="muted">Loading track…</p></section>`;

    const selLap = laps.find((l) => l.lap_number === selected);
    const refLap = laps.find((l) => l.lap_number === refNumber);
    const totalDelta = sel && ref ? sel.time_s[sel.time_s.length - 1] - ref.time_s[ref.time_s.length - 1] : null;

    return html`<${React.Fragment}>
      <section className="card">
        <div className="card-head">
          <div>
            <div className="card-title">Track</div>
            <div className="lap-vs">
              <span className="dot sel-bg"></span>Lap ${selected} ${selLap ? html`<span className="mono">${fmtLap(selLap.lap_time_s)}</span>` : null}
              ${ref
                ? html`<span className="faint">vs</span><span className="dot ref-bg"></span>Lap ${refNumber} <span className="mono">${fmtLap(refLap && refLap.lap_time_s)}</span>
                    <span className=${"delta-chip " + deltaClass(totalDelta)}>${fmtDelta(totalDelta)}</span>`
                : null}
            </div>
          </div>
          <div className="head-actions">
            <${Segmented}
              label="Map colouring"
              value=${mode}
              onChange=${(m) => {
                setMode(m);
                store("pcc.mapMode", m);
              }}
              options=${[
                { value: "delta", label: "Time gain/loss" },
                { value: "speed", label: "Speed" },
                { value: "inputs", label: "Inputs" },
              ]}
            />
          </div>
        </div>
        ${!sel
          ? html`<p className="muted">Lap ${selected} wasn't a clean timed lap, so it has no telemetry trace. Pick another lap.</p>`
          : html`<div className="track-grid">
              <div className="map-col">
                <${TrackMapView}
                  map=${map}
                  sel=${sel}
                  refTrace=${ref}
                  mode=${mode}
                  cursorPct=${cursorPct}
                  onHover=${setCursorPct}
                  activeTurn=${activeTurn}
                  onTurnClick=${(i) => selectTurn(activeTurn === i ? null : i)}
                />
                <div className="map-legend">
                  ${mode === "delta"
                    ? ref
                      ? html`<span><i className="good-bg"></i>Lap ${selected} faster</span><span><i className="bad-bg"></i>Lap ${selected} slower</span>`
                      : html`<span className="faint">Shift+click a lap to compare against</span>`
                    : mode === "speed"
                      ? html`<span className="speed-scale"></span><span className="faint">slow → fast</span>`
                      : html`<span><i style=${{ background: "#ff5f5f" }}></i>Braking</span><span><i style=${{ background: "#e8c547" }}></i>Part throttle</span><span><i style=${{ background: "#3ddc84" }}></i>Full throttle</span><span><i style=${{ background: "#6b7486" }}></i>Coast</span>`}
                </div>
                <p className="faint map-note">
                  ${(map.source === "gps" ? "Map from GPS telemetry. " : "Map reconstructed from heading data. ") +
                  "Turn numbers are auto-detected — rename them to match the official ones."}
                </p>
              </div>
              <div className="corner-col">
                <div className="corner-head">
                  <span className="section-label">Corner by corner</span>
                  ${editing
                    ? html`<span className="head-actions">
                        <button className="btn btn-ghost btn-sm" onClick=${() => setEditing(false)}>Cancel</button>
                        <button className="btn btn-primary btn-sm" onClick=${saveLabels}>Save names</button>
                      </span>`
                    : html`<button
                        className="btn btn-ghost btn-sm"
                        onClick=${() => {
                          setLabels(map.turns.map((t) => t.label));
                          setEditing(true);
                        }}
                      >Rename turns</button>`}
                </div>
                <${CornerTable}
                  corners=${corners}
                  hasRef=${!!ref}
                  activeTurn=${activeTurn}
                  onSelectTurn=${selectTurn}
                  editing=${editing}
                  labels=${labels}
                  setLabels=${setLabels}
                />
                ${!ref ? html`<p className="faint" style=${{ fontSize: "12px", marginTop: "8px" }}>Shift+click another lap to see where you gain and lose time.</p>` : null}
              </div>
            </div>`}
      </section>

      ${sel
        ? html`<section className="card">
            <div className="card-head">
              <div className="card-title">Telemetry</div>
              <div className="head-actions">
                <span className="legend">
                  <span><i className="sel-bg"></i>Lap ${selected}</span>
                  ${ref ? html`<span><i className="ref-bg"></i>Lap ${refNumber}</span>` : null}
                </span>
                ${view[0] > 0 || view[1] < 1
                  ? html`<button className="btn btn-ghost btn-sm" onClick=${() => selectTurn(null)}>Reset zoom</button>`
                  : null}
              </div>
            </div>
            <div className=${sel.lat_g && sel.lat_g.length ? "telemetry-grid" : ""}>
              <${TraceChart}
                sel=${sel}
                refTrace=${ref}
                lengthM=${map.length_m}
                turns=${map.turns}
                sectorPcts=${map.sector_pcts || []}
                cursorPct=${cursorPct}
                onCursor=${setCursorPct}
                view=${view}
                setView=${setView}
              />
              ${sel.lat_g && sel.lat_g.length
                ? html`<${GripCircle}
                    sel=${sel}
                    refTrace=${ref && ref.lat_g && ref.lat_g.length ? ref : null}
                    view=${view}
                    peakG=${split.peak_g}
                    cursorPct=${cursorPct}
                    onCursor=${setCursorPct}
                    selected=${selected}
                    refNumber=${refNumber}
                  />`
                : null}
            </div>
          </section>`
        : null}
    <//>`;
  }

  // ---------- Lap table ----------

  function LapTable({ laps, stats, excluded, selected, compare, onPick, onToggle }) {
    const nSectors = stats ? stats.nSectors : 0;
    const showTyres = laps.some((lap) => isNum(lap.tyre_temp_avg_c));
    const bestTime = stats ? stats.best.lap_time_s : null;

    return html`<div className="table-wrap">
      <table className="laps">
        <thead>
          <tr>
            <th title="Count this lap in stats">Use</th>
            <th className="l">Lap</th>
            <th>Time</th>
            <th>Δ Best</th>
            ${range(nSectors).map((i) => html`<th key=${i}>S${i + 1}</th>`)}
            <th>Avg kph</th>
            ${showTyres ? html`<th>Tyre °C</th><th>Spread</th>` : null}
          </tr>
        </thead>
        <tbody>
          ${laps.map((lap) => {
            const isExcluded = !lap.is_complete || excluded.has(lap.lap_number);
            const isBest = stats && lap.lap_number === stats.best.lap_number;
            const delta = lap.is_complete && isNum(bestTime) ? lap.lap_time_s - bestTime : null;
            const cls = [
              isExcluded ? "excluded" : "",
              lap.lap_number === compare ? "compare" : "",
              lap.lap_number === selected ? "selected" : "",
            ].join(" ");
            return html`<tr key=${lap.lap_number} className=${cls} onClick=${(e) => onPick(lap.lap_number, e.shiftKey)}>
              <td onClick=${(e) => e.stopPropagation()}>
                <input
                  type="checkbox"
                  className="check"
                  checked=${!isExcluded}
                  disabled=${!lap.is_complete}
                  onChange=${() => onToggle(lap.lap_number)}
                  aria-label=${"Count lap " + lap.lap_number + " in stats"}
                />
              </td>
              <td className="l">
                ${lap.lap_number} ${isBest ? html`<span className="tag tag-best">best</span>` : null}
                ${!lap.is_complete ? html`<span className="tag tag-out">untimed</span>` : null}
              </td>
              <td className=${isBest ? "time-best" : ""}>${fmtLap(lap.lap_time_s)}</td>
              <td className=${isBest ? "purple" : deltaClass(delta)}>${isBest ? "—" : fmtDelta(delta)}</td>
              ${range(nSectors).map((i) => {
                const v = sectorOf(lap, i);
                const isSectorBest = !isExcluded && isNum(v) && v === stats.bestSectors[i];
                return html`<td key=${i} className=${isSectorBest ? "sector-best" : ""}>${fmtNum(v, 3)}</td>`;
              })}
              <td>${fmtNum(lap.avg_speed_kph, 1)}</td>
              ${showTyres ? html`<td>${fmtNum(lap.tyre_temp_avg_c, 1)}</td><td>${fmtNum(lap.tyre_temp_delta_c, 1)}</td>` : null}
            </tr>`;
          })}
        </tbody>
      </table>
    </div>`;
  }

  // ---------- Lap detail ----------

  function Metric({ label, value, delta, unit, digits = 1, higherIsBetter }) {
    const hasDelta = isNum(delta) && Math.abs(delta) >= 0.05;
    const better = hasDelta && (higherIsBetter ? delta > 0 : delta < 0);
    return html`<div className="metric">
      <div className="k">${label}</div>
      <div className="v">${fmtNum(value, digits, unit)}</div>
      <div className=${"dv " + (hasDelta ? (better ? "good" : "bad") : "faint")}>
        ${hasDelta ? (delta > 0 ? "+" : "−") + Math.abs(delta).toFixed(digits) : " "}
      </div>
    </div>`;
  }

  function LapDetail({ laps, insights, stats, selected, compare, refNumber, autoRef, setCompare }) {
    const lap = laps.find((l) => l.lap_number === selected);
    if (!lap) {
      return html`<section className="card"><p className="muted">Pick a lap to see the breakdown.</p></section>`;
    }
    const ref = laps.find((l) => l.lap_number === refNumber);
    const autoLap = laps.find((l) => l.lap_number === autoRef);
    const isSelf = ref && ref.lap_number === lap.lap_number;
    const delta = ref && lap.is_complete && ref.is_complete ? lap.lap_time_s - ref.lap_time_s : null;
    const insight = insights[lap.lap_number];

    const nSectors = Math.max((lap.sectors || []).length, ref ? (ref.sectors || []).length : 0);
    const sectorDeltas = range(nSectors).map((i) => {
      const a = sectorOf(lap, i);
      const b = ref ? sectorOf(ref, i) : null;
      return { i: i + 1, a, d: isNum(a) && isNum(b) ? a - b : null };
    });
    const hasSectorDeltas = !isSelf && sectorDeltas.some((s) => s.d !== null);
    const maxAbs = Math.max(0.05, ...sectorDeltas.map((s) => Math.abs(s.d || 0)));
    const biggestLoss = hasSectorDeltas
      ? sectorDeltas.reduce((a, b) => ((b.d ?? -Infinity) > (a.d ?? -Infinity) ? b : a))
      : null;

    const diff = (key) => (ref && !isSelf && isNum(lap[key]) && isNum(ref[key]) ? lap[key] - ref[key] : null);

    return html`<section className="card detail">
      <div className="detail-hero">
        <div>
          <div className="kpi-label">Lap ${lap.lap_number} ${!lap.is_complete ? "· untimed" : ""}</div>
          <div className="detail-time">${fmtLap(lap.lap_time_s)}</div>
        </div>
        ${!isSelf && delta !== null
          ? html`<span className=${"delta-chip " + deltaClass(delta)}>${fmtDelta(delta)}</span>`
          : isSelf
            ? html`<span className="tag tag-best">reference</span>`
            : null}
      </div>

      <div className="vs-row">
        <span className="vs-dot" style=${{ background: "var(--compare)" }}></span>
        <span className="muted" style=${{ fontSize: "13px" }}>vs</span>
        <select
          className="select"
          value=${compare === null ? "best" : String(compare)}
          onChange=${(e) => setCompare(e.target.value === "best" ? null : Number(e.target.value))}
          aria-label="Compare against"
        >
          <option value="best">
            ${autoLap
              ? `${stats && autoLap.lap_number === stats.best.lap_number ? "Best" : "Next best"} (L${autoLap.lap_number} · ${fmtLap(autoLap.lap_time_s)})`
              : "Best lap"}
          </option>
          ${laps
            .filter((l) => l.is_complete)
            .map((l) => html`<option key=${l.lap_number} value=${l.lap_number}>Lap ${l.lap_number} · ${fmtLap(l.lap_time_s)}</option>`)}
        </select>
      </div>

      ${hasSectorDeltas
        ? html`<div className="sector-rows">
            ${sectorDeltas.map(
              (s) => html`<div key=${s.i} className="sector-row">
                <span className="name">S${s.i}</span>
                <div className="bar">
                  ${s.d !== null && Math.abs(s.d) >= 0.0005
                    ? html`<span className=${s.d < 0 ? "good" : "bad"} style=${{ width: `${(Math.abs(s.d) / maxAbs) * 50}%` }}></span>`
                    : null}
                </div>
                <span className=${"val " + deltaClass(s.d)}>${fmtDelta(s.d)}</span>
              </div>`
            )}
            ${biggestLoss && biggestLoss.d > 0.01
              ? html`<p className="muted" style=${{ fontSize: "13px" }}>
                  Biggest loss in <b style=${{ color: "var(--text)" }}>sector ${biggestLoss.i}</b> (${fmtDelta(biggestLoss.d)}).
                </p>`
              : null}
          </div>`
        : !isSelf && !(stats && stats.nSectors)
          ? html`<p className="faint" style=${{ fontSize: "12px" }}>No sector splits in this data source.</p>`
          : null}

      <div className="metric-grid">
        <${Metric} label="Avg speed" value=${lap.avg_speed_kph} delta=${diff("avg_speed_kph")} higherIsBetter=${true} />
        <${Metric} label="Tyre avg" value=${lap.tyre_temp_avg_c} delta=${diff("tyre_temp_avg_c")} unit="°" />
        <${Metric} label="Tyre spread" value=${lap.tyre_temp_delta_c} delta=${diff("tyre_temp_delta_c")} unit="°" />
      </div>

      ${insight
        ? html`<div className="notes">
            <div>
              <div className="section-label" style=${{ marginBottom: "6px" }}>Went well</div>
              <ul className="note-list good">${insight.went_well.map((t, i) => html`<li key=${i}>${t}</li>`)}</ul>
            </div>
            <div>
              <div className="section-label" style=${{ marginBottom: "6px" }}>To work on</div>
              <ul className="note-list bad">${insight.went_bad.map((t, i) => html`<li key=${i}>${t}</li>`)}</ul>
            </div>
          </div>`
        : null}
    </section>`;
  }

  // ---------- Run view ----------

  function RunView({ split, laps }) {
    const [excluded, setExcluded] = useState(() => new Set());
    const [selected, setSelected] = useState(null);
    const [compare, setCompare] = useState(null);
    const traceCache = useRef(new Map());

    // Start on the most recent timed lap.
    useEffect(() => {
      const complete = laps.filter((lap) => lap.is_complete);
      const last = complete.length ? complete[complete.length - 1] : laps[laps.length - 1];
      setSelected(last ? last.lap_number : null);
    }, [split.id]);

    const stats = useMemo(() => computeStats(laps, excluded), [laps, excluded]);
    const insights = useMemo(() => Object.fromEntries(laps.map((lap) => [lap.lap_number, lap])), [laps]);

    // Reference lap: the one picked with Shift+click, otherwise your best — or your
    // next-best when the best lap is the one selected.
    const autoRef = useMemo(() => {
      if (!stats) return null;
      if (stats.best.lap_number !== selected) return stats.best.lap_number;
      const others = stats.counted.filter((l) => l.lap_number !== selected);
      return others.length ? others.reduce((a, b) => (b.lap_time_s < a.lap_time_s ? b : a)).lap_number : null;
    }, [stats, selected]);
    const refNumber = compare !== null ? compare : autoRef;

    const pick = useCallback((lapNumber, asCompare) => {
      if (asCompare) {
        setCompare((current) => (current === lapNumber ? null : lapNumber));
      } else {
        setSelected(lapNumber);
      }
    }, []);

    const toggle = useCallback((lapNumber) => {
      setExcluded((current) => {
        const next = new Set(current);
        if (next.has(lapNumber)) next.delete(lapNumber);
        else next.add(lapNumber);
        return next;
      });
    }, []);

    // Arrow keys step through laps, B jumps to the best lap.
    useEffect(() => {
      function onKey(e) {
        if (e.target instanceof HTMLInputElement || e.target instanceof HTMLSelectElement) return;
        const idx = laps.findIndex((lap) => lap.lap_number === selected);
        if (e.key === "ArrowRight") {
          e.preventDefault();
          if (idx < laps.length - 1) setSelected(laps[idx + 1].lap_number);
        } else if (e.key === "ArrowLeft") {
          e.preventDefault();
          if (idx > 0) setSelected(laps[idx - 1].lap_number);
        } else if ((e.key === "b" || e.key === "B") && stats) {
          setSelected(stats.best.lap_number);
        }
      }
      window.addEventListener("keydown", onKey);
      return () => window.removeEventListener("keydown", onKey);
    }, [laps, selected, stats]);

    const subtitle = [
      split.car,
      fmtTimeOfDay(split.started_at_ms) + (split.source === "Live" ? " – " + fmtTimeOfDay(split.ended_at_ms) : ""),
      `${laps.filter((l) => l.is_complete).length} timed laps`,
      excluded.size ? `${excluded.size} excluded from stats` : null,
    ]
      .filter(Boolean)
      .join(" · ");

    return html`<${React.Fragment}>
      <div className="page-head">
        <div>
          <h1>${runTitle(split)}</h1>
          <p>${subtitle}</p>
        </div>
        <div className="legend">
          <span><i style=${{ background: "var(--purple)" }}></i>Session best</span>
          <span><i style=${{ background: "var(--accent)" }}></i>Selected</span>
          <span><i style=${{ background: "var(--compare)" }}></i>Compare</span>
        </div>
      </div>

      ${stats
        ? html`<div className="kpis">
            <${Kpi} highlight=${true} label="Best lap" value=${fmtLap(stats.best.lap_time_s)} sub=${`Lap ${stats.best.lap_number}`} />
            ${stats.optimal !== null
              ? html`<${Kpi}
                  label="Optimal lap"
                  value=${fmtLap(stats.optimal)}
                  sub=${`${(stats.best.lap_time_s - stats.optimal).toFixed(3)}s left on the table`}
                  title="Sum of your best sectors"
                />`
              : html`<${Kpi}
                  label="Best 3-lap avg"
                  value=${stats.bestRun ? fmtLap(stats.bestRun.avg) : "—"}
                  sub=${stats.bestRun ? `Laps ${stats.bestRun.from}–${stats.bestRun.to}` : "Needs 3 laps"}
                  title="Your quickest three consecutive counted laps"
                />`}
            <${Kpi} label="Average" value=${fmtLap(stats.avg)} sub=${`${fmtDelta(stats.avg - stats.best.lap_time_s)} to best`} />
            <${Kpi}
              label="Consistency"
              value=${stats.sd !== null ? "±" + stats.sd.toFixed(3) : "—"}
              sub=${`${stats.nearBest}/${stats.counted.length} laps within 0.5s`}
              title="Standard deviation of counted lap times"
            />
            <${Kpi}
              label="Trend"
              value=${html`<span className=${deltaClass(stats.trend)}>${stats.trend !== null ? fmtDelta(stats.trend) : "—"}</span>`}
              sub=${stats.trend === null ? "Needs 4 laps" : stats.trend < -0.05 ? "Getting quicker" : stats.trend > 0.05 ? "Dropping off" : "Holding steady"}
              title="Average of your last laps vs your first laps"
            />
            ${stats.focus
              ? html`<${Kpi}
                  label="Focus"
                  value=${"Sector " + stats.focus.sector}
                  sub=${`avg ${stats.focus.loss.toFixed(3)}s off your best`}
                  title="The sector where a typical lap loses the most vs your best time in that sector"
                />`
              : html`<${Kpi} label="Avg speed" value=${fmtNum(stats.avgSpeed, 1)} sub="kph, counted laps" />`}
          </div>`
        : html`<section className="card"><p className="muted">No counted laps — tick at least one lap in the table to see stats.</p></section>`}

      <section className="card">
        <div className="card-head">
          <div className="card-title">Pick a lap</div>
          <div className="legend">
            <span>Click to select · <span className="kbd">Shift</span>+click to compare</span>
            <span><span className="kbd">←</span> <span className="kbd">→</span> step · <span className="kbd">B</span> best</span>
          </div>
        </div>
        <div className="lap-strip">
          ${laps.map((lap) => {
            const isBest = stats && lap.lap_number === stats.best.lap_number;
            const d = stats && lap.is_complete ? lap.lap_time_s - stats.best.lap_time_s : null;
            const cls = [
              "lap-chip",
              isBest ? "is-best" : "",
              !lap.is_complete || excluded.has(lap.lap_number) ? "excluded" : "",
              lap.lap_number === compare ? "compare" : "",
              lap.lap_number === selected ? "selected" : "",
            ].join(" ");
            return html`<button key=${lap.lap_number} className=${cls} onClick=${(e) => pick(lap.lap_number, e.shiftKey)}>
              <span className="n">LAP ${lap.lap_number}</span>
              <span className="t">${fmtLap(lap.lap_time_s)}</span>
              <span className=${"d " + (isBest ? "purple" : deltaClass(d))}>${isBest ? "best" : !lap.is_complete ? "untimed" : fmtDelta(d)}</span>
            </button>`;
          })}
        </div>
        <div style=${{ marginTop: "14px" }}>
          <${PaceChart} laps=${laps} stats=${stats} excluded=${excluded} selected=${selected} compare=${compare} onPick=${pick} />
        </div>
      </section>

      <${TrackSection} split=${split} laps=${laps} selected=${selected} refNumber=${refNumber} traceCache=${traceCache} />

      <div className="split-2">
        <section className="card">
          <div className="card-head">
            <div className="card-title">Lap times</div>
            <span className="faint" style=${{ fontSize: "12px" }}>Untick out laps or spins to leave them out of the stats</span>
          </div>
          <${LapTable} laps=${laps} stats=${stats} excluded=${excluded} selected=${selected} compare=${compare} onPick=${pick} onToggle=${toggle} />
        </section>
        <${LapDetail}
          laps=${laps}
          insights=${insights}
          stats=${stats}
          selected=${selected}
          compare=${compare}
          refNumber=${refNumber}
          autoRef=${autoRef}
          setCompare=${setCompare}
        />
      </div>

      <div className="coach">
        <section className="card">
          <div className="card-head"><div className="card-title">Next time out</div></div>
          <ol className="suggestions">
            ${split.suggestions.map((t, i) => html`<li key=${i}><b>${i + 1}</b><span>${t}</span></li>`)}
          </ol>
        </section>
        <section className="card">
          <div className="card-head">
            <div className="card-title">Coach feedback</div>
            <span className="tag">${split.model}</span>
          </div>
          <p className="feedback">${split.feedback}</p>
        </section>
      </div>
    <//>`;
  }

  // ---------- Empty state ----------

  function EmptyState({ onStart, onOpen, busy }) {
    return html`<section className="empty">
      <div className="empty-icon">🏁</div>
      <h2>Ready when you are</h2>
      <p>Record a stint live, or open an iRacing telemetry file to get lap times, a track map and corner-by-corner comparisons.</p>
      <div className="steps">
        <div className="step"><b>1</b>Get on track in iRacing</div>
        <div className="step"><b>2</b>Start recording</div>
        <div className="step"><b>3</b>Stop & review</div>
      </div>
      <div style=${{ display: "flex", gap: "8px", marginTop: "6px" }}>
        <button className="btn btn-record" onClick=${onStart} disabled=${busy}><span className="rec-dot"></span> Start recording</button>
        <button className="btn" onClick=${onOpen} disabled=${busy}>Open session file</button>
      </div>
    </section>`;
  }

  // ---------- App ----------

  function App() {
    const [status, setStatus] = useState({ is_recording: false, split_count: 0 });
    const [splits, setSplits] = useState([]);
    const [selectedId, setSelectedId] = useState(null);
    const [split, setSplit] = useState(null);
    const [laps, setLaps] = useState([]);
    const [live, setLive] = useState(null);
    const [model, setModelState] = useState(() => stored("pcc.model", ""));
    const [busy, setBusy] = useState(false);
    const [importOpen, setImportOpen] = useState(false);
    const [toast, setToast] = useState(null);
    const selectedIdRef = useRef(selectedId);
    selectedIdRef.current = selectedId;

    const notify = (kind, text) => setToast({ kind, text });
    const setModel = (value) => {
      setModelState(value);
      store("pcc.model", value);
    };

    async function refresh() {
      const [newStatus, newSplits, newLive] = await Promise.all([
        api("/api/status"),
        api("/api/splits"),
        api("/api/recording/live"),
      ]);
      setStatus(newStatus);
      setSplits(newSplits || []);
      setLive(newLive);
      if (!model && newStatus.default_model) setModelState(newStatus.default_model);
      if (!selectedIdRef.current && newSplits && newSplits.length) {
        setSelectedId(newSplits[newSplits.length - 1].id);
      }
    }

    useEffect(() => {
      refresh().catch((err) => notify("error", err.message));
    }, []);

    // Poll live laps while recording.
    useEffect(() => {
      if (!status.is_recording) return undefined;
      const id = setInterval(() => {
        api("/api/recording/live").then(setLive).catch(() => {});
      }, 1500);
      return () => clearInterval(id);
    }, [status.is_recording]);

    useEffect(() => {
      if (!selectedId) {
        setSplit(null);
        setLaps([]);
        return undefined;
      }
      let cancelled = false;
      Promise.all([api(`/api/splits/${selectedId}`), api(`/api/splits/${selectedId}/laps`)])
        .then(([detail, lapList]) => {
          if (cancelled) return;
          setLaps(lapList || []);
          setSplit(detail);
        })
        .catch((err) => notify("error", err.message));
      return () => {
        cancelled = true;
      };
    }, [selectedId]);

    async function onStart() {
      setBusy(true);
      try {
        await api("/api/recording/start", { method: "POST", body: JSON.stringify({ model: model || null }) });
        await refresh();
        notify("ok", "Recording — go drive. Laps show up live as you cross the line.");
      } catch (err) {
        notify("error", err.message);
      } finally {
        setBusy(false);
      }
    }

    async function onStop() {
      setBusy(true);
      try {
        const result = await api("/api/recording/stop", { method: "POST" });
        await refresh();
        setSelectedId(result.id);
        notify("ok", "Run saved and analyzed.");
      } catch (err) {
        await refresh().catch(() => {});
        notify("error", err.message);
      } finally {
        setBusy(false);
      }
    }

    async function onImport(path) {
      setBusy(true);
      try {
        const result = await api("/api/splits/import", {
          method: "POST",
          body: JSON.stringify({ path, model: model || null }),
        });
        await refresh();
        setSelectedId(result.id);
        setImportOpen(false);
        notify("ok", `Opened ${result.lap_count} laps${result.track_label ? " at " + result.track_label : ""}.`);
      } catch (err) {
        notify("error", err.message);
      } finally {
        setBusy(false);
      }
    }

    async function onDelete(id) {
      if (!window.confirm("Delete this run? This can't be undone.")) return;
      try {
        await api(`/api/splits/${id}`, { method: "DELETE" });
        if (selectedId === id) setSelectedId(null);
        await refresh();
      } catch (err) {
        notify("error", err.message);
      }
    }

    const showRun = split && split.id === selectedId;

    return html`<div className="shell">
      <${TopBar}
        isRecording=${status.is_recording}
        recordingStartedAt=${live && live.started_at_ms}
        replayFile=${status.replay_file}
        busy=${busy}
        model=${model}
        setModel=${setModel}
        onStart=${onStart}
        onStop=${onStop}
        onImport=${() => setImportOpen(true)}
      />
      <div className="body">
        <${Sidebar} splits=${splits} selectedId=${selectedId} onSelect=${setSelectedId} onDelete=${onDelete} />
        <main className="main">
          ${status.is_recording ? html`<${LivePanel} live=${live} />` : null}
          ${showRun
            ? html`<${RunView} key=${split.id} split=${split} laps=${laps} />`
            : splits.length === 0 && !status.is_recording
              ? html`<${EmptyState} onStart=${onStart} onOpen=${() => setImportOpen(true)} busy=${busy} />`
              : null}
        </main>
      </div>
      ${importOpen ? html`<${ImportModal} busy=${busy} onCancel=${() => setImportOpen(false)} onImport=${onImport} />` : null}
      <${Toast} toast=${toast} onClose=${() => setToast(null)} />
    </div>`;
  }

  ReactDOM.createRoot(document.getElementById("root")).render(html`<${App} />`);
})();
