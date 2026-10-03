import { OffTrackMark, api, deltaClass, fastest, fmtAgo, fmtClock, fmtDelta, fmtLap, fmtTimeOfDay, html, parseTelemetryName, post, request, stdDev, store, stored, useCallback, useEffect, useRef, useState } from "./js/lib.js";
import { RunView, runTitle } from "./js/run.js";

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
            ? html`<p className="muted pad">No .ibt files in this folder.</p>`
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
          <input className="input mono" value=${path} onInput=${(e) => setPath(e.target.value)} placeholder="Path to a .ibt file" aria-label="File path" />
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

function Sidebar({ splits, selectedId, onSelect, onDelete }) {
  const ordered = splits.slice().reverse();
  return html`<aside className="sidebar">
    <div className="section-label"><span>Runs</span><span>${splits.length}</span></div>
    ${ordered.length === 0
      ? html`<p className="faint fs-13">No runs yet. Record a stint or open a session file.</p>`
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
  const best = fastest(complete);
  const last = laps.length ? laps[laps.length - 1] : null;
  const lastDelta = best && last && last.is_complete ? last.lap_time_s - best.lap_time_s : null;
  const sd = stdDev(complete.map((lap) => lap.lap_time_s));

  return html`<section className="card live">
    <div className="card-head">
      <div className="card-title">Live run</div>
      <span className="faint fs-12">Laps appear as you cross the line</span>
    </div>
    <div className="live-grid">
      <div>
        <div className="kpi-label">Last lap</div>
        <div className="live-big">${last ? fmtLap(last.lap_time_s) : "—"}</div>
        <div className=${"mono fs-13 " + deltaClass(lastDelta)}>
          ${last && !last.is_complete ? "not timed" : lastDelta !== null ? fmtDelta(lastDelta) + " to best" : " "}
        </div>
      </div>
      <div>
        <div className="kpi-label">Best this run</div>
        <div className="live-big purple">${best ? fmtLap(best.lap_time_s) : "—"}</div>
        <div className="faint fs-13">${best ? "Lap " + best.lap_number : " "}</div>
      </div>
      <div>
        <div className="kpi-label">Consistency</div>
        <div className="live-big">${sd !== null ? "±" + sd.toFixed(3) : "—"}</div>
        <div className="faint fs-13">std dev</div>
      </div>
      <div>
        <div className="kpi-label">Timed laps</div>
        <div className="live-big">${complete.length}</div>
        <div className="faint fs-13">${laps.length - complete.length ? `+ ${laps.length - complete.length} untimed` : " "}</div>
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
  const [status, setStatus] = useState({ is_recording: false });
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
      await post("/api/recording/start", { model: model || null });
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
      const result = await post("/api/recording/stop");
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
      const result = await post("/api/splits/import", { path, model: model || null });
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
      await request(`/api/splits/${id}`, { method: "DELETE" });
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
