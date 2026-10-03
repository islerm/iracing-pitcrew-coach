import { html, isNum, useContext, useMemo, useRef, useState, useStored } from "./lib.js";

// ---------- Track analysis helpers ----------

export const idxOf = (pct, n) => Math.max(0, Math.min(n, Math.round(pct * n)));

/** Each turn owns the stretch of track from halfway after the previous turn to halfway to the next. */
function turnSegments(turns) {
  return turns.map((turn, i) => ({
    ...turn,
    start: i === 0 ? 0 : (turns[i - 1].pct + turn.pct) / 2,
    end: i === turns.length - 1 ? 1 : (turn.pct + turns[i + 1].pct) / 2,
  }));
}

/**
 * Sector spans as lap fractions, from the track's sector start points. Matches how the server
 * splits lap times: boundaries strictly inside the lap, then the finish line.
 */
function sectorSpans(sectorPcts) {
  const bounds = (sectorPcts || []).filter((p) => p > 0 && p < 1);
  if (!bounds.length) return [];
  const edges = [0, ...bounds];
  return [...bounds, 1].map((end, i) => ({ n: i + 1, start: edges[i], end }));
}

/** The turns a sector covers, e.g. "T4–T7", or null when it has none. */
function sectorTurnsLabel(turns, span) {
  const inside = turns.filter((t) => t.pct >= span.start && t.pct < span.end);
  if (!inside.length) return null;
  const a = inside[0].label;
  const b = inside[inside.length - 1].label;
  return inside.length === 1 ? `T${a}` : `T${a}–T${b}`;
}

/** The per-map part of TurnContext: turns, what each turn owns, and the sectors (with the turns they cover). */
export function mapIndex(map) {
  return {
    turns: map.turns,
    segments: turnSegments(map.turns),
    sectors: sectorSpans(map.sector_pcts).map((span) => ({ ...span, turns: sectorTurnsLabel(map.turns, span) })),
  };
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

export const gearColor = (g) => GEAR_COLORS[Math.max(0, Math.min(GEAR_COLORS.length - 1, g || 0))];

// ---------- Track map ----------

/** Where the comparison lap was when the analysed lap reached `pct`, as a lap fraction. */
export function ghostPct(sel, ref, pct) {
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

/** SVG path along the track points a..b (inclusive). */
function pathRange(points, a, b) {
  let d = "";
  for (let j = a; j <= b; j++) d += `${j === a ? "M" : "L"}${points[j][0].toFixed(1)},${(-points[j][1]).toFixed(1)}`;
  return d;
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
    path: pathRange(points, 0, points.length - 1) + "Z",
  };
}

/** Path along the track between two lap fractions. */
function stretchPath(points, start, end) {
  const n = points.length - 1;
  return pathRange(points, idxOf(start, n), idxOf(end, n));
}

/** The track point at `pct` and the unit normal pointing to the outside of the bend there. */
function outsideNormal(points, pct) {
  const n = points.length - 1;
  const i = idxOf(pct, n);
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
  return { p, nx: (side * ty) / len, ny: (side * -tx) / len };
}

/** Turn badge positions, just outside each corner. */
function turnBadges(map, s) {
  return map.turns.map((turn, ti) => {
    const { p, nx, ny } = outsideNormal(map.points, turn.pct);
    return { ...turn, i: ti, x: p[0] + nx * 20 * s, y: -(p[1] + ny * 20 * s) };
  });
}

/**
 * A tick across the track where each sector starts, labelled on the inside of the bend so it
 * stays clear of the turn badges. The first sector starts at the start line, which is already drawn.
 */
function sectorMarks(spans, points, s) {
  return spans.map((span) => {
    const { p, nx, ny } = outsideNormal(points, span.start);
    const tick = span.start > 0 ? { x1: p[0] + nx * 10 * s, y1: -(p[1] + ny * 10 * s), x2: p[0] - nx * 10 * s, y2: -(p[1] - ny * 10 * s) } : null;
    return { ...span, tick, x: p[0] - nx * 22 * s, y: -(p[1] - ny * 22 * s) };
  });
}

function TurnBadge({ b, s, active, hovered, onClick }) {
  return html`<g
    className=${"turn-badge" + (active ? " active" : "") + (hovered ? " hovered" : "")}
    transform=${`translate(${b.x} ${b.y})`}
    onClick=${() => onClick(b.i)}
  >
    <title>Turn ${b.label}: click for an entry and exit breakdown</title>
    <circle r=${9 * s} />
    <text fontSize=${(b.label.length > 2 ? 7.5 : 9.5) * s} dy=${3.3 * s}>${b.label}</text>
  </g>`;
}

/** Sector start ticks and turn badges, highlighted for the hovered sector / turn. */
const MapMarks = React.memo(function MapMarks({ badges, marks, s, activeTurn, hover, onTurnClick }) {
  const hoveredSector = hover ? hover.sector : null;
  return html`<g>
    ${marks.map(
      (m) => html`<g key=${"sec" + m.n} className=${"sector-mark" + (hoveredSector === m.n - 1 ? " hovered" : "")}>
        <title>Sector ${m.n} starts here</title>
        ${m.tick ? html`<line ...${m.tick} />` : null}
        <text x=${m.x} y=${m.y} fontSize=${11 * s} dy=${3.8 * s} strokeWidth=${3.5 * s}>S${m.n}</text>
      </g>`
    )}
    ${badges.map(
      (b) => html`<${TurnBadge} key=${b.i} b=${b} s=${s} active=${activeTurn === b.i} hovered=${!!hover && hover.turn === b.i} onClick=${onTurnClick} />`
    )}
  </g>`;
});

/** What both maps draw beneath the track line: the outline, the open and hovered turn, the hovered sector. */
function MapBackdrop({ geometry, points, segments, sectors, activeTurn, hover, children }) {
  const activeSeg = isNum(activeTurn) ? segments[activeTurn] : null;
  const hoverSeg = hover && isNum(hover.turn) && hover.turn !== activeTurn ? segments[hover.turn] : null;
  const hoverSpan = hover && isNum(hover.sector) ? sectors[hover.sector] : null;
  return html`<g>
    <path d=${geometry.path} className="map-base" />
    ${activeSeg ? html`<path d=${stretchPath(points, activeSeg.start, activeSeg.end)} className="map-active" />` : null}
    ${hoverSeg ? html`<path d=${stretchPath(points, hoverSeg.start, hoverSeg.end)} className="map-active hover" />` : null}
    ${hoverSpan ? html`<path d=${stretchPath(points, hoverSpan.start, hoverSpan.end)} className="map-sector" />` : null}
    ${children}
  </g>`;
}

const MapDot = ({ p, s, ghost }) => html`<circle cx=${p[0]} cy=${-p[1]} r=${6 * s} className=${"map-cursor" + (ghost ? " ghost" : "")} />`;

export function TrackMapView({ map, sel, refTrace: ref, mode, cursorPct, onHover, activeTurn, onTurnClick, selOff, refOff }) {
  const { segments, sectors } = useContext(TurnContext);
  const hover = useContext(HoverContext);
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
      out.push(html`<path key=${k} d=${pathRange(points, k, e)} stroke=${color} className="map-overlay" />`);
    }
    return out;
  }, [map, sel, ref, mode]);

  const badges = useMemo(() => turnBadges(map, s), [map]);
  const marks = useMemo(() => sectorMarks(sectors, points, s), [map, sectors]);

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

  // The cursor comes from the traces / grip circle, or from a shift chip being hovered.
  const cursorAt = isNum(cursorPct) ? cursorPct : hover && isNum(hover.pct) ? hover.pct : null;
  const cursor = cursorAt !== null ? points[idxOf(cursorAt, n)] : null;
  const ghost = cursor && sel && ref ? points[idxOf(ghostPct(sel, ref, cursorAt), n)] : null;
  const offMarks = (pcts, cls) =>
    (pcts || []).map((pct, i) => {
      const p = points[idxOf(pct, n)];
      return html`<g key=${cls + i} className=${"off-marker " + cls} transform=${`translate(${p[0]} ${-p[1]})`}>
        <title>${cls === "sel" ? "Analysed" : "Comparison"} lap went off track here</title>
        <path d=${`M0,${-9 * s}L${8 * s},${6 * s}H${-8 * s}Z`} />
        <text fontSize=${9 * s} dy=${4 * s}>!</text>
      </g>`;
    });

  return html`<svg ref=${svgRef} className="track-map" viewBox=${geometry.viewBox} onMouseMove=${onMove} onMouseLeave=${() => onHover(null)} role="img" aria-label="Track map">
    <${MapBackdrop} geometry=${geometry} points=${points} segments=${segments} sectors=${sectors} activeTurn=${activeTurn} hover=${hover}>
      ${overlay || html`<path d=${geometry.path} className="map-line" />`}
    <//>
    <line ...${startLine} className="map-start" />
    <${MapMarks} badges=${badges} marks=${marks} s=${s} activeTurn=${activeTurn} hover=${hover} onTurnClick=${onTurnClick} />
    ${offMarks(refOff, "ref")}
    ${offMarks(selOff, "sel")}
    ${ghost ? html`<${MapDot} p=${ghost} s=${s} ghost=${true} />` : null}
    ${cursor ? html`<${MapDot} p=${cursor} s=${s} />` : null}
  </svg>`;
}

// ---------- Turn references ----------

/**
 * Links text that mentions turns or sectors to the track map. Provided by RunView's TurnScope once the
 * map has loaded: `turns`, `segments` (what each turn owns), `sectors` (spans plus the turns they cover;
 * see mapIndex), `setHover`, `focusTurn(i)`, which opens that turn's breakdown in the track section,
 * and `focusSector(i)`, which zooms the traces to that sector. It never changes while hovering.
 */
export const TurnContext = React.createContext(null);

/** What is being pointed at, so the components that highlight it can re-render alone: { turn }, { sector } or { pct }, or null. */
const HoverContext = React.createContext(null);

/** Owns the hover state so that moving the mouse over a chip only re-renders what reads HoverContext. */
export function TurnScope({ base, children }) {
  const [hover, setHover] = useState(null);
  const ctx = useMemo(() => (base ? { ...base, setHover } : null), [base]);
  return html`<${TurnContext.Provider} value=${ctx}><${HoverContext.Provider} value=${hover}>${children}<//><//>`;
}

// "Turn 5", "Turns 5 and 6", "turns 10-11", "T5", "T10a".
const TURN_RE = /\b([Tt]urns?\s+)(\d{1,2}[a-zA-Z]?(?:\s*(?:,|and|&|or|to|-|–)\s*\d{1,2}[a-zA-Z]?)*)\b|\bT(\d{1,2}[a-zA-Z]?)\b/g;

// "Sector 2", "S2".
const SECTOR_RE = /\b[Ss]ector\s+(\d)\b|\bS(\d)\b/g;

export function turnIndex(turns, label) {
  const want = String(label).toLowerCase();
  return turns.findIndex((t) => t.label.toLowerCase() === want);
}

/**
 * A turn or sector name that points at the map: hovering highlights it, clicking opens the turn's
 * breakdown (kind "turn") or zooms the traces to the sector (kind "sector"). Plain text when there's
 * no map to point at.
 */
export function MapRef({ kind, index, children }) {
  const ctx = useContext(TurnContext);
  const hover = useContext(HoverContext);
  const item = ctx && (kind === "turn" ? ctx.turns : ctx.sectors)[index];
  if (!item) return children;
  const what = kind === "turn" ? `Turn ${item.label}` : `Sector ${item.n}${item.turns ? ` (${item.turns})` : ""}`;
  const point = (on) => ctx.setHover(on ? { [kind]: index } : null);
  return html`<button
    type="button"
    className=${kind + "-ref" + (hover && hover[kind] === index ? " hovered" : "")}
    title=${`${what}: hover to see it on the map, click to ${kind === "turn" ? "open its breakdown" : "zoom the traces to it"}`}
    onMouseEnter=${() => point(true)}
    onMouseLeave=${() => point(false)}
    onFocus=${() => point(true)}
    onBlur=${() => point(false)}
    onClick=${(e) => {
      e.stopPropagation();
      point(false);
      (kind === "turn" ? ctx.focusTurn : ctx.focusSector)(index);
    }}
  >${children}</button>`;
}

/** Turn mentions in `text` as chips that point at the turn on the map. */
function turnPieces(text, ctx, keys) {
  if (!ctx.turns.length) return [text];
  const out = [];
  let last = 0;
  for (const m of text.matchAll(TURN_RE)) {
    if (m.index > last) out.push(text.slice(last, m.index));
    last = m.index + m[0].length;
    if (m[3]) {
      const i = turnIndex(ctx.turns, m[3]);
      out.push(i >= 0 ? html`<${MapRef} kind="turn" key=${keys.k++} index=${i}>${m[0]}<//>` : m[0]);
      continue;
    }
    const nums = m[2].split(/(\d{1,2}[a-zA-Z]?)/);
    const single = nums.filter((p, j) => j % 2 === 1).length === 1;
    const i0 = single ? turnIndex(ctx.turns, nums[1]) : -1;
    if (single && i0 >= 0) {
      out.push(html`<${MapRef} kind="turn" key=${keys.k++} index=${i0}>${m[0]}<//>`);
      continue;
    }
    out.push(m[1]);
    nums.forEach((part, j) => {
      const i = j % 2 === 1 ? turnIndex(ctx.turns, part) : -1;
      out.push(i >= 0 ? html`<${MapRef} kind="turn" key=${keys.k++} index=${i}>${part}<//>` : part);
    });
  }
  if (last < text.length) out.push(text.slice(last));
  return out;
}

/** Text with every turn and sector it mentions turned into a chip that points at it on the map. */
export function TurnText({ text }) {
  const ctx = useContext(TurnContext);
  if (!text || !ctx) return text || null;
  const out = [];
  const keys = { k: 0 };
  let last = 0;
  for (const m of text.matchAll(SECTOR_RE)) {
    const i = Number(m[1] || m[2]) - 1;
    if (!ctx.sectors[i]) continue;
    if (m.index > last) out.push(...turnPieces(text.slice(last, m.index), ctx, keys));
    out.push(html`<${MapRef} kind="sector" key=${keys.k++} index=${i}>${m[0]}<//>`);
    last = m.index + m[0].length;
  }
  if (last < text.length) out.push(...turnPieces(text.slice(last), ctx, keys));
  return out;
}

/**
 * A small track map pinned to the corner of the window while the main map is scrolled out of
 * view, so turn numbers in tables and coach notes always have a map next to them.
 */
export function MiniMap({ map, title, hidden }) {
  const ctx = useContext(TurnContext);
  const hover = useContext(HoverContext);
  const [pref, setPref] = useStored("pcc.miniMap", "open");
  const open = pref === "open";
  const geometry = useMemo(() => mapGeometry(map.points, 220), [map]);
  const badges = useMemo(() => turnBadges(map, geometry.s), [map, geometry]);
  const marks = useMemo(() => sectorMarks(ctx.sectors, map.points, geometry.s), [ctx, geometry]);
  // Pointing at a turn while the map is collapsed peeks it open.
  const show = open || !!hover;
  if (hidden) return null;

  const toggle = () => setPref(open ? "closed" : "open");
  if (!show) {
    return html`<button className="mini-map-toggle" onClick=${toggle} title="Show the track map">
      <svg viewBox=${geometry.viewBox} aria-hidden="true"><path d=${geometry.path} /></svg>
      Map
    </button>`;
  }

  const { s } = geometry;
  const points = map.points;
  const dot = hover && isNum(hover.pct) ? points[idxOf(hover.pct, points.length - 1)] : null;
  return html`<aside className=${"mini-map" + (open ? "" : " peek")} aria-label="Track map">
    <div className="mini-map-head">
      <span className="mini-map-title">${title || "Track"}</span>
      <button className="btn btn-ghost btn-icon" onClick=${toggle} title=${open ? "Minimise the map" : "Keep the map open"} aria-label=${open ? "Minimise map" : "Keep map open"}>
        ${open ? "–" : "📌"}
      </button>
    </div>
    <svg className="track-map" viewBox=${geometry.viewBox} role="img" aria-label="Track map">
      <${MapBackdrop} geometry=${geometry} points=${points} segments=${ctx.segments} sectors=${ctx.sectors} hover=${hover}>
        <path d=${geometry.path} className="map-line" />
      <//>
      <${MapMarks} badges=${badges} marks=${marks} s=${s} hover=${hover} onTurnClick=${ctx.focusTurn} />
      ${dot ? html`<${MapDot} p=${dot} s=${s} />` : null}
    </svg>
    <div className="mini-map-hint faint">${badges.length ? "Click a turn to open its breakdown" : "Sector starts are marked S1, S2…"}</div>
  </aside>`;
}
