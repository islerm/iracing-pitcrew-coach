// iRacing's official track map (the one on the members site): its turn numbers and its
// orientation, fitted onto our own outline so they line up with the lap-distance grid.

const BASE = "https://members-assets.iracing.com/public/track-maps";

/** Folder of the official map layers, e.g. tracks_midohio/588-midohio-2026-full/, or null. */
export function officialMapUrl(map) {
  if (!map || !map.track_id || !map.track_name) return null;
  const name = map.track_name.trim().toLowerCase().replace(/\s+/g, "-");
  return `${BASE}/tracks_${name.split("-")[0]}/${map.track_id}-${name}/`;
}

async function fetchSvg(url) {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`${res.status} for ${url}`);
  const doc = new DOMParser().parseFromString(await res.text(), "image/svg+xml");
  const svg = doc.documentElement;
  if (!svg || svg.nodeName !== "svg") throw new Error(`Not an SVG: ${url}`);
  return svg;
}

/**
 * Mounts an SVG off screen so the browser can measure it (path lengths, text boxes, transforms),
 * at 1:1 with its viewBox so getCTM() gives viewBox coordinates.
 */
function mount(svg) {
  const node = document.importNode(svg, true);
  const vb = (node.getAttribute("viewBox") || "0 0 1920 1080").split(/[\s,]+/).map(Number);
  node.removeAttribute("viewBox");
  node.setAttribute("width", vb[2]);
  node.setAttribute("height", vb[3]);
  node.setAttribute("style", "position:fixed;left:-99999px;top:0;visibility:hidden;pointer-events:none");
  node.setAttribute("transform", `translate(${-vb[0]} ${-vb[1]})`);
  document.body.appendChild(node);
  return node;
}

function apply(m, x, y) {
  return [m.a * x + m.c * y + m.e, m.b * x + m.d * y + m.f];
}

/** Points every ~`step` px along every shape in the layer, in viewBox coordinates. */
function samplePoints(node, step = 3) {
  const out = [];
  node.querySelectorAll("path, polyline, polygon, line, ellipse, circle, rect").forEach((el) => {
    if (typeof el.getTotalLength !== "function") return;
    const m = el.getCTM();
    const len = el.getTotalLength();
    if (!m || !(len > 0)) return;
    for (let d = 0; d <= len; d += step) {
      const p = el.getPointAtLength(d);
      out.push(apply(m, p.x, p.y));
    }
  });
  return out;
}

/** Turn-number labels ("1", "10a") and the centre of each, in viewBox coordinates. Names like "The Carousel" are skipped. */
function turnLabels(node) {
  const out = [];
  node.querySelectorAll("text").forEach((el) => {
    const label = (el.textContent || "").trim();
    if (!/^\d{1,2}[a-z]?$/i.test(label)) return;
    const m = el.getCTM();
    const box = el.getBBox();
    if (!m) return;
    out.push({ label, at: apply(m, box.x + box.width / 2, box.y + box.height / 2) });
  });
  return out;
}

// ---------- Fitting ----------

function centroid(pts) {
  let x = 0;
  let y = 0;
  for (const p of pts) {
    x += p[0];
    y += p[1];
  }
  return [x / pts.length, y / pts.length];
}

function rmsRadius(pts, c) {
  let s = 0;
  for (const p of pts) s += (p[0] - c[0]) ** 2 + (p[1] - c[1]) ** 2;
  return Math.sqrt(s / pts.length);
}

function thin(pts, max) {
  if (pts.length <= max) return pts;
  const k = pts.length / max;
  return Array.from({ length: max }, (_, i) => pts[Math.floor(i * k)]);
}

/** Similarity transform: p → s·R(θ)·p + t. */
const transform = (T, p) => {
  const c = Math.cos(T.theta) * T.s;
  const s = Math.sin(T.theta) * T.s;
  return [c * p[0] - s * p[1] + T.tx, s * p[0] + c * p[1] + T.ty];
};

function nearest(p, cloud) {
  let best = Infinity;
  let bi = -1;
  for (let i = 0; i < cloud.length; i++) {
    const d = (cloud[i][0] - p[0]) ** 2 + (cloud[i][1] - p[1]) ** 2;
    if (d < best) {
      best = d;
      bi = i;
    }
  }
  return [bi, Math.sqrt(best)];
}

/** Mean distance from each of `a` (transformed) to the nearest of `b`, and back. */
function chamfer(T, a, b) {
  const ta = a.map((p) => transform(T, p));
  let d = 0;
  for (const p of ta) d += nearest(p, b)[1];
  for (const p of b) d += nearest(p, ta)[1];
  return d / (a.length + b.length);
}

/** Best similarity transform taking a[i] onto b[i] (2D Umeyama, no reflection). */
function procrustes(a, b) {
  const ca = centroid(a);
  const cb = centroid(b);
  let sxx = 0;
  let sxy = 0;
  let aa = 0;
  for (let i = 0; i < a.length; i++) {
    const ax = a[i][0] - ca[0];
    const ay = a[i][1] - ca[1];
    const bx = b[i][0] - cb[0];
    const by = b[i][1] - cb[1];
    sxx += ax * bx + ay * by;
    sxy += ax * by - ay * bx;
    aa += ax * ax + ay * ay;
  }
  const theta = Math.atan2(sxy, sxx);
  const s = (sxx * Math.cos(theta) + sxy * Math.sin(theta)) / aa;
  const T = { theta, s, tx: 0, ty: 0 };
  const m = transform(T, ca);
  return { theta, s, tx: cb[0] - m[0], ty: cb[1] - m[1] };
}

/**
 * Fits our outline (`ours`, y down) onto the official one: a coarse search over rotation with
 * centroids and sizes matched, then ICP. Returns the transform and the mean miss as a share
 * of the track's size.
 */
function fit(ours, official) {
  const a = thin(ours, 220);
  const b = thin(official, 450);
  const ca = centroid(a);
  const cb = centroid(b);
  const s = rmsRadius(b, cb) / rmsRadius(a, ca);
  const around = (theta) => {
    const T = { theta, s, tx: 0, ty: 0 };
    const m = transform(T, ca);
    return { theta, s, tx: cb[0] - m[0], ty: cb[1] - m[1] };
  };
  let best = null;
  for (let deg = 0; deg < 360; deg += 3) {
    const T = around((deg * Math.PI) / 180);
    const err = chamfer(T, a, b);
    if (!best || err < best.err) best = { T, err };
  }
  let T = best.T;
  for (let iter = 0; iter < 25; iter++) {
    const pairs = a.map((p) => b[nearest(transform(T, p), b)[0]]);
    T = procrustes(a, pairs);
  }
  return { T, err: chamfer(T, a, b) / rmsRadius(b, cb) };
}

// ---------- Turns ----------

function naturalOrder(x, y) {
  const nx = parseInt(x.label, 10);
  const ny = parseInt(y.label, 10);
  return nx !== ny ? nx - ny : x.label.localeCompare(y.label);
}

/**
 * Lap fraction for each label. A label sits beside its corner, but where the track doubles back
 * the nearest bit of road can belong to another corner, so each label keeps a few candidate
 * spots and the labels are then placed in number order around the lap (one wrap past the line
 * allowed, for tracks where turn 1 comes before the start line).
 */
function placeLabels(labels, placed) {
  const n = placed.length - 1;
  const win = Math.max(2, Math.round(n * 0.01));
  const options = labels.map(({ at }) => {
    const d = placed.map((q) => Math.hypot(q[0] - at[0], q[1] - at[1]));
    const dMin = Math.min(...d);
    const cands = [];
    for (let i = 0; i <= n; i++) {
      if (d[i] > Math.max(1.6 * dMin, dMin + 25)) continue;
      let isMin = true;
      for (let j = Math.max(0, i - win); j <= Math.min(n, i + win); j++) if (d[j] < d[i]) isMin = false;
      if (isMin) cands.push({ pct: i / n, cost: d[i] });
    }
    return cands.length ? cands : [{ pct: d.indexOf(dMin) / n, cost: dMin }];
  });

  // Dynamic programme over (label, candidate, wrapped yet).
  const WRAP_COST = 5;
  let prev = options[0].map((c) => [
    { cost: c.cost, back: null },
    { cost: Infinity, back: null },
  ]);
  const table = [prev];
  for (let k = 1; k < options.length; k++) {
    const row = options[k].map((c) => {
      const cell = [
        { cost: Infinity, back: null },
        { cost: Infinity, back: null },
      ];
      options[k - 1].forEach((p, j) => {
        for (const w of [0, 1]) {
          const from = prev[j][w].cost;
          if (!Number.isFinite(from)) continue;
          if (c.pct > p.pct && from + c.cost < cell[w].cost) cell[w] = { cost: from + c.cost, back: [j, w] };
          if (w === 0 && c.pct < p.pct && from + c.cost + WRAP_COST < cell[1].cost) cell[1] = { cost: from + c.cost + WRAP_COST, back: [j, 0] };
        }
      });
      return cell;
    });
    table.push(row);
    prev = row;
  }
  let end = null;
  prev.forEach((cell, j) =>
    [0, 1].forEach((w) => {
      if (Number.isFinite(cell[w].cost) && (!end || cell[w].cost < end.cost)) end = { cost: cell[w].cost, j, w };
    })
  );
  if (!end) {
    // Nothing fits in order: fall back to each label's nearest spot.
    return labels.map((l, k) => ({ label: l.label, pct: options[k].reduce((a, b) => (b.cost < a.cost ? b : a)).pct }));
  }
  const out = [];
  let j = end.j;
  let w = end.w;
  for (let k = options.length - 1; k >= 0; k--) {
    out.unshift({ label: labels[k].label, pct: options[k][j].pct });
    const back = table[k][j][w].back;
    if (back) [j, w] = back;
  }
  return out;
}

/**
 * Turn numbers from iRacing's official map, placed on our lap-distance grid, plus the rotation
 * (clockwise degrees) that draws our outline the way the official map is drawn. Throws when the
 * official map can't be found or doesn't match the outline.
 */
export async function officialTurns(map) {
  const url = officialMapUrl(map);
  if (!url) throw new Error("No iRacing track id for this track yet.");
  const [active, turnsSvg] = await Promise.all([fetchSvg(url + "active.svg"), fetchSvg(url + "turns.svg")]);
  const activeNode = mount(active);
  const turnsNode = mount(turnsSvg);
  let official;
  let labels;
  try {
    official = samplePoints(activeNode);
    labels = turnLabels(turnsNode).sort(naturalOrder);
  } finally {
    activeNode.remove();
    turnsNode.remove();
  }
  if (official.length < 50 || !labels.length) throw new Error("The official map has no outline or turn numbers.");

  // Our outline is x east / y north; SVG is y down.
  const ours = map.points.map((p) => [p[0], -p[1]]);
  const { T, err } = fit(ours, official);
  if (err > 0.06) throw new Error(`The official map doesn't line up with this outline (miss ${(err * 100).toFixed(1)}%).`);

  const placed = ours.map((p) => transform(T, p));
  const turns = placeLabels(labels, placed).sort((x, y) => x.pct - y.pct);
  const rotation = (((T.theta * 180) / Math.PI) % 360 + 360) % 360;
  return { turns, rotation_deg: Math.round(rotation * 10) / 10, fit_error: err };
}

/** The map with its outline turned `map.rotation_deg` clockwise, for drawing. Lap fractions are unchanged. */
export function rotatedMap(map) {
  const deg = (map && map.rotation_deg) || 0;
  if (!deg) return map;
  const r = (deg * Math.PI) / 180;
  const c = Math.cos(r);
  const s = Math.sin(r);
  // Clockwise on screen, in y-up coordinates.
  return { ...map, points: map.points.map(([x, y]) => [x * c + y * s, -x * s + y * c]) };
}
