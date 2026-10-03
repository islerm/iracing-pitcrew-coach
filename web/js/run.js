import { Kpi, OffTrackMark, SKY_ICON, Segmented, WETNESS, api, computeStats, deltaClass, fastest, fmtDelta, fmtLap, fmtNum, fmtTempRange, fmtTimeOfDay, html, isNum, lapWeatherTitle, offTrackTitle, offTracks, post, range, rangeTitle, sameConditions, sectorOf, trackTempSpan, useCallback, useContext, useEffect, useMemo, useRef, useState, useStored } from "./lib.js";
import { MapRef, MiniMap, TrackMapView, TurnContext, TurnScope, TurnText, gearColor, ghostPct, idxOf, mapIndex, turnIndex } from "./trackmap.js";
import { GripCircle, PaceChart, TraceChart } from "./charts.js";
import { CornerCoach, CornerTable, ShiftsCard } from "./corners.js";
import { RadioToggle, SpeakButton } from "./voice.js";
import { officialTurns, rotatedMap } from "./officialmap.js";

export function runTitle(split) {
  if (split.track_label) return split.track_label;
  return split.source === "Live" ? `Run ${split.id.replace("split-", "")}` : split.source;
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

// ---------- Weather ----------

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

// ---------- Track section ----------

const scrollToAnalysis = () => {
  const el = document.getElementById("track-analysis");
  if (el) el.scrollIntoView({ behavior: "smooth", block: "start" });
};

const NO_SEGMENTS = [];

function TrackSection({ split, laps, selected, refNumber, excludedList, model, map, setMap, mapError, focus, onMapVisible }) {
  const turnCtx = useContext(TurnContext);
  const segments = turnCtx ? turnCtx.segments : NO_SEGMENTS;
  const [traces, setTraces] = useState({});
  const [mode, setMode] = useStored("pcc.mapMode", "delta");
  const [cursorPct, setCursorPct] = useState(null);
  const [activeTurn, setActiveTurn] = useState(null);
  const [view, setView] = useState([0, 1]);
  const [editing, setEditing] = useState(false);
  const [labels, setLabels] = useState([]);
  const tracedLaps = split.traced_laps || [];

  const selTraced = tracedLaps.includes(selected);
  const refTraced = refNumber !== null && refNumber !== selected && tracedLaps.includes(refNumber);

  useEffect(() => {
    [selTraced ? selected : null, refTraced ? refNumber : null].forEach((lap) => {
      if (lap === null || traces[lap]) return;
      api(`/api/splits/${split.id}/laps/${lap}/trace`)
        .then((trace) => setTraces((t) => ({ ...t, [lap]: trace })))
        .catch(() => {});
    });
  }, [split.id, selected, refNumber]);

  const sel = selTraced ? traces[selected] : null;
  const ref = refTraced ? traces[refNumber] : null;
  const corners = useMemo(() => (map && sel ? cornerStats(segments, sel, ref, map.length_m, split.peak_g) : []), [map, segments, sel, ref, split.peak_g]);
  const gearsUsed = useMemo(() => (sel ? [...new Set(sel.gear)].filter((g) => g > 0).sort((a, b) => a - b) : []), [sel]);
  const altRange = useMemo(() => (sel && sel.alt_m && sel.alt_m.length ? Math.max(...sel.alt_m) - Math.min(...sel.alt_m) : 0), [sel]);

  const [shifts, setShifts] = useState(null);
  useEffect(() => {
    let cancelled = false;
    api(`/api/splits/${split.id}/shifts?exclude=${excludedList.join(",")}`)
      .then((r) => !cancelled && setShifts(r))
      .catch(() => !cancelled && setShifts(null));
    return () => {
      cancelled = true;
    };
  }, [split.id, excludedList]);
  const selShifts = useMemo(() => (shifts ? shifts.upshifts.filter((s) => s.lap === selected) : []), [shifts, selected]);

  const selectTurn = useCallback(
    (i) => {
      setActiveTurn(i);
      if (i === null) {
        setView([0, 1]);
      } else {
        const seg = segments[i];
        const margin = (seg.end - seg.start) * 0.15;
        setView([Math.max(0, seg.start - margin), Math.min(1, seg.end + margin)]);
      }
    },
    [segments]
  );
  const onTurnClick = useCallback((i) => selectTurn(activeTurn === i ? null : i), [selectTurn, activeTurn]);
  const onTableSelect = useCallback(
    (i) => {
      selectTurn(i);
      if (i !== null) scrollToAnalysis();
    },
    [selectTurn]
  );
  const onShiftJump = useCallback((pct) => {
    const w = 0.04;
    setActiveTurn(null);
    setView([Math.max(0, pct - w), Math.min(1, pct + w)]);
    setCursorPct(pct);
    scrollToAnalysis();
  }, []);

  // A turn clicked elsewhere on the page (a chip in the coach's notes, the mini-map).
  useEffect(() => {
    if (!focus) return;
    const span = isNum(focus.sector) && turnCtx ? turnCtx.sectors[focus.sector] : null;
    if (span) {
      setActiveTurn(null);
      setView([span.start, span.end]);
    } else if (segments[focus.turn]) {
      selectTurn(focus.turn);
    } else {
      return;
    }
    scrollToAnalysis();
  }, [focus]);

  // Tell RunView whether the main map is on screen (or still below it), so the mini-map only
  // shows once the main map has been scrolled past: it's "visible" until 65% of it has left the top.
  const mapBoxRef = useRef(null);
  const hasMapBox = !!(map && sel);
  useEffect(() => {
    const el = mapBoxRef.current;
    if (!el) {
      onMapVisible(false);
      return undefined;
    }
    // The root extends far below the window, so a map that hasn't been reached yet counts as visible.
    const observer = new IntersectionObserver(([entry]) => onMapVisible(entry.intersectionRatio >= 0.35), {
      rootMargin: "0px 0px 100000px 0px",
      threshold: [0.35],
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, [hasMapBox]);

  const [syncing, setSyncing] = useState(false);

  // Clicks can come faster than the server answers, so count from the last requested angle.
  const rotationRef = useRef(0);
  useEffect(() => {
    if (map) rotationRef.current = map.rotation_deg || 0;
  }, [map && map.rotation_deg]);

  async function rotate(by) {
    rotationRef.current = (rotationRef.current + by + 360) % 360;
    try {
      setMap(await post(`/api/splits/${split.id}/track/rotation`, { rotation_deg: rotationRef.current }));
    } catch (err) {
      window.alert(err.message);
    }
  }

  async function syncOfficial() {
    if (map.turns_source === "manual" && !window.confirm("Replace your edited turn labels with iRacing's turn numbers?")) return;
    setSyncing(true);
    try {
      // `map` is drawn rotated already, so the fit's rotation is on top of that.
      const official = await officialTurns(map);
      await post(`/api/splits/${split.id}/track/rotation`, { rotation_deg: (map.rotation_deg || 0) + official.rotation_deg });
      setMap(await post(`/api/splits/${split.id}/track/turns`, { turns: official.turns, official: true }));
    } catch (err) {
      window.alert(`Couldn't use iRacing's map: ${err.message}`);
    } finally {
      setSyncing(false);
    }
  }

  async function saveLabels() {
    try {
      const turns = map.turns.map((t, i) => ({ pct: t.pct, label: (labels[i] || "").trim() || t.label }));
      const updated = await post(`/api/splits/${split.id}/track/turns`, { turns });
      setMap(updated);
      setEditing(false);
    } catch (err) {
      window.alert(err.message);
    }
  }

  if (!split.has_track) {
    return html`<section className="card"><p className="muted">No complete laps with telemetry in this run, so there's no track map yet.</p></section>`;
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
  const zoomed = view[0] > 0 || view[1] < 1;

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
            onChange=${setMode}
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
                cursorPct=${cursorPct}
                onHover=${setCursorPct}
                activeTurn=${activeTurn}
                onTurnClick=${onTurnClick}
                selOff=${selLap && selLap.off_track_pcts}
                refOff=${ref && refLap ? refLap.off_track_pcts : null}
              />
              <div className="map-tools">
                <button className="btn btn-ghost btn-icon btn-sm" title="Rotate the map 90° anticlockwise" aria-label="Rotate map anticlockwise" onClick=${() => rotate(-90)}>⟲</button>
                <button className="btn btn-ghost btn-icon btn-sm" title="Rotate the map 90° clockwise" aria-label="Rotate map clockwise" onClick=${() => rotate(90)}>⟳</button>
                ${map.track_id
                  ? html`<button
                      className="btn btn-ghost btn-sm"
                      disabled=${syncing}
                      title=${map.turns_source === "official" ? "Turn numbers and layout come from iRacing's track map. Click to fetch them again." : "Use iRacing's turn numbers and map layout"}
                      onClick=${syncOfficial}
                    >${syncing ? html`<span className="spinner"></span>` : map.turns_source === "official" ? "✓ iRacing turns" : "Use iRacing turns"}</button>`
                  : null}
              </div>
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
                    excludedList=${excludedList}
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
            onSelectTurn=${onTableSelect}
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
      ? html`<${ShiftsCard} shifts=${shifts} selected=${selected} onJump=${onShiftJump} />`
      : null}
  <//>`;
}

// ---------- Lap table ----------

function LapTable({ laps, stats, excluded, selected, compare, onPick, onToggle }) {
  const nSectors = stats ? stats.nSectors : 0;
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
          ${range(nSectors).map((i) => html`<th key=${i}><${MapRef} kind="sector" index=${i}>S${i + 1}<//></th>`)}
          <th>Avg kph</th>
          ${showIncidents ? html`<th title="Incident points picked up on the lap">Inc</th>` : null}
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
            ${showTrackTemp ? html`<td title=${lapWeatherTitle(lap)}>${fmtNum(lap.track_temp_c, 1)}${lap.track_wetness > 1 ? html` <span className="tag tag-wet">wet</span>` : null}</td>` : null}
          </tr>`;
        })}
      </tbody>
    </table>
  </div>`;
}

// ---------- Lap detail ----------

function Metric({ label, value, delta, unit, digits = 1, higherIsBetter, neutral }) {
  // Only show a difference that survives rounding to the displayed digits.
  const hasDelta = isNum(delta) && Math.abs(delta) >= 0.5 * 10 ** -digits;
  const better = hasDelta && (higherIsBetter ? delta > 0 : delta < 0);
  return html`<div className="metric">
    <div className="k">${label}</div>
    <div className="v">${fmtNum(value, digits, unit)}</div>
    <div className=${"dv " + (hasDelta ? (neutral ? "faint" : better ? "good" : "bad") : "faint")}>
      ${hasDelta ? fmtDelta(delta, digits) : " "}
    </div>
  </div>`;
}

function LapDetail({ laps, stats, selected, compare, refNumber }) {
  const turnCtx = useContext(TurnContext);
  const lap = laps.find((l) => l.lap_number === selected);
  if (!lap) {
    return html`<section className="card"><p className="muted">Pick a lap to see the breakdown.</p></section>`;
  }
  const ref = laps.find((l) => l.lap_number === refNumber);
  const isSelf = ref && ref.lap_number === lap.lap_number;
  const delta = ref && lap.is_complete && ref.is_complete ? lap.lap_time_s - ref.lap_time_s : null;
  const sectorTurns = (i) => (turnCtx && turnCtx.sectors[i] ? turnCtx.sectors[i].turns : null);

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
              <span className="name">
                <${MapRef} kind="sector" index=${s.i - 1}>S${s.i}<//>
                ${sectorTurns(s.i - 1) ? html`<span className="sector-turns">${sectorTurns(s.i - 1)}</span>` : null}
              </span>
              <div className="bar">
                ${s.d !== null && Math.abs(s.d) >= 0.0005
                  ? html`<span className=${s.d < 0 ? "good" : "bad"} style=${{ width: `${(Math.abs(s.d) / maxAbs) * 50}%` }}></span>`
                  : null}
              </div>
              <span className=${"val " + deltaClass(s.d)}>${fmtDelta(s.d)}</span>
            </div>`
          )}
          ${biggestLoss && biggestLoss.d > 0.01
            ? html`<p className="muted fs-13">
                Biggest loss in <b style=${{ color: "var(--text)" }}><${MapRef} kind="sector" index=${biggestLoss.i - 1}>sector ${biggestLoss.i}<//></b>
                ${sectorTurns(biggestLoss.i - 1) ? ` (${sectorTurns(biggestLoss.i - 1)})` : ""} · ${fmtDelta(biggestLoss.d)}.
              </p>`
            : null}
        </div>`
      : !isSelf && !(stats && stats.nSectors)
        ? html`<p className="faint fs-12">No sector splits in this data source.</p>`
        : null}

    <div className="metric-grid">
      <${Metric} label="Avg speed" value=${lap.avg_speed_kph} delta=${diff("avg_speed_kph")} higherIsBetter=${true} />
      ${isNum(lap.top_speed_kph) ? html`<${Metric} label="Top speed" value=${lap.top_speed_kph} delta=${diff("top_speed_kph")} higherIsBetter=${true} />` : null}
      ${isNum(lap.full_throttle_pct)
        ? html`<${Metric} label="Full throttle" value=${lap.full_throttle_pct} delta=${diff("full_throttle_pct")} unit="%" digits=${0} higherIsBetter=${true} />`
        : null}
      ${isNum(lap.braking_pct) ? html`<${Metric} label="Braking" value=${lap.braking_pct} delta=${diff("braking_pct")} unit="%" digits=${0} neutral=${true} />` : null}
      ${isNum(lap.track_temp_c) ? html`<${Metric} label="Track temp" value=${lap.track_temp_c} delta=${diff("track_temp_c")} unit="°" neutral=${true} />` : null}
      ${isNum(lap.air_temp_c) ? html`<${Metric} label="Air temp" value=${lap.air_temp_c} delta=${diff("air_temp_c")} unit="°" neutral=${true} />` : null}
      ${lap.track_wetness > 1 ? html`<div className="metric"><div className="k">Track</div><div className="v wet">${WETNESS[lap.track_wetness]}</div></div>` : null}
    </div>

    ${lap.went_well || lap.went_bad
      ? html`<div className="notes">
          <div>
            <div className="section-label" style=${{ marginBottom: "6px" }}>Went well</div>
            <ul className="note-list good">${(lap.went_well || []).map((t, i) => html`<li key=${i}>${t}</li>`)}</ul>
          </div>
          <div>
            <div className="section-label" style=${{ marginBottom: "6px" }}>To work on</div>
            <ul className="note-list bad">${(lap.went_bad || []).map((t, i) => html`<li key=${i}>${t}</li>`)}</ul>
          </div>
        </div>`
      : null}
  </section>`;
}

// ---------- Run view ----------

/**
 * The coach model's write-up. It's made on request rather than when the run is saved (the model
 * is slow and takes a lot of memory): automatically on opening the run, unless a recording is
 * running, so the model never loads while you're driving.
 */
function CoachFeedback({ split, model }) {
  const [done, setDone] = useState(() => (split.feedback ? { feedback: split.feedback, model: split.model } : null));
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState(null);

  async function ask() {
    setLoading(true);
    setError(null);
    try {
      setDone(await post(`/api/splits/${split.id}/feedback`, { model: model || null }));
    } catch (err) {
      setError(err.message);
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    if (done) return undefined;
    let cancelled = false;
    api("/api/status")
      .then((status) => {
        if (!cancelled && !status.is_recording) ask();
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [split.id]);

  return html`<section className="card">
    <div className="card-head">
      <div className="card-title">Coach feedback</div>
      <span className="head-actions">
        ${done
          ? html`<${RadioToggle} />
              <${SpeakButton} text=${done.feedback} label="Hear the coach feedback" />`
          : null}
        <span className="tag">${done ? done.model : model || split.model}</span>
      </span>
    </div>
    ${done
      ? html`<p className="feedback"><${TurnText} text=${done.feedback} /></p>`
      : loading
        ? html`<p className="muted"><span className="spinner"></span> The coach is writing this up…</p>`
        : html`<div>
            <p className=${error ? "bad" : "muted"} style=${{ marginBottom: "8px" }}>
              ${error || "The coach writes this up when you ask, so the model isn't loaded while you're driving."}
            </p>
            <button className="btn btn-sm" onClick=${ask}>Ask the coach</button>
          </div>`}
  </section>`;
}

/** The pit engineer's call after each lap, oldest first; click a lap to analyse it. */
function RadioTranscript({ entries, laps, onOpenLap }) {
  const lapOf = (n) => laps.find((l) => l.lap_number === n);
  const all = entries.map((e) => `Lap ${e.lap_number}. ${e.text}`).join("\n");
  const anyAfter = entries.some((e) => !e.live);
  return html`<section className="card">
    <div className="card-head">
      <div className="card-title">Radio transcript</div>
      ${entries.length
        ? html`<span className="head-actions">
            <${RadioToggle} />
            <${SpeakButton} text=${all} label="Hear the whole transcript" />
          </span>`
        : null}
    </div>
    ${entries.length === 0
      ? html`<p className="muted">No radio calls for this run: it has no laps the engineer could talk about.</p>`
      : html`<ol className="radio-log">
          ${entries.map((e) => {
            const lap = lapOf(e.lap_number);
            if (e.kind === "pit") {
              return html`<li key=${e.lap_number + "pit"} className="pit">
                <div className="radio-lap"><span>Pit debrief</span><span className="mono">after L${e.lap_number}</span></div>
                <div className="radio-text">
                  ${e.live ? html`<span className="tag tag-live" title="Said over the radio during the run">📻 live</span>` : null}
                  <${TurnText} text=${e.text} />
                </div>
                <${SpeakButton} text=${e.text} label="Hear the debrief" />
              </li>`;
            }
            return html`<li key=${e.lap_number + "lap"}>
              <button className="radio-lap" onClick=${() => onOpenLap(e.lap_number)} title=${`Analyse lap ${e.lap_number}`}>
                <span>L${e.lap_number}</span>
                <span className="mono">${lap ? fmtLap(lap.lap_time_s) : ""}</span>
                <${OffTrackMark} lap=${lap} />
              </button>
              <div className="radio-text">
                ${e.live ? html`<span className="tag tag-live" title="Said over the radio during the run">📻 live</span>` : null}
                <${TurnText} text=${e.text} />
              </div>
              <${SpeakButton} text=${e.text} label="Hear this call" />
            </li>`;
          })}
        </ol>`}
    ${anyAfter
      ? html`<p className="faint fs-12">
          Calls without the 📻 tag were worked out after the run: what the engineer would have said at the time, from the laps driven up to then.
        </p>`
      : null}
  </section>`;
}

export function RunView({ split, laps, model }) {
  const [excluded, setExcluded] = useState(() => new Set());
  const [selected, setSelected] = useState(null);
  const [compare, setCompare] = useState(null);

  // The track map is shared: the track section draws it, and turn chips and the mini-map
  // anywhere on the page point at it.
  const [map, setMap] = useState(null);
  const [mapError, setMapError] = useState(null);
  const [focus, setFocus] = useState(null);
  const [mainMapVisible, setMainMapVisible] = useState(false);
  const [tab, setTab] = useStored("pcc.runTab", "analysis");
  useEffect(() => {
    if (!split.has_track) return;
    let cancelled = false;
    api(`/api/splits/${split.id}/track`)
      .then(async (loaded) => {
        if (cancelled) return;
        setMap(loaded);
        // First time on a track: swap the detected corners for iRacing's own turn numbers,
        // and draw the map the way iRacing does.
        if (loaded.turns_source !== "detected" || !loaded.track_id) return;
        try {
          const official = await officialTurns(loaded);
          await post(`/api/splits/${split.id}/track/rotation`, { rotation_deg: official.rotation_deg });
          const updated = await post(`/api/splits/${split.id}/track/turns`, { turns: official.turns, official: true });
          if (!cancelled) setMap(updated);
        } catch (err) {
          console.warn("Official turn numbers unavailable:", err.message);
        }
      })
      .catch((err) => setMapError(err.message));
    return () => {
      cancelled = true;
    };
  }, [split.id]);
  // Drawn turned to match iRacing's map; lap fractions (and so turns) are unaffected.
  const viewMap = useMemo(() => map && rotatedMap(map), [map]);
  const turnBase = useMemo(
    () =>
      map && {
        ...mapIndex(map),
        focusTurn: (turn) => setFocus({ turn, at: Date.now() }),
        focusSector: (sector) => setFocus({ sector, at: Date.now() }),
      },
    [map]
  );

  // Start on the most recent timed lap.
  useEffect(() => {
    const complete = laps.filter((lap) => lap.is_complete);
    const last = complete.length ? complete[complete.length - 1] : laps[laps.length - 1];
    setSelected(last ? last.lap_number : null);
  }, [split.id]);

  const stats = useMemo(() => computeStats(laps, excluded), [laps, excluded]);
  const excludedList = useMemo(() => [...excluded].sort((a, b) => a - b), [excluded]);

  // Reference lap: the one picked with Shift+click, otherwise your best lap on a similar surface
  // (a dry lap isn't measured against a wet one) — or the next-best when that's the one selected.
  const autoRef = useMemo(() => {
    if (!stats) return null;
    const selLap = laps.find((l) => l.lap_number === selected);
    const others = stats.counted.filter((l) => l.lap_number !== selected && sameConditions(l, selLap));
    const best = fastest(others);
    return best ? best.lap_number : null;
  }, [stats, selected, laps]);
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

  const focusSector = stats && stats.focus && turnBase && turnBase.sectors[stats.focus.sector - 1];
  const focusTurns = focusSector ? focusSector.turns : null;

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

  const radio = split.radio || [];
  const openLap = (n) => {
    selectLap(n);
    setTab("analysis");
  };

  return html`<${TurnScope} base=${turnBase}>
    <div className="page-head">
      <div>
        <h1>${runTitle(split)}</h1>
        <p>${subtitle}</p>
        <${WeatherStrip} weather=${split.weather} />
      </div>
      <${Segmented}
        label="View"
        value=${tab}
        onChange=${setTab}
        options=${[
          { value: "analysis", label: "Analysis" },
          { value: "radio", label: `Radio${radio.length ? ` · ${radio.length}` : ""}` },
        ]}
      />
    </div>

    ${tab === "radio"
      ? html`<${RadioTranscript} entries=${radio} laps=${laps} onOpenLap=${openLap} />`
      : html`<${React.Fragment}>
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
                value=${html`<${MapRef} kind="sector" index=${stats.focus.sector - 1}>Sector ${stats.focus.sector}<//>`}
                sub=${[focusTurns, `avg ${stats.focus.loss.toFixed(3)}s off your best`].filter(Boolean).join(" · ")}
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
      excludedList=${excludedList}
      model=${model}
      map=${viewMap}
      setMap=${setMap}
      mapError=${mapError}
      focus=${focus}
      onMapVisible=${setMainMapVisible}
    />

    <div className="split-2">
      <section className="card">
        <div className="card-head">
          <div className="card-title">Lap times</div>
          <span className="faint fs-12">Untick out laps or spins to leave them out of the stats</span>
        </div>
        <${LapTable} laps=${laps} stats=${stats} excluded=${excluded} selected=${selected} compare=${refNumber} onPick=${pick} onToggle=${toggle} />
      </section>
      <${LapDetail} laps=${laps} stats=${stats} selected=${selected} compare=${compare} refNumber=${refNumber} />
    </div>

    <div className="coach">
      <section className="card">
        <div className="card-head">
          <div className="card-title">Next time out</div>
          <${SpeakButton} text=${split.suggestions.join("\n")} label="Hear the suggestions" />
        </div>
        <ol className="suggestions">
          ${split.suggestions.map((t, i) => html`<li key=${i}><b>${i + 1}</b><span><${TurnText} text=${t} /></span></li>`)}
        </ol>
      </section>
      <${CoachFeedback} split=${split} model=${model} />
    </div>
    <//>`}

    ${turnBase && (turnBase.turns.length || turnBase.sectors.length)
      ? html`<${MiniMap} map=${viewMap} title=${split.track_label} hidden=${tab === "analysis" && mainMapVisible} />`
      : null}
  <//>`;
}
