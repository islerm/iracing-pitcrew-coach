(function () {
  const { useCallback, useContext, useEffect, useMemo, useRef, useState } = React;
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

  // Suggested coach models, smallest first. Sizes are Ollama's download sizes; the model also
  // needs about that much free GPU memory (or it falls back to the much slower CPU).
  const RECOMMENDED_MODELS = [
    { name: "llama3.2", size: "2.0 GB", note: "Fastest, plainest feedback" },
    { name: "qwen3.5:4b", size: "3.4 GB", note: "Small and quick" },
    { name: "qwen3.5:9b", size: "6.6 GB", note: "Balance of detail and speed", pick: true },
    { name: "gemma4:12b", size: "8.0 GB", note: "Most detailed · needs a 12 GB+ GPU" },
  ];

  /** Ollama names without a tag mean ":latest". */
  const modelKey = (name) => (name.includes(":") ? name : name + ":latest").toLowerCase();
  const shortName = (name) => name.replace(/:latest$/, "");

  function ModelPicker({ model, setModel, disabled }) {
    const [list, setList] = useState(null);
    const [loading, setLoading] = useState(false);
    const [copied, setCopied] = useState(null);
    const [custom, setCustom] = useState(false);
    const load = () => {
      setLoading(true);
      api("/api/models")
        .then(setList)
        .catch((err) => setList({ ollama: true, installed: [], error: err.message }))
        .finally(() => setLoading(false));
    };
    useEffect(load, []);

    const installed = list ? list.installed : [];
    const have = new Set(installed.map((m) => modelKey(m.name)));
    const toGet = RECOMMENDED_MODELS.filter((r) => !have.has(modelKey(r.name)));
    const current = modelKey(model || "");
    const noteFor = (m) => (RECOMMENDED_MODELS.find((r) => modelKey(r.name) === modelKey(m.name)) || {}).note;
    const copy = (text) => {
      try {
        navigator.clipboard.writeText(text);
        setCopied(text);
        setTimeout(() => setCopied(null), 1500);
      } catch (_err) {
        /* clipboard unavailable: the command is in the button's tooltip */
      }
    };

    return html`<div className="model-picker">
      <div className="model-picker-head">
        <span className="model-picker-title">Coach model</span>
        <button className="btn btn-ghost btn-icon btn-sm" onClick=${load} disabled=${loading} title="Check for newly downloaded models" aria-label="Refresh model list">
          ${loading ? html`<span className="spinner"></span>` : "↻"}
        </button>
      </div>

      ${list && !list.ollama
        ? html`<p className="model-warn">Ollama isn't installed or isn't on PATH. Install it from ollama.com to get written feedback.</p>`
        : list && list.error
          ? html`<p className="model-warn">${list.error}</p>`
          : null}

      ${!list
        ? html`<p className="faint model-help"><span className="spinner inline"></span>Checking installed models…</p>`
        : installed.length
          ? html`<div className="model-options" role="radiogroup" aria-label="Coach model">
              ${installed.map((m) => {
                const active = current === modelKey(m.name);
                const note = noteFor(m);
                return html`<button
                  key=${m.name}
                  role="radio"
                  aria-checked=${active}
                  className=${"model-option" + (active ? " active" : "")}
                  disabled=${disabled}
                  onClick=${() => setModel(shortName(m.name))}
                >
                  <span className="model-dot"></span>
                  <span className="model-text">
                    <span className="model-name">${shortName(m.name)}</span>
                    ${note ? html`<span className="model-note">${note}</span>` : null}
                  </span>
                  <span className="model-size">${m.size}</span>
                </button>`;
              })}
            </div>`
          : list.ollama
            ? html`<p className="faint model-help">No models downloaded yet. Pick one below to get started.</p>`
            : null}

      ${list && list.ollama && toGet.length
        ? html`<div className="model-get">
            <div className="model-get-title">Download more</div>
            ${toGet.map((r) => {
              const cmd = `ollama pull ${r.name}`;
              return html`<div key=${r.name} className="model-get-row">
                <span className="model-text">
                  <span className="model-name">${r.name}${r.pick ? html`<span className="model-rec">Recommended</span>` : null}</span>
                  <span className="model-note">${r.note} · ${r.size}</span>
                </span>
                <button className=${"btn btn-sm model-copy" + (copied === cmd ? " done" : "")} onClick=${() => copy(cmd)} title=${cmd}>
                  ${copied === cmd ? "✓ Copied" : "Copy command"}
                </button>
              </div>`;
            })}
            <p className="faint model-help">Paste the command into a terminal, then press ↻ once it finishes.</p>
          </div>`
        : null}

      ${custom
        ? html`<div className="field">
            <label htmlFor="model">Any Ollama model name</label>
            <input id="model" className="input mono" value=${model} disabled=${disabled} onInput=${(e) => setModel(e.target.value)} placeholder="llama3.2" autoFocus />
          </div>`
        : html`<button className="model-link" onClick=${() => setCustom(true)}>Use a different model…</button>`}
    </div>`;
  }

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
              <${ModelPicker} model=${model} setModel=${setModel} disabled=${isRecording} />
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
                <span className="n">L${lap.lap_number} ${!lap.is_complete ? html`<span className="tag tag-out">untimed</span>` : null} <${OffTrackMark} lap=${lap} /></span>
                <span className="t">${fmtLap(lap.lap_time_s)}</span>
                <span className=${"d " + (isBest ? "purple" : deltaClass(d))}>${isBest ? "best" : d !== null ? fmtDelta(d) : " "}</span>
              </div>`;
            })}
          </div>`}
    </section>`;
  }

  // ---------- Off track ----------

  const offTracks = (lap) => (lap && lap.off_track_pcts ? lap.off_track_pcts.length : 0);

  function offTrackTitle(lap) {
    const n = offTracks(lap);
    if (!n) return "";
    const inc = isNum(lap.incidents) && lap.incidents > 0 ? `, ${lap.incidents}x incident${lap.incidents === 1 ? "" : "s"}` : "";
    return `Went off track ${n === 1 ? "once" : n + " times"}${inc}`;
  }

  function OffTrackMark({ lap }) {
    if (!offTracks(lap)) return null;
    const n = offTracks(lap);
    return html`<span className="off-mark" title=${offTrackTitle(lap)} aria-label=${offTrackTitle(lap)}>!${n > 1 ? html`<small>${n}</small>` : null}</span>`;
  }

  // ---------- Lap bar (global lap selection) ----------

  /**
   * The one place laps are chosen. Everything below (map, corners, telemetry, grip circle,
   * lap detail) shows the analysed lap against the comparison lap picked here.
   */
  function LapBar({ laps, stats, excluded, selected, compare, refNumber, autoRef, setSelected, setCompare, onPick }) {
    const stripRef = useRef(null);
    const timed = laps.filter((l) => l.is_complete);
    const selLap = laps.find((l) => l.lap_number === selected);
    const refLap = laps.find((l) => l.lap_number === refNumber);
    const delta = selLap && refLap && selLap.is_complete && refLap.is_complete ? selLap.lap_time_s - refLap.lap_time_s : null;
    const idx = laps.findIndex((l) => l.lap_number === selected);
    const autoLap = laps.find((l) => l.lap_number === autoRef);
    const lapOption = (l) => `Lap ${l.lap_number} · ${fmtLap(l.lap_time_s)}${!l.is_complete ? " (untimed)" : ""}${offTracks(l) ? " ⚠ off track" : ""}`;

    // Sticky panels below (map, lap detail) sit just under the bar, whatever height it wraps to.
    const barRef = useRef(null);
    useEffect(() => {
      const bar = barRef.current;
      if (!bar) return undefined;
      const root = document.documentElement.style;
      const update = () => root.setProperty("--lapbar-h", `${Math.ceil(bar.getBoundingClientRect().height)}px`);
      update();
      const observer = new ResizeObserver(update);
      observer.observe(bar);
      return () => {
        observer.disconnect();
        root.removeProperty("--lapbar-h");
      };
    }, []);

    // Keep the analysed lap in view in the strip.
    useEffect(() => {
      const strip = stripRef.current;
      const chip = strip && strip.querySelector(".lap-chip.selected");
      if (chip) chip.scrollIntoView({ block: "nearest", inline: "nearest" });
    }, [selected]);

    return html`<div className="lap-bar" ref=${barRef}>
      <div className="lap-bar-row">
        <div className="lap-slot sel">
          <span className="slot-label"><i className="sel-bg"></i>Analysing</span>
          <button className="btn btn-ghost btn-icon" disabled=${idx <= 0} onClick=${() => setSelected(laps[idx - 1].lap_number)} aria-label="Previous lap">‹</button>
          <select className="select" value=${selected === null ? "" : String(selected)} onChange=${(e) => setSelected(Number(e.target.value))} aria-label="Lap to analyse">
            ${laps.map((l) => html`<option key=${l.lap_number} value=${l.lap_number}>${lapOption(l)}</option>`)}
          </select>
          <button className="btn btn-ghost btn-icon" disabled=${idx < 0 || idx >= laps.length - 1} onClick=${() => setSelected(laps[idx + 1].lap_number)} aria-label="Next lap">›</button>
        </div>
        <button
          className="btn btn-ghost btn-icon swap"
          title="Swap the two laps"
          aria-label="Swap laps"
          disabled=${refNumber === null || selected === null}
          onClick=${() => {
            const a = selected;
            setSelected(refNumber);
            setCompare(a);
          }}
        >⇄</button>
        <div className="lap-slot ref">
          <span className="slot-label"><i className="ref-bg"></i>Compared with</span>
          <select
            className="select"
            value=${compare === null ? "auto" : String(compare)}
            onChange=${(e) => setCompare(e.target.value === "auto" ? null : Number(e.target.value))}
            aria-label="Lap to compare with"
          >
            <option value="auto">
              ${autoLap
                ? `Auto: ${stats && autoLap.lap_number === stats.best.lap_number ? "best" : "next best"} (lap ${autoLap.lap_number} · ${fmtLap(autoLap.lap_time_s)})`
                : "Auto: best lap"}
            </option>
            ${timed.filter((l) => l.lap_number !== selected).map((l) => html`<option key=${l.lap_number} value=${l.lap_number}>${lapOption(l)}</option>`)}
          </select>
        </div>
        ${delta !== null ? html`<span className=${"delta-chip " + deltaClass(delta)} title="Analysed lap minus comparison lap">${fmtDelta(delta)}</span>` : null}
        <span className="lap-bar-hint faint">
          Click a lap to analyse it · <b className="ref-text">vs</b> to compare · <span className="kbd">←</span><span className="kbd">→</span> step · <span className="kbd">B</span> best
        </span>
      </div>
      <div className="lap-strip" ref=${stripRef}>
        ${laps.map((lap) => {
          const isBest = stats && lap.lap_number === stats.best.lap_number;
          const isSel = lap.lap_number === selected;
          const isRef = lap.lap_number === refNumber && !isSel;
          const d = stats && lap.is_complete ? lap.lap_time_s - stats.best.lap_time_s : null;
          const cls = [
            "lap-chip",
            isBest ? "is-best" : "",
            !lap.is_complete || excluded.has(lap.lap_number) ? "excluded" : "",
            isRef ? "compare" : "",
            isSel ? "selected" : "",
          ].join(" ");
          return html`<div key=${lap.lap_number} className=${cls}>
            <button className="chip-main" onClick=${(e) => onPick(lap.lap_number, e.shiftKey)} title=${offTrackTitle(lap) || `Analyse lap ${lap.lap_number}`}>
              <span className="n">LAP ${lap.lap_number} <${OffTrackMark} lap=${lap} /></span>
              <span className="t">${fmtLap(lap.lap_time_s)}</span>
              <span className=${"d " + (isBest ? "purple" : deltaClass(d))}>${isBest ? "best" : !lap.is_complete ? "untimed" : fmtDelta(d)}</span>
            </button>
            ${isSel
              ? html`<span className="chip-role sel">lap</span>`
              : lap.is_complete
                ? html`<button
                    className=${"chip-role vs" + (isRef ? " on" : "")}
                    title=${isRef ? (compare === null ? "Compared automatically (your best)" : "Click to go back to comparing with your best") : `Compare with lap ${lap.lap_number}`}
                    onClick=${() => setCompare(isRef && compare !== null ? null : lap.lap_number)}
                  >vs</button>`
                : null}
          </div>`;
        })}
      </div>
    </div>`;
  }

  // ---------- Pace chart ----------

  function PaceChart({ laps, stats, excluded, selected, compare, onPick }) {
    const W = 1000;
    const H = 200;
    const temp = trackTempSpan(laps);
    const pad = { l: 64, r: temp ? 52 : 16, t: 14, b: 24 };
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
    // Track temperature on its own scale (right axis), so pace can be read against it.
    const yTemp = temp ? (c) => pad.t + (1 - (c - temp.lo) / (temp.hi - temp.lo)) * (H - pad.t - pad.b) : null;
    const tempPath = temp
      ? timed
          .map((lap, i) => (isNum(lap.track_temp_c) ? `${x(i).toFixed(1)},${yTemp(lap.track_temp_c).toFixed(1)}` : null))
          .reduce((d, p, i, all) => (p === null ? d : d + (i && all[i - 1] !== null ? "L" : "M") + p), "")
      : null;

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
      ${temp
        ? html`<g className="temp-axis">
            <path className="temp-line" d=${tempPath} />
            <text x=${W - pad.r + 8} y=${yTemp(temp.hi) + 4}>${temp.hi.toFixed(0)}°C</text>
            <text x=${W - pad.r + 8} y=${yTemp(temp.lo) + 4}>${temp.lo.toFixed(0)}°C</text>
          </g>`
        : null}
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
          <title>Lap ${lap.lap_number}: ${fmtLap(lap.lap_time_s)}${isNum(lap.track_temp_c) ? ` · track ${lap.track_temp_c.toFixed(1)}°C` : ""}${offTracks(lap) ? " · " + offTrackTitle(lap) : ""}</title>
          <circle cx=${x(i)} cy=${y(lap.lap_time_s)} r="5" />
          ${offTracks(lap) ? html`<text className="off-pt" x=${x(i)} y=${y(lap.lap_time_s) - 11}>!</text>` : null}
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
      let gearSel = null;
      let gearRef = null;
      for (let j = a; j <= b; j++) {
        if (sel.speed_kph[j] < minSel) {
          minSel = sel.speed_kph[j];
          gearSel = sel.gear[j];
        }
        if (ref && ref.speed_kph[j] < minRef) {
          minRef = ref.speed_kph[j];
          gearRef = ref.gear[j];
        }
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
        gearSel,
        gearRef,
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
  // Low → high ground: deep blue, teal, sand, white.
  const ELEV_STOPS = [
    [52, 84, 209],
    [45, 180, 190],
    [226, 200, 120],
    [245, 245, 240],
  ];
  const GEAR_COLORS = ["#6b7486", "#ff5f5f", "#ff9f43", "#e8c547", "#3ddc84", "#4cc2ff", "#8f7bff", "#e67bff", "#ffffff"];
  const gearColor = (g) => GEAR_COLORS[Math.max(0, Math.min(GEAR_COLORS.length - 1, g || 0))];

  // ---------- Track map ----------

  /** Where the comparison lap was when the analysed lap reached `pct`, as a lap fraction. */
  function ghostPct(sel, ref, pct) {
    const n = sel.time_s.length - 1;
    const t = sel.time_s[idxOf(pct, n)];
    let lo = 0;
    let hi = n;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (ref.time_s[mid] < t) lo = mid + 1;
      else hi = mid;
    }
    return lo / n;
  }

  /** viewBox and outline path for a track map, plus `s`: metres per pixel at `size` px wide. */
  function mapGeometry(points, size = 440) {
    const xs = points.map((p) => p[0]);
    const ys = points.map((p) => p[1]);
    const minX = Math.min(...xs);
    const maxX = Math.max(...xs);
    const minY = Math.min(...ys);
    const maxY = Math.max(...ys);
    const s = Math.max(maxX - minX, maxY - minY) / size;
    const pad = 30 * s;
    return {
      s,
      viewBox: `${minX - pad} ${-maxY - pad} ${maxX - minX + 2 * pad} ${maxY - minY + 2 * pad}`,
      path: points.map((p, i) => `${i ? "L" : "M"}${p[0].toFixed(1)},${(-p[1]).toFixed(1)}`).join(" ") + "Z",
    };
  }

  /** Path along the track between two lap fractions. */
  function stretchPath(points, start, end) {
    const n = points.length - 1;
    const a = idxOf(start, n);
    const b = idxOf(end, n);
    const d = [];
    for (let j = a; j <= b; j++) d.push(`${j === a ? "M" : "L"}${points[j][0].toFixed(1)},${(-points[j][1]).toFixed(1)}`);
    return d.join(" ");
  }

  /** Turn badge positions, just outside each corner. */
  function turnBadges(map, s) {
    const points = map.points;
    const n = points.length - 1;
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
  }

  function TurnBadge({ b, s, active, hovered, onClick, onHover }) {
    return html`<g
      className=${"turn-badge" + (active ? " active" : "") + (hovered ? " hovered" : "")}
      transform=${`translate(${b.x} ${b.y})`}
      onClick=${() => onClick(b.i)}
      onMouseEnter=${onHover ? () => onHover(b.i) : null}
      onMouseLeave=${onHover ? () => onHover(null) : null}
    >
      <title>Turn ${b.label}: click for an entry and exit breakdown</title>
      <circle r=${9 * s} />
      <text fontSize=${(b.label.length > 2 ? 7.5 : 9.5) * s} dy=${3.3 * s}>${b.label}</text>
    </g>`;
  }

  function TrackMapView({ map, sel, refTrace: ref, mode, cursorPct, onHover, activeTurn, hoverTurn, onTurnClick, selOff, refOff }) {
    const svgRef = useRef(null);
    const points = map.points;
    const n = points.length - 1;

    const geometry = useMemo(() => mapGeometry(points), [map]);
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
      const alt = sel.alt_m && sel.alt_m.length ? sel.alt_m : null;
      const aMin = alt ? Math.min(...alt) : 0;
      const aMax = alt ? Math.max(...alt) : 1;
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
        } else if (mode === "gear") {
          color = gearColor(sel.gear[Math.round((k + e) / 2)]);
        } else if (mode === "elevation" && alt) {
          color = lerpColor(ELEV_STOPS, (alt[Math.round((k + e) / 2)] - aMin) / Math.max(0.5, aMax - aMin));
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

    const badges = useMemo(() => turnBadges(map, s), [map]);

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
    const ghost = cursor && sel && ref ? points[idxOf(ghostPct(sel, ref, cursorPct), n)] : null;
    const offMarks = (pcts, cls) =>
      (pcts || []).map((pct, i) => {
        const p = points[idxOf(pct, n)];
        return html`<g key=${cls + i} className=${"off-marker " + cls} transform=${`translate(${p[0]} ${-p[1]})`}>
          <title>${cls === "sel" ? "Analysed" : "Comparison"} lap went off track here</title>
          <path d=${`M0,${-9 * s}L${8 * s},${6 * s}H${-8 * s}Z`} />
          <text fontSize=${9 * s} dy=${4 * s}>!</text>
        </g>`;
      });
    const segs = turnSegments(map.turns);
    const activeSeg = isNum(activeTurn) ? segs[activeTurn] : null;
    const activePath = activeSeg ? stretchPath(points, activeSeg.start, activeSeg.end) : null;
    const hoverSeg = isNum(hoverTurn) && hoverTurn !== activeTurn ? segs[hoverTurn] : null;

    return html`<svg ref=${svgRef} className="track-map" viewBox=${geometry.viewBox} onMouseMove=${onMove} onMouseLeave=${() => onHover(null)} role="img" aria-label="Track map">
      <path d=${geometry.path} className="map-base" />
      ${activePath ? html`<path d=${activePath} className="map-active" />` : null}
      ${hoverSeg ? html`<path d=${stretchPath(points, hoverSeg.start, hoverSeg.end)} className="map-active hover" />` : null}
      ${overlay || html`<path d=${geometry.path} className="map-line" />`}
      <line ...${startLine} className="map-start" />
      ${badges.map((b) => html`<${TurnBadge} key=${b.i} b=${b} s=${s} active=${activeTurn === b.i} hovered=${hoverTurn === b.i} onClick=${onTurnClick} />`)}
      ${offMarks(refOff, "ref")}
      ${offMarks(selOff, "sel")}
      ${ghost ? html`<circle cx=${ghost[0]} cy=${-ghost[1]} r=${6 * s} className="map-cursor ghost" />` : null}
      ${cursor ? html`<circle cx=${cursor[0]} cy=${-cursor[1]} r=${6 * s} className="map-cursor" />` : null}
    </svg>`;
  }

  // ---------- Turn references ----------

  /**
   * Links text that mentions turns to the track map. Provided by RunView once the map has loaded:
   * `turns`, `hover` ({ turn } or { pct } being pointed at, or null), `setHover`, and `focusTurn(i)`,
   * which opens that turn's breakdown in the track section.
   */
  const TurnContext = React.createContext(null);

  // "Turn 5", "Turns 5 and 6", "turns 10-11", "T5", "T10a".
  const TURN_RE = /\b([Tt]urns?\s+)(\d{1,2}[a-zA-Z]?(?:\s*(?:,|and|&|or|to|-|–)\s*\d{1,2}[a-zA-Z]?)*)\b|\bT(\d{1,2}[a-zA-Z]?)\b/g;

  function turnIndex(turns, label) {
    const want = String(label).toLowerCase();
    return turns.findIndex((t) => t.label.toLowerCase() === want);
  }

  function TurnChip({ index, children }) {
    const ctx = useContext(TurnContext);
    const label = ctx.turns[index].label;
    return html`<button
      type="button"
      className=${"turn-ref" + (ctx.hover && ctx.hover.turn === index ? " hovered" : "")}
      title=${`Turn ${label}: hover to see it on the map, click to open its breakdown`}
      onMouseEnter=${() => ctx.setHover({ turn: index })}
      onMouseLeave=${() => ctx.setHover(null)}
      onFocus=${() => ctx.setHover({ turn: index })}
      onBlur=${() => ctx.setHover(null)}
      onClick=${(e) => {
        e.stopPropagation();
        ctx.setHover(null);
        ctx.focusTurn(index);
      }}
    >${children}</button>`;
  }

  /** Text with every turn it mentions turned into a chip that points at the turn on the map. */
  function TurnText({ text }) {
    const ctx = useContext(TurnContext);
    if (!text || !ctx || !ctx.turns.length) return text || null;
    const out = [];
    let last = 0;
    let k = 0;
    for (const m of text.matchAll(TURN_RE)) {
      if (m.index > last) out.push(text.slice(last, m.index));
      last = m.index + m[0].length;
      if (m[3]) {
        const i = turnIndex(ctx.turns, m[3]);
        out.push(i >= 0 ? html`<${TurnChip} key=${k++} index=${i}>${m[0]}<//>` : m[0]);
        continue;
      }
      const nums = m[2].split(/(\d{1,2}[a-zA-Z]?)/);
      const single = nums.filter((p, j) => j % 2 === 1).length === 1;
      const i0 = single ? turnIndex(ctx.turns, nums[1]) : -1;
      if (single && i0 >= 0) {
        out.push(html`<${TurnChip} key=${k++} index=${i0}>${m[0]}<//>`);
        continue;
      }
      out.push(m[1]);
      nums.forEach((part, j) => {
        const i = j % 2 === 1 ? turnIndex(ctx.turns, part) : -1;
        out.push(i >= 0 ? html`<${TurnChip} key=${k++} index=${i}>${part}<//>` : part);
      });
    }
    if (last < text.length) out.push(text.slice(last));
    return out;
  }

  /**
   * A small track map pinned to the corner of the window while the main map is scrolled out of
   * view, so turn numbers in tables and coach notes always have a map next to them.
   */
  function MiniMap({ map, title, hidden }) {
    const ctx = useContext(TurnContext);
    const [open, setOpen] = useState(() => stored("pcc.miniMap", "open") === "open");
    const geometry = useMemo(() => mapGeometry(map.points, 220), [map]);
    const badges = useMemo(() => turnBadges(map, geometry.s), [map, geometry]);
    const hover = ctx && ctx.hover;
    // Pointing at a turn while the map is collapsed peeks it open.
    const show = open || !!hover;
    if (hidden) return null;

    const toggle = () => {
      setOpen(!open);
      store("pcc.miniMap", open ? "closed" : "open");
    };
    if (!show) {
      return html`<button className="mini-map-toggle" onClick=${toggle} title="Show the track map">
        <svg viewBox=${geometry.viewBox} aria-hidden="true"><path d=${geometry.path} /></svg>
        Map
      </button>`;
    }

    const { s } = geometry;
    const points = map.points;
    const segs = turnSegments(map.turns);
    const seg = hover && isNum(hover.turn) ? segs[hover.turn] : null;
    const dot = hover && isNum(hover.pct) ? points[idxOf(hover.pct, points.length - 1)] : null;
    return html`<aside className=${"mini-map" + (open ? "" : " peek")} aria-label="Track map">
      <div className="mini-map-head">
        <span className="mini-map-title">${title || "Track"}</span>
        <button className="btn btn-ghost btn-icon" onClick=${toggle} title=${open ? "Minimise the map" : "Keep the map open"} aria-label=${open ? "Minimise map" : "Keep map open"}>
          ${open ? "–" : "📌"}
        </button>
      </div>
      <svg className="track-map" viewBox=${geometry.viewBox} role="img" aria-label="Track map">
        <path d=${geometry.path} className="map-base" />
        <path d=${geometry.path} className="map-line" />
        ${seg ? html`<path d=${stretchPath(points, seg.start, seg.end)} className="map-active hover" />` : null}
        ${badges.map(
          (b) => html`<${TurnBadge}
            key=${b.i}
            b=${b}
            s=${s}
            hovered=${seg && hover.turn === b.i}
            onClick=${(i) => ctx.focusTurn(i)}
          />`
        )}
        ${dot ? html`<circle cx=${dot[0]} cy=${-dot[1]} r=${6 * s} className="map-cursor" />` : null}
      </svg>
      <div className="mini-map-hint faint">Click a turn to open its breakdown</div>
    </aside>`;
  }

  // ---------- Weather ----------

  const WETNESS = [null, "Dry", "Mostly dry", "Very lightly wet", "Lightly wet", "Moderately wet", "Very wet", "Extremely wet"];
  const SKY_ICON = { clear: "☀️", "partly cloudy": "⛅", "mostly cloudy": "🌥️", overcast: "☁️" };

  function fmtTempRange(r) {
    return Math.abs(r.end - r.start) >= 0.5 ? `${r.start.toFixed(0)}→${r.end.toFixed(0)}°C` : `${r.start.toFixed(0)}°C`;
  }

  const lapWeatherTitle = (lap) =>
    [isNum(lap.air_temp_c) ? `Air ${lap.air_temp_c.toFixed(1)}°C` : null, lap.track_wetness ? `Track ${(WETNESS[lap.track_wetness] || "").toLowerCase()}` : null].filter(Boolean).join(" · ");

  /** Min and max track temperature over the timed laps, when it moved enough to be worth charting. */
  function trackTempSpan(laps) {
    const temps = laps.filter((l) => l.is_complete && isNum(l.track_temp_c)).map((l) => l.track_temp_c);
    if (temps.length < 2) return null;
    const lo = Math.min(...temps);
    const hi = Math.max(...temps);
    return hi - lo >= 0.5 ? { lo, hi } : null;
  }

  const rangeTitle = (what, r) => `${what}: ${r.start.toFixed(1)}°C at the start, ${r.end.toFixed(1)}°C at the end (${r.min.toFixed(1)}–${r.max.toFixed(1)}°C)`;

  /** The run's conditions in one line under the title. */
  function WeatherStrip({ weather: w }) {
    if (!w) return null;
    const item = (key, label, value, title, cls) =>
      html`<span key=${key} className=${"wx " + (cls || "")} title=${title || ""}>${label ? html`<span className="wx-k">${label}</span>` : null}${value}</span>`;
    const items = [];
    if (w.skies) {
      const icon = SKY_ICON[w.skies.toLowerCase()] || "";
      items.push(item("sky", null, `${icon} ${w.skies}${w.skies_end ? ` → ${w.skies_end.toLowerCase()}` : ""}`, "Sky"));
    }
    if (w.air_temp_c) items.push(item("air", "Air", fmtTempRange(w.air_temp_c), rangeTitle("Air temperature", w.air_temp_c)));
    if (w.track_temp_c) {
      const swing = w.track_temp_c.max - w.track_temp_c.min >= 4;
      items.push(item("track", "Track", fmtTempRange(w.track_temp_c), rangeTitle("Track temperature", w.track_temp_c), swing ? "warn" : ""));
    }
    if (isNum(w.wind_kph)) items.push(item("wind", "Wind", `${w.wind_kph.toFixed(0)} km/h${w.wind_dir ? " " + w.wind_dir : ""}`, "Average wind speed and direction"));
    if (isNum(w.humidity_pct)) items.push(item("hum", "Humidity", `${w.humidity_pct.toFixed(0)}%`));
    if (w.wetness) {
      const wet = w.wetness !== "Dry" || (w.wetness_end && w.wetness_end !== "Dry");
      items.push(item("wet", null, `${wet ? "💧 " : ""}${w.wetness}${w.wetness_end ? ` → ${w.wetness_end.toLowerCase()}` : ""}`, "Track surface", wet ? "warn" : ""));
    }
    if (isNum(w.rain_pct) && w.rain_pct > 0.5) items.push(item("rain", "Rain", `up to ${w.rain_pct.toFixed(0)}%`, "Heaviest rain during the run", "warn"));
    if (w.declared_wet) items.push(item("decl", null, "Declared wet", "Race control declared the session wet", "warn"));
    if (w.time_of_day) items.push(item("tod", null, `🕐 ${w.time_of_day}`, "In-sim time of day when the session started"));
    if (w.weather_type) items.push(item("type", null, w.weather_type === "Realistic" ? "Dynamic weather" : `${w.weather_type} weather`, null, "faint"));
    return items.length ? html`<div className="weather-strip">${items}</div>` : null;
  }

  // ---------- Corner table ----------

  function CornerTable({ corners, hasRef, activeTurn, onSelectTurn, editing, labels, setLabels }) {
    const turnCtx = useContext(TurnContext);
    const hoverRow = (i) => turnCtx && turnCtx.setHover(i === null ? null : { turn: i });
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
            <th title="Gear at the slowest point (the comparison lap's in brackets when it differs)">Gear</th>
            ${hasGrip ? html`<th title="Average grip used while braking and cornering, as % of the session's peak">Grip</th>` : null}
            ${hasBal ? html`<th title="Mid-corner steering beyond what the car's rotation needed. + understeer, − oversteer. Compare laps, not corners.">Balance</th>` : null}
            ${hasAbs ? html`<th title="Share of braking with ABS active">ABS</th>` : null}
          </tr>
        </thead>
        <tbody>
          ${corners.map((c, i) => {
            const speedDiff = isNum(c.minRef) ? c.minSel - c.minRef : null;
            return html`<tr
              key=${i}
              className=${activeTurn === i ? "active" : ""}
              onClick=${() => onSelectTurn(activeTurn === i ? null : i)}
              onMouseEnter=${() => hoverRow(i)}
              onMouseLeave=${() => hoverRow(null)}
            >
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
              <td>
                ${c.gearSel ? html`<span className="gear-badge sm" style=${{ background: gearColor(c.gearSel) }}>${c.gearSel}</span>` : "—"}
                ${hasRef && c.gearRef && c.gearRef !== c.gearSel ? html`<span className="bal-ref"> (${c.gearRef})</span>` : null}
              </td>
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
    { key: "rpm", label: "RPM", h: 72, digits: 0 },
    { key: "steer_deg", label: "Steering", unit: "°", h: 72, symmetric: true },
    { key: "lat_g", label: "Lateral g", unit: "g", h: 64, symmetric: true, minSpan: 0.5, digits: 2 },
    { key: "long_g", label: "Long g", unit: "g", h: 64, symmetric: true, minSpan: 0.5, digits: 2 },
    { key: "balance_deg", label: "Balance · + under / − over", unit: "°", h: 72, symmetric: true, signed: true },
    { key: "alt_rel", src: "alt_m", label: "Elevation", unit: "m", h: 56, digits: 1 },
  ];

  const VERDICT_LABEL = { early: "early", late: "late", on_time: "on time", part_throttle: "part throttle", unknown: "" };

  function TraceChart({ sel, refTrace: ref, lengthM, turns, sectorPcts, cursorPct, onCursor, view, setView, shiftRpm, upshifts }) {
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
    // Elevation relative to the lowest point of the analysed lap, for both laps.
    const altRel = useMemo(() => {
      if (!sel.alt_m || !sel.alt_m.length) return null;
      const base = Math.min(...sel.alt_m);
      const rel = (t) => (t && t.alt_m && t.alt_m.length ? t.alt_m.map((v) => v - base) : null);
      return { sel: rel(sel), ref: rel(ref) };
    }, [sel, ref]);
    const channel = (trace, key) => {
      if (!trace) return null;
      if (key === "delta") return trace === sel ? delta : null;
      if (key === "alt_rel") return altRel ? (trace === sel ? altRel.sel : altRel.ref) : null;
      return trace[key];
    };

    let y = pad.top;
    const layout = PANELS.filter((p) => p.key === "delta" || (sel[p.src || p.key] && sel[p.src || p.key].length)).map((p) => {
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
      if (p.key === "rpm" && isNum(shiftRpm)) values.push(shiftRpm);
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
      return v.toFixed(p.digits || 0) + (p.unit === "%" || p.unit === "g" || p.unit === "m" ? p.unit : "");
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
              ${p.key === "rpm" && isNum(shiftRpm)
                ? html`<line className="shift-line" x1=${pad.l} x2=${pad.l + plotW} y1=${yv(shiftRpm)} y2=${yv(shiftRpm)}><title>Shift light ${shiftRpm.toFixed(0)} rpm</title></line>`
                : null}
              ${refArr ? html`<path className="trace-line ref" d=${line(refArr, p, dom, p.step)} />` : null}
              ${selArr && p.key !== "delta" ? html`<path className="trace-line sel" d=${line(selArr, p, dom, p.step)} />` : null}
              ${p.key === "rpm"
                ? (upshifts || [])
                    .filter((s) => s.pct >= r0 && s.pct <= r1)
                    .map(
                      (s, i) => html`<circle key=${i} className=${"shift-pt " + s.verdict} cx=${x(s.pct * n)} cy=${yv(s.rpm)} r="4">
                        <title>${s.from}→${s.to} at ${s.rpm.toFixed(0)} rpm${VERDICT_LABEL[s.verdict] ? " · " + VERDICT_LABEL[s.verdict] : ""}</title>
                      </circle>`
                    )
                : null}
            </g>
            <text className="panel-label" x=${pad.l + 6} y=${p.top + 13}>
              ${p.key === "brake" && sel.abs && sel.abs.length
                ? "Brake · ABS marked"
                : p.key === "rpm" && isNum(shiftRpm)
                  ? "RPM · dashed = shift light · dots = upshifts"
                  : p.key === "alt_rel"
                    ? "Elevation · metres above the lowest point"
                    : p.label}
            </text>
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
   * How a lap used the tyres over the samples in [i0, i1]: `used` = average combined g while
   * braking or cornering, as % of the session peak; `limit` = share of that time spent at 85%+
   * of the peak; `trail` = share of braking done while also turning (trail braking).
   */
  function gripStats(t, i0, i1, peakG) {
    if (!t || !t.lat_g || !t.lat_g.length || !isNum(peakG)) return null;
    let working = 0;
    let sum = 0;
    let near = 0;
    let braking = 0;
    let trail = 0;
    for (let j = i0; j <= i1; j++) {
      const lat = Math.abs(t.lat_g[j]);
      const combined = Math.hypot(t.lat_g[j], t.long_g[j]);
      const isBraking = t.brake[j] >= 5;
      if (isBraking) {
        braking++;
        if (lat >= 0.35 * peakG) trail++;
      }
      if (isBraking || lat > 0.3 * peakG) {
        working++;
        sum += combined;
        if (combined >= 0.85 * peakG) near++;
      }
    }
    if (!working) return null;
    return {
      used: (sum / working / peakG) * 100,
      limit: (near / working) * 100,
      trail: braking ? (trail / braking) * 100 : null,
    };
  }

  const GRIP_ROWS = [
    ["Grip used", "used", "Average force while braking or cornering, as % of the dashed ring. Higher = using more of what the tyres can give."],
    ["Time at the limit", "limit", "Share of braking and cornering spent at 85%+ of the ring. That's where lap time is: more is quicker, as long as the car stays tidy."],
    ["Trail braking", "trail", "Share of braking done while already turning in: dots in the lower diagonals. Some is good; it keeps the front loaded for turn-in."],
  ];

  /**
   * Friction circle (g-g plot) for the part of the lap in view: every point is one sample of
   * lateral vs longitudinal g. A lap that fills the circle out to the session peak, including
   * the diagonals (trail braking into the corner, throttle on the way out), is using the grip.
   */
  function GripCircle({ sel, refTrace: ref, view, peakG, cursorPct, onCursor, selected, refNumber }) {
    const svgRef = useRef(null);
    const [help, setHelp] = useState(() => stored("pcc.gripHelpOpen", "no") === "yes");
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
    const pctOfPeak = (p) => (isNum(peakG) ? ` · ${((Math.hypot(p.lat, p.long) / peakG) * 100).toFixed(0)}% of the ring` : "");
    const statsSel = gripStats(sel, i0, i1, peakG);
    const statsRef = gripStats(ref, i0, i1, peakG);
    const zoomed = view[0] > 0 || view[1] < 1;
    // Quadrant labels sit on the diagonals, inside the plot.
    const d45 = (c - 18) * 0.78 * Math.SQRT1_2;

    return html`<div className="grip-circle">
      <div className="section-label">
        <span>Grip circle · ${zoomed ? "zoomed section" : "whole lap"}</span>
        <button
          className=${"btn btn-sm help-btn" + (help ? " on" : "")}
          aria-expanded=${help}
          onClick=${() => {
            setHelp(!help);
            store("pcc.gripHelpOpen", help ? "no" : "yes");
          }}
        >${help ? html`<span aria-hidden="true">✕</span> Hide help` : html`<span className="help-icon" aria-hidden="true">?</span> How to read this`}</button>
      </div>
      ${help
        ? html`<div className="grip-help">
            <p>Every dot is one moment of the lap, placed by the force on the car: <b>left/right</b> is cornering, <b>up</b> is accelerating, <b>down</b> is braking.</p>
            <p>The <b>dashed ring</b> is the most grip you used all session, roughly what the tyres can give. The further the dots reach toward it, the more grip you're using.</p>
            <p>A <b>"+" shape</b> means you brake, then turn, then accelerate as separate steps. Dots filling the <b>diagonals</b> mean you blend them: trail braking into the corner (lower corners) and feeding in throttle while still turning (upper corners). That's usually quicker.</p>
            <p>Compare with the <span className="ref-text">orange</span> lap: where it reaches further out, that lap used more grip. Click a turn to see just that corner.</p>
          </div>`
        : null}
      <svg ref=${svgRef} viewBox=${`0 0 ${size} ${size}`} role="img" aria-label="Lateral versus longitudinal g" onMouseMove=${onMove} onMouseLeave=${() => onCursor(null)}>
        <line className="gg-axis" x1=${px(-extent)} x2=${px(extent)} y1=${c} y2=${c} />
        <line className="gg-axis" x1=${c} x2=${c} y1=${py(extent)} y2=${py(-extent)} />
        ${rings.map((g) => html`<g key=${g}>
          <circle className="gg-ring" cx=${c} cy=${c} r=${g * r} />
          <text className="gg-tick" x=${c + 3} y=${py(g) - 3}>${g}g</text>
        </g>`)}
        ${isNum(peakG) ? html`<circle className="gg-peak" cx=${c} cy=${c} r=${peakG * r}><title>Session peak ${peakG.toFixed(2)} g</title></circle>` : null}
        <text className="gg-edge" x=${c} y=${10}>Accelerating</text>
        <text className="gg-edge" x=${c} y=${size - 3}>Braking</text>
        <text className="gg-edge" x=${4} y=${c - 4} textAnchor="start">Left</text>
        <text className="gg-edge" x=${size - 4} y=${c - 4} textAnchor="end">Right</text>
        <text className="gg-quad" x=${c - d45} y=${c + d45}>trail brake</text>
        <text className="gg-quad" x=${c + d45} y=${c + d45}>trail brake</text>
        <text className="gg-quad" x=${c - d45} y=${c - d45}>power out</text>
        <text className="gg-quad" x=${c + d45} y=${c - d45}>power out</text>
        ${ref && ref.lat_g ? html`<path className="gg-dots ref" d=${dots(ref)} />` : null}
        <path className="gg-dots sel" d=${dots(sel)} />
        ${curRef ? html`<circle className="gg-cursor ref" cx=${px(curRef.lat)} cy=${py(curRef.long)} r="4.5" />` : null}
        ${cur ? html`<circle className="gg-cursor sel" cx=${px(cur.lat)} cy=${py(cur.long)} r="4.5" />` : null}
      </svg>
      <div className="gg-readout mono">
        ${cur
          ? html`<div><span className="sel-fill">Lap ${selected}</span> ${fmtSigned(cur.lat, 2)} lat ${fmtSigned(cur.long, 2)} long${pctOfPeak(cur)}</div>
              ${curRef ? html`<div><span className="ref-fill">Lap ${refNumber}</span> ${fmtSigned(curRef.lat, 2)} lat ${fmtSigned(curRef.long, 2)} long${pctOfPeak(curRef)}</div>` : null}`
          : html`<div className="faint">Hover the map, the traces or the dots for values</div>`}
      </div>
      ${statsSel
        ? html`<table className="grip-stats">
            <thead>
              <tr><th></th><th><span className="dot sel-bg"></span>Lap ${selected}</th>${statsRef ? html`<th><span className="dot ref-bg"></span>Lap ${refNumber}</th>` : null}</tr>
            </thead>
            <tbody>
              ${GRIP_ROWS.map(([label, key, hint]) => {
                const a = statsSel[key];
                const b = statsRef ? statsRef[key] : null;
                const cls = isNum(a) && isNum(b) && Math.abs(a - b) >= 3 ? (a > b ? "good" : "bad") : "";
                return html`<tr key=${key} title=${hint}>
                  <td>${label} <span className="info-dot">?</span></td>
                  <td className=${"mono " + cls}>${isNum(a) ? a.toFixed(0) + "%" : "—"}</td>
                  ${statsRef ? html`<td className="mono faint">${isNum(b) ? b.toFixed(0) + "%" : "—"}</td>` : null}
                </tr>`;
              })}
            </tbody>
          </table>`
        : null}
    </div>`;
  }

  // ---------- Corner coach ----------

  const ordinal = (n) => n + (n % 100 >= 11 && n % 100 <= 13 ? "th" : ["th", "st", "nd", "rd"][n % 10] || "th");

  /** Rows of the numbers table: [label, key, unit, digits, which way is better (+1 higher, −1 lower, 0 neither), hint]. */
  const CORNER_ROWS = [
    ["Speed at braking", "entry_speed_kph", " kph", 0, 1, "Where braking starts. Mostly set by the exit of the corner before."],
    ["Brake point", "brake_point_m", " m", 0, 0, "Metres before the apex where braking starts. Smaller = later."],
    ["Minimum speed", "min_speed_kph", " kph", 0, 1, ""],
    ["Throttle pickup", "throttle_on_m", " m", 0, -1, "Metres after the apex where the throttle reaches 20% (negative = before the apex)."],
    ["Full throttle", "full_throttle_m", " m", 0, -1, "Metres after the apex to full throttle."],
    ["Exit speed", "exit_speed_kph", " kph", 0, 1, "Speed where the corner hands over to the next straight."],
    ["Coasting", "coast_m", " m", 0, -1, "Distance with neither pedal pressed."],
    ["Gear at apex", "apex_gear", "", 0, 0, "Gear at the slowest point of the corner."],
    ["ABS", "abs_pct", "%", 0, -1, "Share of braking with ABS active."],
    ["Entry balance", "entry_balance_deg", "°", 1, 0, "+ understeer, − oversteer (degrees of extra steering lock)."],
    ["Exit balance", "exit_balance_deg", "°", 1, 0, "+ understeer, − oversteer."],
  ];

  function GradeChip({ label, pct }) {
    const dir = pct >= 1 ? "uphill" : pct <= -1 ? "downhill" : "flat";
    return html`<span className=${"chip grade " + dir} title="Average gradient over ~150 m">
      ${label} ${dir === "flat" ? "flat" : html`${dir === "uphill" ? "↗" : "↘"} ${dir} ${Math.abs(pct).toFixed(0)}%`}
    </span>`;
  }

  function PhaseColumn({ title, dt, rank, notes, vs }) {
    return html`<div className="phase">
      <div className="phase-head">
        <span className="section-label">${title}</span>
        <span className=${"delta-chip " + deltaClass(dt)} title=${`Time through the ${title.toLowerCase()} vs ${vs}`}>${fmtDelta(dt)}</span>
      </div>
      ${rank
        ? html`<div className="faint phase-rank">
            ${rank.rank === 1 ? "Quickest" : ordinal(rank.rank) + " quickest"} of ${rank.of} laps${rank.rank > 1 ? ` · best is lap ${rank.best_lap}` : ""}
          </div>`
        : null}
      ${notes.went_well.length ? html`<ul className="note-list good">${notes.went_well.map((t, i) => html`<li key=${i}>${t}</li>`)}</ul>` : null}
      ${notes.to_work_on.length ? html`<ul className="note-list bad">${notes.to_work_on.map((t, i) => html`<li key=${i}>${t}</li>`)}</ul>` : null}
    </div>`;
  }

  /**
   * Entry/exit breakdown of one corner: the analysed lap against the comparison lap, or against
   * the driver's typical lap (median of the other counted laps), with optional coach advice.
   */
  function CornerCoach({ split, turnIndex, turnLabel, selected, refNumber, excluded, model, offHere, onClose }) {
    const hasRef = refNumber !== null && refNumber !== selected;
    const [mode, setMode] = useState(() => stored("pcc.cornerMode", "ref"));
    const vsRef = mode === "ref" && hasRef;
    const [report, setReport] = useState(null);
    const [error, setError] = useState(null);
    const [coach, setCoach] = useState({ loading: false, text: null, error: null });
    const excludedKey = [...excluded].sort((a, b) => a - b).join(",");

    const body = (withCoach) =>
      JSON.stringify({
        lap: selected,
        ref_lap: vsRef ? refNumber : null,
        exclude: excludedKey ? excludedKey.split(",").map(Number) : [],
        coach: withCoach,
        model: model || null,
      });

    useEffect(() => {
      let cancelled = false;
      setError(null);
      setCoach({ loading: false, text: null, error: null });
      api(`/api/splits/${split.id}/corners/${turnIndex}`, { method: "POST", body: body(false) })
        .then((r) => !cancelled && setReport(r.report))
        .catch((err) => {
          if (cancelled) return;
          setReport(null);
          setError(err.message);
        });
      return () => {
        cancelled = true;
      };
    }, [split.id, turnIndex, selected, refNumber, vsRef, excludedKey]);

    async function askCoach() {
      setCoach({ loading: true, text: null, error: null });
      try {
        const r = await api(`/api/splits/${split.id}/corners/${turnIndex}`, { method: "POST", body: body(true) });
        setCoach({ loading: false, text: r.feedback, error: null, model: r.model });
      } catch (err) {
        setCoach({ loading: false, text: null, error: err.message });
      }
    }

    const vs = report ? report.baseline_label : vsRef ? `lap ${refNumber}` : "your typical lap";
    const stale = report && (report.lap !== selected || (report.ref_lap ?? null) !== (vsRef ? refNumber : null));

    return html`<div className="corner-coach">
      <div className="corner-coach-head">
        <div>
          <div className="corner-coach-title">
            <span className="turn-pill big">${turnLabel}</span>
            <span>Turn ${turnLabel}</span>
            <span className="lap-vs">
              <span className="dot sel-bg"></span>Lap ${selected}
              <span className="faint">vs</span>
              ${vsRef ? html`<span className="dot ref-bg"></span>Lap ${refNumber}` : html`<span>typical lap${report ? ` (median of ${report.field_size})` : ""}</span>`}
            </span>
          </div>
          ${offHere ? html`<div className="bad corner-off"><span className="off-mark">!</span> Lap ${selected} went off track in this corner.</div>` : null}
        </div>
        <div className="head-actions">
          <${Segmented}
            label="Compare corner with"
            value=${vsRef ? "ref" : "field"}
            onChange=${(m) => {
              setMode(m);
              store("pcc.cornerMode", m);
            }}
            options=${[
              ...(hasRef ? [{ value: "ref", label: `vs lap ${refNumber}` }] : []),
              { value: "field", label: "vs all my laps" },
            ]}
          />
          <button className="btn btn-ghost btn-icon" onClick=${onClose} aria-label="Close corner breakdown" title="Close">✕</button>
        </div>
      </div>

      ${error
        ? html`<p className="muted">${error}</p>`
        : !report
          ? html`<p className="muted"><span className="spinner inline"></span>Comparing…</p>`
          : html`<div className=${"corner-coach-body" + (stale ? " stale" : "")}>
              <div className="phases">
                <${PhaseColumn} title="Entry" dt=${report.sel.entry_time_s - report.base.entry_time_s} rank=${report.entry_rank} notes=${report.entry} vs=${vs} />
                <${PhaseColumn} title="Exit" dt=${report.sel.exit_time_s - report.base.exit_time_s} rank=${report.exit_rank} notes=${report.exit} vs=${vs} />
              </div>
              <div className="corner-details">
              <div className="corner-context">
              ${report.terrain_notes.length || report.terrain
                ? html`<div className="terrain">
                    <div className="section-label">The road here</div>
                    ${report.terrain
                      ? html`<div className="terrain-chips">
                          <${GradeChip} label="Braking zone" pct=${report.terrain.entry_grade_pct} />
                          <span className="chip" title="Height of the apex compared with the road 40 m either side">
                            Apex ${report.terrain.apex_crest_m >= 0.5 ? "on a crest" : report.terrain.apex_crest_m <= -0.5 ? "in a compression" : "flat"}
                          </span>
                          <${GradeChip} label="Exit" pct=${report.terrain.exit_grade_pct} />
                        </div>`
                      : null}
                    ${report.terrain_notes.length ? html`<ul className="note-list neutral">${report.terrain_notes.map((t, i) => html`<li key=${i}>${t}</li>`)}</ul>` : null}
                  </div>`
                : null}
              ${report.gear_options.length
                ? html`<div className="gear-options">
                    <div className="section-label">Gear at the apex, all counted laps</div>
                    <div className="gear-option-row">
                      ${(() => {
                        const best = Math.min(...report.gear_options.map((g) => g.best_time_s));
                        return report.gear_options.map(
                          (g) => html`<div
                            key=${g.gear}
                            className=${"gear-option" + (g.gear === report.sel.apex_gear ? " mine" : "")}
                            title=${`Laps ${g.laps.join(", ")} · best ${g.best_time_s.toFixed(2)}s, average ${g.avg_time_s.toFixed(2)}s through the corner`}
                          >
                            <span className="gear-badge" style=${{ background: gearColor(g.gear) }}>${g.gear}</span>
                            <span>
                              <b className="mono">${g.best_time_s - best < 0.0005 ? "quickest" : "+" + (g.best_time_s - best).toFixed(2) + "s"}</b>
                              <span className="faint"> best pass · ${g.laps.length} lap${g.laps.length === 1 ? "" : "s"}${g.gear === report.sel.apex_gear ? " · this lap" : ""}</span>
                            </span>
                          </div>`
                        );
                      })()}
                    </div>
                  </div>`
                : null}
              </div>
              <div className="corner-numbers">
                <table className="corners compact">
                  <thead>
                    <tr><th className="l"></th><th><span className="dot sel-bg"></span>Lap ${report.lap}</th><th>${vsRef ? html`<span className="dot ref-bg"></span>Lap ${report.ref_lap}` : "Typical"}</th><th>Diff</th></tr>
                  </thead>
                  <tbody>
                    ${CORNER_ROWS.filter(([, key]) => isNum(report.sel[key]) || isNum(report.base[key])).map(([label, key, unit, digits, better, hint]) => {
                      const a = report.sel[key];
                      const b = report.base[key];
                      const d = isNum(a) && isNum(b) ? a - b : null;
                      const threshold = unit === " kph" ? 1 : unit === " m" ? 3 : unit === "%" ? 5 : 1.5;
                      const cls = d === null || !better || Math.abs(d) < threshold ? "faint" : d * better > 0 ? "good" : "bad";
                      return html`<tr key=${key} title=${hint}>
                        <td className="l">${label}</td>
                        <td>${fmtNum(a, digits, unit)}</td>
                        <td className="faint">${fmtNum(b, digits, unit)}</td>
                        <td className=${cls}>${d === null ? "—" : fmtSigned(d, digits) + unit}</td>
                      </tr>`;
                    })}
                  </tbody>
                </table>
              </div>
              </div>
              <div className="coach-ask">
                ${coach.text
                  ? html`<div className="coach-text">
                      <div className="section-label">Coach <span className="tag">${coach.model}</span></div>
                      <p className="feedback"><${TurnText} text=${coach.text} /></p>
                    </div>`
                  : coach.error
                    ? html`<p className="bad">${coach.error}</p>`
                    : null}
                <button className="btn btn-primary btn-sm" onClick=${askCoach} disabled=${coach.loading || stale}>
                  ${coach.loading ? html`<span className="spinner"></span> Coach is thinking…` : coach.text ? "Ask again" : "Ask the coach about this corner"}
                </button>
              </div>
            </div>`}
    </div>`;
  }

  // ---------- Gears & shifts ----------

  const fmtRpm = (v) => (isNum(v) ? Math.round(v).toLocaleString() : "—");

  function afterTurn(turns, pct) {
    const before = turns.filter((t) => t.pct <= pct);
    return before.length ? `after T${before[before.length - 1].label}` : "after the start";
  }

  /** Upshift RPMs by gear against the shift light, the analysed lap's shifts, and the rev limiter. */
  function ShiftsCard({ shifts, selected, turns, onJump }) {
    const turnCtx = useContext(TurnContext);
    const hoverPct = (pct) => turnCtx && turnCtx.setHover(pct === null ? null : { pct });
    const ref = shifts.reference_rpm;
    const tol = shifts.tolerance_rpm;
    const pairs = shifts.pairs;
    const lo = Math.min(...pairs.map((p) => p.min_rpm), isNum(ref) ? ref - tol * 2 : Infinity) - 150;
    const hi = Math.max(...pairs.map((p) => p.max_rpm), isNum(ref) ? ref + tol * 2 : -Infinity, isNum(shifts.redline_rpm) ? shifts.redline_rpm : -Infinity) + 150;
    const pos = (v) => `${((v - lo) / (hi - lo)) * 100}%`;
    const mine = shifts.upshifts.filter((s) => s.lap === selected);
    const limiter = shifts.limiter.find((l) => l.lap === selected);

    return html`<section className="card">
      <div className="card-head">
        <div>
          <div className="card-title">Gears & shifts</div>
          <div className="faint" style=${{ fontSize: "12px", marginTop: "2px" }}>
            ${isNum(shifts.shift_rpm) ? `Shift light ${fmtRpm(shifts.shift_rpm)} rpm` : "No shift light in this file"}
            ${isNum(shifts.redline_rpm) ? ` · redline ${fmtRpm(shifts.redline_rpm)} rpm` : ""}
          </div>
        </div>
        <span className="legend">
          <span><i className="shift-key early"></i>Early</span>
          <span><i className="shift-key on_time"></i>On time</span>
          <span><i className="shift-key late"></i>Late</span>
          <span><i className="shift-key part_throttle"></i>Part throttle</span>
        </span>
      </div>
      <p className="muted shift-explain">
        Each upshift is judged by the peak RPM just before the change${ref ? html`, against ${shifts.reference_source} (${fmtRpm(ref)} rpm, ±${fmtRpm(tol)} counts as on time)` : ""}.
        Changing up too early leaves the engine below its power band in the next gear; too late wastes time near or on the limiter.
        Changes without full throttle (short-shifting for traction) aren't judged.
      </p>
      ${shifts.notes.length
        ? html`<ul className=${"note-list " + (shifts.notes.some((n) => n.includes("sooner") || n.includes("longer") || n.includes("limiter")) ? "bad" : "good")}>
            ${shifts.notes.map((t, i) => html`<li key=${i}><${TurnText} text=${t} /></li>`)}
          </ul>`
        : null}
      <div className="shift-layout">
        <div className="shift-table-wrap">
        <table className="corners shift-table">
          <thead>
            <tr>
              <th className="l">Shift</th>
              <th>Changes</th>
              <th>Typical</th>
              <th className="l rpm-col">RPM range, all counted laps</th>
              <th>Early</th>
              <th>Late</th>
            </tr>
          </thead>
          <tbody>
            ${pairs.map(
              (p) => html`<tr key=${p.from}>
                <td className="l"><span className="gear-badge sm" style=${{ background: gearColor(p.from) }}>${p.from}</span>→<span className="gear-badge sm" style=${{ background: gearColor(p.to) }}>${p.to}</span></td>
                <td className="faint">${p.count}</td>
                <td>${fmtRpm(p.median_rpm)}</td>
                <td className="l rpm-col">
                  <div className="rpm-range" title=${`${fmtRpm(p.min_rpm)}–${fmtRpm(p.max_rpm)} rpm`}>
                    ${isNum(ref) ? html`<span className="rpm-window" style=${{ left: pos(ref - tol), width: `calc(${pos(ref + tol)} - ${pos(ref - tol)})` }}></span>` : null}
                    ${isNum(shifts.redline_rpm) ? html`<span className="rpm-redline" style=${{ left: pos(shifts.redline_rpm) }}></span>` : null}
                    <span className="rpm-span" style=${{ left: pos(p.min_rpm), width: `calc(${pos(p.max_rpm)} - ${pos(p.min_rpm)} + 2px)` }}></span>
                    ${mine
                      .filter((s) => s.from === p.from && s.verdict !== "part_throttle")
                      .map((s, i) => html`<span key=${i} className=${"rpm-mine " + s.verdict} style=${{ left: pos(s.rpm) }} title=${`Lap ${selected}: ${fmtRpm(s.rpm)} rpm`}></span>`)}
                  </div>
                </td>
                <td className=${p.early ? "bad" : "faint"}>${p.early}</td>
                <td className=${p.late ? "bad" : "faint"}>${p.late}</td>
              </tr>`
            )}
          </tbody>
        </table>
        </div>
        <div className="shift-lap">
          <div className="section-label">Lap ${selected}'s upshifts</div>
          ${mine.length
            ? html`<div className="shift-chips">
                ${mine.map(
                  (s, i) => html`<button
                    key=${i}
                    className=${"shift-chip " + s.verdict}
                    onClick=${() => {
                      hoverPct(null);
                      onJump(s.pct);
                    }}
                    onMouseEnter=${() => hoverPct(s.pct)}
                    onMouseLeave=${() => hoverPct(null)}
                    title="Show on the map and traces"
                  >
                    <b>${s.from}→${s.to}</b> <span className="mono">${fmtRpm(s.rpm)}</span>
                    <span className="faint">${afterTurn(turns, s.pct)}</span>
                    ${VERDICT_LABEL[s.verdict] ? html`<span className="verdict">${VERDICT_LABEL[s.verdict]}</span>` : null}
                  </button>`
                )}
              </div>`
            : html`<p className="faint">No upshifts on this lap.</p>`}
          ${limiter ? html`<p className=${limiter.metres >= 20 ? "bad" : "faint"} style=${{ fontSize: "12px", marginTop: "8px" }}>${limiter.metres.toFixed(0)} m on the rev limiter this lap.</p>` : null}
          <p className="faint" style=${{ fontSize: "12px", marginTop: "8px" }}>Which gear is quicker through a corner: click the turn on the map. When your laps used different gears there, the breakdown compares them.</p>
        </div>
      </div>
    </section>`;
  }

  // ---------- Track section ----------

  const scrollToAnalysis = () => {
    const el = document.getElementById("track-analysis");
    if (el) el.scrollIntoView({ behavior: "smooth", block: "start" });
  };

  function TrackSection({ split, laps, selected, refNumber, excluded, model, traceCache, map, setMap, mapError, focus, onMapVisible }) {
    const turnCtx = useContext(TurnContext);
    const [traces, setTraces] = useState({});
    const [mode, setMode] = useState(() => stored("pcc.mapMode", "delta"));
    const [cursorPct, setCursorPct] = useState(null);
    const [activeTurn, setActiveTurn] = useState(null);
    const [view, setView] = useState([0, 1]);
    const [editing, setEditing] = useState(false);
    const [labels, setLabels] = useState([]);
    const traced = new Set(split.traced_laps || []);

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

    const [shifts, setShifts] = useState(null);
    const excludedKey = [...excluded].sort((a, b) => a - b).join(",");
    useEffect(() => {
      let cancelled = false;
      api(`/api/splits/${split.id}/shifts?exclude=${excludedKey}`)
        .then((r) => !cancelled && setShifts(r))
        .catch(() => !cancelled && setShifts(null));
      return () => {
        cancelled = true;
      };
    }, [split.id, excludedKey]);

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

    // A turn clicked elsewhere on the page (a chip in the coach's notes, the mini-map).
    useEffect(() => {
      if (!focus || !segments[focus.turn]) return;
      selectTurn(focus.turn);
      scrollToAnalysis();
    }, [focus]);

    // Tell RunView whether the main map is on screen (or still below it), so the mini-map only
    // shows once the main map has been scrolled past.
    const mapBoxRef = useRef(null);
    const hasMapBox = !!(map && sel);
    useEffect(() => {
      const el = mapBoxRef.current;
      if (!el) {
        onMapVisible(false);
        return;
      }
      // Cheap enough to run on every scroll event: React skips the update when nothing changed.
      const check = () => {
        const r = el.getBoundingClientRect();
        onMapVisible(r.bottom - r.height * 0.35 > 0);
      };
      check();
      window.addEventListener("scroll", check, { passive: true });
      window.addEventListener("resize", check);
      return () => {
        window.removeEventListener("scroll", check);
        window.removeEventListener("resize", check);
      };
    }, [hasMapBox]);

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
    const activeSeg = activeTurn !== null ? segments[activeTurn] : null;
    const offHere = !!(activeSeg && selLap && (selLap.off_track_pcts || []).some((p) => p >= activeSeg.start && p <= activeSeg.end));
    const ghost = sel && ref && isNum(cursorPct) ? ghostPct(sel, ref, cursorPct) : null;
    let ghostGap = null;
    if (ghost !== null) {
      // Wrap so a gap across the line reads as a small number, not a whole lap.
      let g = ghost - cursorPct;
      if (g > 0.5) g -= 1;
      if (g < -0.5) g += 1;
      ghostGap = g * map.length_m;
    }

    const hasAlt = !!(sel && sel.alt_m && sel.alt_m.length);
    const modeOptions = [
      { value: "delta", label: "Gain/loss" },
      { value: "speed", label: "Speed" },
      { value: "inputs", label: "Inputs" },
      { value: "gear", label: "Gear" },
      ...(hasAlt ? [{ value: "elevation", label: "Elevation" }] : []),
    ];
    const mapMode = modeOptions.some((o) => o.value === mode) ? mode : "delta";
    const gearsUsed = sel ? [...new Set(sel.gear)].filter((g) => g > 0).sort((a, b) => a - b) : [];
    const altRange = hasAlt ? Math.max(...sel.alt_m) - Math.min(...sel.alt_m) : 0;
    const zoomed = view[0] > 0 || view[1] < 1;
    const selShifts = shifts ? shifts.upshifts.filter((s) => s.lap === selected) : [];

    return html`<${React.Fragment}>
      <section className="card" id="track-analysis">
        <div className="card-head">
          <div>
            <div className="card-title">Track & telemetry</div>
            <div className="lap-vs">
              <span className="dot sel-bg"></span>Lap ${selected} ${selLap ? html`<span className="mono">${fmtLap(selLap.lap_time_s)}</span>` : null}
              <${OffTrackMark} lap=${selLap} />
              ${ref
                ? html`<span className="faint">vs</span><span className="dot ref-bg"></span>Lap ${refNumber} <span className="mono">${fmtLap(refLap && refLap.lap_time_s)}</span>
                    <${OffTrackMark} lap=${refLap} />
                    <span className=${"delta-chip " + deltaClass(totalDelta)}>${fmtDelta(totalDelta)}</span>`
                : html`<span className="faint">· pick a lap to compare with in the lap bar</span>`}
            </div>
          </div>
          <div className="head-actions">
            ${zoomed
              ? html`<button className="btn btn-sm" onClick=${() => selectTurn(null)}>
                  ${activeTurn !== null && map.turns[activeTurn] ? `Turn ${map.turns[activeTurn].label} · ` : ""}Show whole lap
                </button>`
              : null}
            <${Segmented}
              label="Map colouring"
              value=${mapMode}
              onChange=${(m) => {
                setMode(m);
                store("pcc.mapMode", m);
              }}
              options=${modeOptions}
            />
          </div>
        </div>
        ${!sel
          ? html`<p className="muted">Lap ${selected} wasn't a clean timed lap, so it has no telemetry trace. Pick another lap.</p>`
          : html`<div className="analysis-grid">
              <div className="analysis-side">
                <div ref=${mapBoxRef} className="map-box">
                <${TrackMapView}
                  map=${map}
                  sel=${sel}
                  refTrace=${ref}
                  mode=${mapMode}
                  cursorPct=${isNum(cursorPct) ? cursorPct : turnCtx && turnCtx.hover && isNum(turnCtx.hover.pct) ? turnCtx.hover.pct : null}
                  onHover=${setCursorPct}
                  activeTurn=${activeTurn}
                  hoverTurn=${turnCtx && turnCtx.hover ? turnCtx.hover.turn : null}
                  onTurnClick=${(i) => selectTurn(activeTurn === i ? null : i)}
                  selOff=${selLap && selLap.off_track_pcts}
                  refOff=${ref && refLap ? refLap.off_track_pcts : null}
                />
                </div>
                <div className="map-legend">
                  ${mapMode === "delta"
                    ? ref
                      ? html`<span><i className="good-bg"></i>Lap ${selected} gaining on lap ${refNumber}</span><span><i className="bad-bg"></i>losing</span>`
                      : html`<span className="faint">Pick a lap to compare with in the lap bar above</span>`
                    : mapMode === "speed"
                      ? html`<span className="speed-scale"></span><span className="faint">slow → fast</span>`
                      : mapMode === "gear"
                        ? gearsUsed.map((g) => html`<span key=${g}><span className="gear-badge sm" style=${{ background: gearColor(g) }}>${g}</span></span>`)
                        : mapMode === "elevation"
                          ? html`<span className="elev-scale"></span><span className="faint">low → high · ${altRange.toFixed(0)} m range</span>`
                          : html`<span><i style=${{ background: "#ff5f5f" }}></i>Braking</span><span><i style=${{ background: "#e8c547" }}></i>Part throttle</span><span><i style=${{ background: "#3ddc84" }}></i>Full throttle</span><span><i style=${{ background: "#6b7486" }}></i>Coast</span>`}
                </div>
                <div className="map-legend map-hover">
                  ${isNum(cursorPct)
                    ? html`<span><i className="sel-bg round"></i>Lap ${selected}</span>
                        ${ghostGap !== null
                          ? html`<span><i className="ref-bg round"></i>Lap ${refNumber}:
                              <b className=${Math.abs(ghostGap) < 1 ? "" : ghostGap > 0 ? "bad" : "good"}>
                                ${Math.abs(ghostGap) < 1 ? " level" : ` ${Math.abs(ghostGap).toFixed(0)} m ${ghostGap > 0 ? "ahead" : "behind"}`}
                              </b></span>`
                          : null}`
                    : html`<span className="faint">Hover the map or the traces · click a turn number to zoom in</span>`}
                  ${(selLap && offTracks(selLap)) || (ref && offTracks(refLap)) ? html`<span><span className="off-mark">!</span>off track</span>` : null}
                </div>
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
              <div className="analysis-main">
                ${activeTurn !== null && map.turns[activeTurn]
                  ? html`<${CornerCoach}
                      key=${activeTurn}
                      split=${split}
                      turnIndex=${activeTurn}
                      turnLabel=${map.turns[activeTurn].label}
                      selected=${selected}
                      refNumber=${ref ? refNumber : null}
                      excluded=${excluded}
                      model=${model}
                      offHere=${offHere}
                      onClose=${() => selectTurn(null)}
                    />`
                  : html`<div className="analysis-hint faint">
                      The map, grip circle and traces are linked: hover any of them to see the same moment everywhere.
                      Click a turn number on the map (or a row in the corner table) to zoom in on it and get an entry/exit breakdown.
                    </div>`}
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
                  shiftRpm=${shifts ? shifts.reference_rpm : split.shift_rpm}
                  upshifts=${selShifts}
                />
              </div>
            </div>`}
      </section>

      ${sel
        ? html`<section className="card">
            <div className="card-head">
              <div className="card-title">Corner by corner</div>
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
              onSelectTurn=${(i) => {
                selectTurn(i);
                if (i !== null) scrollToAnalysis();
              }}
              editing=${editing}
              labels=${labels}
              setLabels=${setLabels}
            />
            <p className="faint map-note">
              ${!ref ? "Pick a lap to compare with in the lap bar to see where you gain and lose time. " : ""}
              ${(map.source === "gps" ? "Map from GPS telemetry. " : "Map reconstructed from heading data. ") +
              "Turn numbers are auto-detected — rename them to match the official ones."}
            </p>
          </section>`
        : null}

      ${shifts && shifts.upshifts.length
        ? html`<${ShiftsCard} shifts=${shifts} selected=${selected} turns=${map.turns} onJump=${(pct) => {
            const w = 0.04;
            setActiveTurn(null);
            setView([Math.max(0, pct - w), Math.min(1, pct + w)]);
            setCursorPct(pct);
            scrollToAnalysis();
          }} />`
        : null}
    <//>`;
  }

  // ---------- Lap table ----------

  function LapTable({ laps, stats, excluded, selected, compare, onPick, onToggle }) {
    const nSectors = stats ? stats.nSectors : 0;
    const showTyres = laps.some((lap) => isNum(lap.tyre_temp_avg_c));
    const showIncidents = laps.some((lap) => isNum(lap.incidents));
    const showTrackTemp = laps.some((lap) => isNum(lap.track_temp_c));
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
            ${showIncidents ? html`<th title="Incident points picked up on the lap">Inc</th>` : null}
            ${showTyres ? html`<th>Tyre °C</th><th>Spread</th>` : null}
            ${showTrackTemp ? html`<th title="Average track temperature over the lap">Track °C</th>` : null}
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
                <${OffTrackMark} lap=${lap} />
              </td>
              <td className=${isBest ? "time-best" : ""}>${fmtLap(lap.lap_time_s)}</td>
              <td className=${isBest ? "purple" : deltaClass(delta)}>${isBest ? "—" : fmtDelta(delta)}</td>
              ${range(nSectors).map((i) => {
                const v = sectorOf(lap, i);
                const isSectorBest = !isExcluded && isNum(v) && v === stats.bestSectors[i];
                return html`<td key=${i} className=${isSectorBest ? "sector-best" : ""}>${fmtNum(v, 3)}</td>`;
              })}
              <td>${fmtNum(lap.avg_speed_kph, 1)}</td>
              ${showIncidents ? html`<td className=${lap.incidents > 0 ? "bad" : "faint"}>${isNum(lap.incidents) ? (lap.incidents > 0 ? lap.incidents + "x" : "0") : "—"}</td>` : null}
              ${showTyres ? html`<td>${fmtNum(lap.tyre_temp_avg_c, 1)}</td><td>${fmtNum(lap.tyre_temp_delta_c, 1)}</td>` : null}
              ${showTrackTemp ? html`<td title=${lapWeatherTitle(lap)}>${fmtNum(lap.track_temp_c, 1)}${lap.track_wetness > 1 ? html` <span className="tag tag-wet">wet</span>` : null}</td>` : null}
            </tr>`;
          })}
        </tbody>
      </table>
    </div>`;
  }

  // ---------- Lap detail ----------

  function Metric({ label, value, delta, unit, digits = 1, higherIsBetter, neutral }) {
    const hasDelta = isNum(delta) && Math.abs(delta) >= 0.05;
    const better = hasDelta && (higherIsBetter ? delta > 0 : delta < 0);
    return html`<div className="metric">
      <div className="k">${label}</div>
      <div className="v">${fmtNum(value, digits, unit)}</div>
      <div className=${"dv " + (hasDelta ? (neutral ? "faint" : better ? "good" : "bad") : "faint")}>
        ${hasDelta ? (delta > 0 ? "+" : "−") + Math.abs(delta).toFixed(digits) : " "}
      </div>
    </div>`;
  }

  function LapDetail({ laps, insights, stats, selected, compare, refNumber }) {
    const lap = laps.find((l) => l.lap_number === selected);
    if (!lap) {
      return html`<section className="card"><p className="muted">Pick a lap to see the breakdown.</p></section>`;
    }
    const ref = laps.find((l) => l.lap_number === refNumber);
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
          <div className="kpi-label">Lap ${lap.lap_number} ${!lap.is_complete ? "· untimed" : ""} <${OffTrackMark} lap=${lap} /></div>
          <div className="detail-time">${fmtLap(lap.lap_time_s)}</div>
        </div>
        ${!isSelf && delta !== null
          ? html`<span className=${"delta-chip " + deltaClass(delta)}>${fmtDelta(delta)}</span>`
          : isSelf
            ? html`<span className="tag tag-best">reference</span>`
            : null}
      </div>

      ${ref && !isSelf
        ? html`<div className="vs-row muted">
            <span className="vs-dot ref-bg"></span>
            vs lap ${ref.lap_number} <span className="mono">${fmtLap(ref.lap_time_s)}</span>
            <span className="faint">${compare === null ? (stats && ref.lap_number === stats.best.lap_number ? "· your best" : "· your next best") : ""}</span>
          </div>`
        : null}

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
        ${isNum(lap.track_temp_c) ? html`<${Metric} label="Track temp" value=${lap.track_temp_c} delta=${diff("track_temp_c")} unit="°" neutral=${true} />` : null}
        ${isNum(lap.air_temp_c) ? html`<${Metric} label="Air temp" value=${lap.air_temp_c} delta=${diff("air_temp_c")} unit="°" neutral=${true} />` : null}
        ${lap.track_wetness > 1 ? html`<div className="metric"><div className="k">Track</div><div className="v wet">${WETNESS[lap.track_wetness]}</div></div>` : null}
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

  function RunView({ split, laps, model }) {
    const [excluded, setExcluded] = useState(() => new Set());
    const [selected, setSelected] = useState(null);
    const [compare, setCompare] = useState(null);
    const traceCache = useRef(new Map());

    // The track map is shared: the track section draws it, and turn chips and the mini-map
    // anywhere on the page point at it.
    const [map, setMap] = useState(null);
    const [mapError, setMapError] = useState(null);
    const [hover, setHover] = useState(null);
    const [focus, setFocus] = useState(null);
    const [mainMapVisible, setMainMapVisible] = useState(false);
    useEffect(() => {
      if (!split.has_track) return;
      api(`/api/splits/${split.id}/track`).then(setMap).catch((err) => setMapError(err.message));
    }, [split.id]);
    const turnCtx = useMemo(
      () => (map && map.turns.length ? { turns: map.turns, hover, setHover, focusTurn: (turn) => setFocus({ turn, at: Date.now() }) } : null),
      [map, hover]
    );

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

    // Picking the comparison lap as the analysed lap swaps them rather than comparing a lap with itself.
    const selectLap = useCallback(
      (lapNumber) => {
        if (lapNumber === compare) setCompare(selected);
        setSelected(lapNumber);
      },
      [compare, selected]
    );

    const pick = useCallback(
      (lapNumber, asCompare) => {
        if (asCompare) {
          if (lapNumber !== selected) setCompare((current) => (current === lapNumber ? null : lapNumber));
        } else {
          selectLap(lapNumber);
        }
      },
      [selected, selectLap]
    );

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
          if (idx < laps.length - 1) selectLap(laps[idx + 1].lap_number);
        } else if (e.key === "ArrowLeft") {
          e.preventDefault();
          if (idx > 0) selectLap(laps[idx - 1].lap_number);
        } else if ((e.key === "b" || e.key === "B") && stats) {
          selectLap(stats.best.lap_number);
        }
      }
      window.addEventListener("keydown", onKey);
      return () => window.removeEventListener("keydown", onKey);
    }, [laps, selected, stats, selectLap]);

    const subtitle = [
      split.car,
      fmtTimeOfDay(split.started_at_ms) + (split.source === "Live" ? " – " + fmtTimeOfDay(split.ended_at_ms) : ""),
      `${laps.filter((l) => l.is_complete).length} timed laps`,
      excluded.size ? `${excluded.size} excluded from stats` : null,
      (() => {
        const n = laps.filter((l) => offTracks(l)).length;
        return n ? `${n} lap${n === 1 ? "" : "s"} with an off-track` : null;
      })(),
    ]
      .filter(Boolean)
      .join(" · ");

    return html`<${TurnContext.Provider} value=${turnCtx}>
      <div className="page-head">
        <div>
          <h1>${runTitle(split)}</h1>
          <p>${subtitle}</p>
          <${WeatherStrip} weather=${split.summary && split.summary.weather} />
        </div>
      </div>

      <${LapBar}
        laps=${laps}
        stats=${stats}
        excluded=${excluded}
        selected=${selected}
        compare=${compare}
        refNumber=${refNumber}
        autoRef=${autoRef}
        setSelected=${selectLap}
        setCompare=${setCompare}
        onPick=${pick}
      />

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
          <div className="card-title">Pace</div>
          <div className="legend">
            <span><i style=${{ background: "var(--purple)" }}></i>Session best</span>
            <span><i className="sel-bg"></i>Analysing</span>
            <span><i className="ref-bg"></i>Compared with</span>
            <span><span className="off-mark">!</span>Off track</span>
            ${trackTempSpan(laps) ? html`<span><i className="temp-key"></i>Track temp (right axis)</span>` : null}
          </div>
        </div>
        <${PaceChart} laps=${laps} stats=${stats} excluded=${excluded} selected=${selected} compare=${refNumber} onPick=${pick} />
      </section>

      <${TrackSection}
        split=${split}
        laps=${laps}
        selected=${selected}
        refNumber=${refNumber}
        excluded=${excluded}
        model=${model}
        traceCache=${traceCache}
        map=${map}
        setMap=${setMap}
        mapError=${mapError}
        focus=${focus}
        onMapVisible=${setMainMapVisible}
      />

      <div className="split-2">
        <section className="card">
          <div className="card-head">
            <div className="card-title">Lap times</div>
            <span className="faint" style=${{ fontSize: "12px" }}>Untick out laps or spins to leave them out of the stats</span>
          </div>
          <${LapTable} laps=${laps} stats=${stats} excluded=${excluded} selected=${selected} compare=${refNumber} onPick=${pick} onToggle=${toggle} />
        </section>
        <${LapDetail} laps=${laps} insights=${insights} stats=${stats} selected=${selected} compare=${compare} refNumber=${refNumber} />
      </div>

      <div className="coach">
        <section className="card">
          <div className="card-head"><div className="card-title">Next time out</div></div>
          <ol className="suggestions">
            ${split.suggestions.map((t, i) => html`<li key=${i}><b>${i + 1}</b><span><${TurnText} text=${t} /></span></li>`)}
          </ol>
        </section>
        <section className="card">
          <div className="card-head">
            <div className="card-title">Coach feedback</div>
            <span className="tag">${split.model}</span>
          </div>
          <p className="feedback"><${TurnText} text=${split.feedback} /></p>
        </section>
      </div>

      ${map && map.turns.length ? html`<${MiniMap} map=${map} title=${split.track_label} hidden=${mainMapVisible} />` : null}
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
            ? html`<${RunView} key=${split.id} split=${split} laps=${laps} model=${model} />`
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
