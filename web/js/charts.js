import { deltaClass, fmtDelta, fmtLap, html, isNum, mean, offTrackTitle, offTracks, trackTempSpan, useContext, useElementWidth, useMemo, useRef, useState, useStored } from "./lib.js";
import { TurnContext, idxOf } from "./trackmap.js";

// ---------- Pace chart ----------

export function PaceChart({ laps, stats, excluded, selected, compare, onPick }) {
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

export const VERDICT_LABEL = { early: "early", late: "late", on_time: "on time", part_throttle: "part throttle", unknown: "" };

const TRACE_PAD = { l: 12, r: 12, top: 18, gap: 8 };

function fmtReadout(p, v) {
  if (!isNum(v)) return "—";
  if (p.key === "delta") return fmtDelta(v);
  if (p.key === "gear") return v === 0 ? "N" : v < 0 ? "R" : String(v);
  if (p.signed) return fmtDelta(v, 0) + p.unit;
  return v.toFixed(p.digits || 0) + (p.unit === "%" || p.unit === "g" || p.unit === "m" ? p.unit : "");
}

export function TraceChart({ sel, refTrace: ref, lengthM, cursorPct, onCursor, view, setView, shiftRpm, upshifts }) {
  const { turns, sectors } = useContext(TurnContext);
  const [wrapRef, width] = useElementWidth();
  const [drag, setDrag] = useState(null);
  const pad = TRACE_PAD;
  const n = sel.time_s.length - 1;
  const W = Math.max(320, width);
  const plotW = W - pad.l - pad.r;
  const [r0, r1] = view;

  const delta = useMemo(() => (ref ? sel.time_s.map((t, j) => t - ref.time_s[j]) : null), [sel, ref]);
  // Elevation relative to the lowest point of the analysed lap, for both laps.
  const altRel = useMemo(() => {
    if (!sel.alt_m || !sel.alt_m.length) return null;
    const base = Math.min(...sel.alt_m);
    const rel = (t) => (t && t.alt_m && t.alt_m.length ? t.alt_m.map((v) => v - base) : null);
    return { sel: rel(sel), ref: rel(ref) };
  }, [sel, ref]);

  // Everything that depends on the data, the zoom and the width but not on the cursor: the panels'
  // domains and paths are computed once, so moving the mouse only redraws the readouts and cursor line.
  const chart = useMemo(() => {
    const i0 = Math.max(0, Math.floor(r0 * n));
    const i1 = Math.min(n, Math.ceil(r1 * n));
    const step = Math.max(1, Math.floor((i1 - i0) / plotW));
    const x = (j) => pad.l + ((j / n - r0) / (r1 - r0)) * plotW;
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
      return { ...p, top, selArr: channel(sel, p.key), refArr: p.key === "delta" ? null : channel(ref, p.key) };
    });
    const H = y;

    function domain(p) {
      if (p.fixed) return p.fixed;
      const values = [];
      for (const arr of [p.selArr, p.refArr]) {
        if (!arr) continue;
        for (let j = i0; j <= i1; j += step) values.push(arr[j]);
      }
      if (!values.length) return [-1, 1];
      if (p.key === "rpm" && isNum(shiftRpm)) values.push(shiftRpm);
      const lo = Math.min(...values);
      const hi = Math.max(...values);
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

    const body = html`<g>
      <defs>
        ${layout.map(
          (p) => html`<clipPath key=${p.key} id=${"clip-" + p.key}>
            <rect x=${pad.l} y=${p.top} width=${plotW} height=${p.h} />
          </clipPath>`
        )}
      </defs>
      ${sectors
        .filter((sp) => sp.start >= r0 && sp.start < r1)
        .map(
          (sp) => html`<g key=${"s" + sp.n}>
            ${sp.start > 0 ? html`<line className="sector-line" x1=${x(sp.start * n)} x2=${x(sp.start * n)} y1=${pad.top} y2=${H - pad.gap} />` : null}
            <text className="sector-label" x=${x(sp.start * n) + 4} y=${H - pad.gap - 4}>S${sp.n}</text>
          </g>`
        )}
      ${turns
        .filter((t) => t.pct >= r0 && t.pct <= r1)
        .map(
          (t) => html`<g key=${"t" + t.label + t.pct}>
            <line className="turn-line" x1=${x(t.pct * n)} x2=${x(t.pct * n)} y1=${pad.top - 2} y2=${H - pad.gap} />
            <text className="turn-label" x=${x(t.pct * n)} y=${11}>T${t.label}</text>
          </g>`
        )}
      ${layout.map((p) => {
        const dom = domain(p);
        const [lo, hi] = dom;
        const yv = (v) => p.top + p.h - ((v - lo) / (hi - lo)) * p.h;
        const { selArr, refArr } = p;
        const zeroY = yv(0);
        return html`<g key=${p.key}>
          <rect className="panel-bg" x=${pad.l} y=${p.top} width=${plotW} height=${p.h} />
          ${p.key === "delta" || p.symmetric ? html`<line className="zero-line" x1=${pad.l} x2=${pad.l + plotW} y1=${zeroY} y2=${zeroY} />` : null}
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
        </g>`;
      })}
    </g>`;
    return { H, x, layout, body, i0, i1 };
  }, [sel, ref, delta, altRel, view, W, shiftRpm, upshifts, turns, sectors]);

  const { H, x, layout, body, i0, i1 } = chart;
  const cursorIdx = isNum(cursorPct) ? idxOf(cursorPct, n) : null;
  const pctFromEvent = (e) => {
    const rect = e.currentTarget.getBoundingClientRect();
    const px = ((e.clientX - rect.left) / rect.width) * W;
    return Math.max(0, Math.min(1, r0 + ((px - pad.l) / plotW) * (r1 - r0)));
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
      ${body}
      ${cursorIdx !== null
        ? layout.map(
            (p) => html`<text key=${p.key} className="panel-readout" x=${pad.l + plotW - 6} y=${p.top + 13}>
              ${p.key === "delta"
                ? html`<tspan className=${p.selArr ? deltaClass(p.selArr[cursorIdx]) + "-fill" : ""}>${p.selArr ? fmtReadout(p, p.selArr[cursorIdx]) : "—"}</tspan>`
                : html`<tspan className="sel-fill">${fmtReadout(p, p.selArr && p.selArr[cursorIdx])}</tspan>${p.refArr
                    ? html`<tspan className="faint-fill"> / </tspan><tspan className="ref-fill">${fmtReadout(p, p.refArr[cursorIdx])}</tspan>`
                    : null}`}
            </text>`
          )
        : null}
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
export function GripCircle({ sel, refTrace: ref, view, peakG, cursorPct, onCursor, selected, refNumber }) {
  const svgRef = useRef(null);
  const [help, setHelp] = useStored("pcc.gripHelpOpen", "no");
  const helpOpen = help === "yes";
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

  // The plot and the grip numbers only change with the laps and the zoom, not with the cursor.
  const plot = useMemo(() => {
    const dots = (t) => {
      let d = "";
      for (let j = i0; j <= i1; j += step) d += `M${px(t.lat_g[j]).toFixed(1)},${py(t.long_g[j]).toFixed(1)}h0`;
      return d;
    };
    const rings = [];
    for (let g = 1; g <= extent; g += 1) rings.push(g);
    // Quadrant labels sit on the diagonals, inside the plot.
    const d45 = (c - 18) * 0.78 * Math.SQRT1_2;
    const layer = html`<g>
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
    </g>`;
    return { layer, statsSel: gripStats(sel, i0, i1, peakG), statsRef: gripStats(ref, i0, i1, peakG) };
  }, [sel, ref, extent, peakG, i0, i1, step]);
  const { layer, statsSel, statsRef } = plot;

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
  const zoomed = view[0] > 0 || view[1] < 1;

  return html`<div className="grip-circle">
    <div className="section-label">
      <span>Grip circle · ${zoomed ? "zoomed section" : "whole lap"}</span>
      <button
        className=${"btn btn-sm help-btn" + (helpOpen ? " on" : "")}
        aria-expanded=${helpOpen}
        onClick=${() => setHelp(helpOpen ? "no" : "yes")}
      >${helpOpen ? html`<span aria-hidden="true">✕</span> Hide help` : html`<span className="help-icon" aria-hidden="true">?</span> How to read this`}</button>
    </div>
    ${helpOpen
      ? html`<div className="grip-help">
          <p>Every dot is one moment of the lap, placed by the force on the car: <b>left/right</b> is cornering, <b>up</b> is accelerating, <b>down</b> is braking.</p>
          <p>The <b>dashed ring</b> is the most grip you used all session, roughly what the tyres can give. The further the dots reach toward it, the more grip you're using.</p>
          <p>A <b>"+" shape</b> means you brake, then turn, then accelerate as separate steps. Dots filling the <b>diagonals</b> mean you blend them: trail braking into the corner (lower corners) and feeding in throttle while still turning (upper corners). That's usually quicker.</p>
          <p>Compare with the <span className="ref-text">orange</span> lap: where it reaches further out, that lap used more grip. Click a turn to see just that corner.</p>
        </div>`
      : null}
    <svg ref=${svgRef} viewBox=${`0 0 ${size} ${size}`} role="img" aria-label="Lateral versus longitudinal g" onMouseMove=${onMove} onMouseLeave=${() => onCursor(null)}>
      ${layer}
      ${curRef ? html`<circle className="gg-cursor ref" cx=${px(curRef.lat)} cy=${py(curRef.long)} r="4.5" />` : null}
      ${cur ? html`<circle className="gg-cursor sel" cx=${px(cur.lat)} cy=${py(cur.long)} r="4.5" />` : null}
    </svg>
    <div className="gg-readout mono">
      ${cur
        ? html`<div><span className="sel-fill">Lap ${selected}</span> ${fmtDelta(cur.lat, 2)} lat ${fmtDelta(cur.long, 2)} long${pctOfPeak(cur)}</div>
            ${curRef ? html`<div><span className="ref-fill">Lap ${refNumber}</span> ${fmtDelta(curRef.lat, 2)} lat ${fmtDelta(curRef.long, 2)} long${pctOfPeak(curRef)}</div>` : null}`
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
