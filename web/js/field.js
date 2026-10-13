// The "Field" tab: how the other drivers in the session did, and where the quickest cars in
// your class gain time on you.

import { Segmented, api, deltaClass, fmtDelta, fmtLap, fmtNum, html, isNum, useEffect, useMemo, useState } from "./lib.js";
import { TurnText } from "./trackmap.js";

// A split never changes once saved, so its field data is fetched once.
const cache = new Map();

function useField(splitId) {
  const [state, setState] = useState(() => ({ id: splitId, data: cache.get(splitId) || null, error: null }));
  useEffect(() => {
    if (cache.has(splitId)) {
      setState({ id: splitId, data: cache.get(splitId), error: null });
      return;
    }
    let cancelled = false;
    setState({ id: splitId, data: null, error: null });
    api(`/api/splits/${splitId}/field`)
      .then((data) => {
        cache.set(splitId, data);
        if (!cancelled) setState({ id: splitId, data, error: null });
      })
      .catch((err) => !cancelled && setState({ id: splitId, data: null, error: err.message || String(err) }));
    return () => {
      cancelled = true;
    };
  }, [splitId]);
  return state.id === splitId ? state : { id: splitId, data: null, error: null };
}

function Leaderboard({ rows, className }) {
  // Several classes are only listed when the session info doesn't say which car is yours:
  // then each class gets its own positions and gaps (the server sends them grouped).
  const multi = new Set(rows.map((r) => r.class_name)).size > 1;
  const positions = [];
  rows.forEach((r, i) => positions.push(i > 0 && rows[i - 1].class_name === r.class_name ? positions[i - 1] + 1 : 1));
  return html`<section className="card">
    <div className="card-head">
      <div className="card-title">Leaderboard${className && !multi ? ` · ${className}` : multi ? " · all classes" : ""}</div>
      <span className="faint fs-12">Best lap in the session${rows.some((r) => r.best_lap_s == null) ? ", where known" : ""}</span>
    </div>
    ${rows.length === 0
      ? html`<p className="muted">The session info doesn't list any drivers for this run.</p>`
      : html`<div className="field-scroll">
          <table className="corners compact field-table">
            <thead>
              <tr>
                <th className="l">#</th>
                <th className="l">Driver</th>
                ${multi ? html`<th className="l">Class</th>` : null}
                <th>iRating</th>
                <th>Licence</th>
                <th>Best lap</th>
                <th title="Behind the best lap in the class">Gap</th>
              </tr>
            </thead>
            <tbody>
              ${rows.map(
                (r, i) => html`<tr key=${r.car_idx} className=${r.is_player ? "me" : ""} title=${r.car}>
                  <td className="l faint">${positions[i]}</td>
                  <td className="l">
                    ${r.is_player ? html`<b>${r.name}</b> <span className="tag tag-best">You</span>` : r.name}
                  </td>
                  ${multi ? html`<td className="l faint">${r.class_name || "—"}</td>` : null}
                  <td>${r.irating > 0 ? r.irating : "—"}</td>
                  <td>${r.license || "—"}</td>
                  <td>${fmtLap(r.best_lap_s)}</td>
                  <td>${isNum(r.gap_s) && r.gap_s > 0 ? fmtDelta(r.gap_s) : "—"}</td>
                </tr>`
              )}
            </tbody>
          </table>
        </div>`}
  </section>`;
}

const pair = (a, b, digits = 0) => `${fmtNum(a, digits)} / ${fmtNum(b, digits)}`;

function Comparison({ comparison, source }) {
  const [sort, setSort] = useState("gap");
  const corners = useMemo(
    () => (sort === "gap" ? [...comparison.corners].sort((a, b) => b.gap_s - a.gap_s) : comparison.corners),
    [comparison, sort]
  );
  const refs = comparison.references;
  return html`<section className="card">
    <div className="card-head">
      <div className="card-title">Against the quickest in your class</div>
      ${source === "library" ? html`<span className="tag tag-out" title="Nobody quick enough recorded a clean lap in this session">from earlier sessions</span>` : null}
    </div>
    <p className="field-against">
      Your best (lap ${comparison.your_lap}, <span className="mono">${fmtLap(comparison.your_time_s)}</span>) against:
      ${refs.map(
        (r, i) => html`<span key=${r.car_idx + ":" + i}>${i ? ", " : " "}<b>${r.name || `Car ${r.car_idx}`}</b>
          <span className="mono muted">${fmtLap(r.lap_time_s)}</span></span>`
      )}
    </p>
    ${comparison.notes.length
      ? html`<ul className="field-notes">
          ${comparison.notes.map((n, i) => html`<li key=${i}><${TurnText} text=${n} /></li>`)}
        </ul>`
      : html`<p className="muted">No corner stands out: you're within a few hundredths of them everywhere.</p>`}
    <div className="card-head field-sort">
      <div className="card-title">Corner by corner</div>
      <${Segmented}
        label="Sort corners"
        value=${sort}
        onChange=${setSort}
        options=${[
          { value: "gap", label: "Biggest gap" },
          { value: "track", label: "Track order" },
        ]}
      />
    </div>
    <div className="field-scroll">
      <table className="corners compact field-table">
        <thead>
          <tr>
            <th className="l">Turn</th>
            <th title="Time you lose (+) or gain (−) through the corner">Gap</th>
            <th title="Minimum speed in the corner, you / them">Apex kph</th>
            <th title="Metres before the apex where you start slowing, you / them">Slowing m</th>
            <th title="Speed at the end of the corner, you / them">Exit kph</th>
            <th title="Gear at the apex, you / them">Gear</th>
            <th className="l">Why</th>
          </tr>
        </thead>
        <tbody>
          ${corners.map(
            (c) => html`<tr key=${c.turn}>
              <td className="l"><${TurnText} text=${"Turn " + c.turn} /></td>
              <td className=${deltaClass(c.gap_s)}>${fmtDelta(c.gap_s)}</td>
              <td>${pair(c.you.apex_speed_kph, c.them.apex_speed_kph)}</td>
              <td>${pair(c.you.slowing_at_m, c.them.slowing_at_m)}</td>
              <td>${pair(c.you.exit_speed_kph, c.them.exit_speed_kph)}</td>
              <td>${c.you.apex_gear > 0 || c.them.apex_gear > 0 ? `${c.you.apex_gear || "—"} / ${c.them.apex_gear || "—"}` : "—"}</td>
              <td className="l why">${c.why ? `They ${c.why}` : ""}</td>
            </tr>`
          )}
        </tbody>
      </table>
    </div>
  </section>`;
}

export function FieldView({ split }) {
  const { data, error } = useField(split.id);
  if (error) return html`<section className="card"><p className="field-error">Could not load the field: ${error}</p></section>`;
  if (!data) return html`<section className="card"><p className="muted">Loading the field…</p></section>`;
  return html`<div className="field-view">
    ${data.source_note ? html`<p className="muted field-note">${data.source_note}</p>` : null}
    <${Leaderboard} rows=${data.leaderboard} className=${data.class_name} />
    ${data.comparison ? html`<${Comparison} comparison=${data.comparison} source=${data.reference_source} />` : null}
  </div>`;
}
