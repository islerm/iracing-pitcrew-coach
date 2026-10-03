import { TurnContext, TurnText, gearColor, turnIndex } from "./trackmap.js";
import { Segmented, api, deltaClass, fmtDelta, fmtNum, html, isNum, post, range, useContext, useEffect, useState, useStored } from "./lib.js";
import { SpeakButton } from "./voice.js";
import { VERDICT_LABEL } from "./charts.js";

// ---------- Corner table ----------

export const CornerTable = React.memo(function CornerTable({ corners, hasRef, activeTurn, onSelectTurn, editing, labels, setLabels }) {
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
                    ${fmtDelta(speedDiff, 0)}
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
                  ${isNum(c.balSel) ? html`${fmtDelta(c.balSel, 0)}°${isNum(c.balRef) ? html`<span className="bal-ref"> (${fmtDelta(c.balSel - c.balRef, 0)})</span>` : null}` : "—"}
                </td>`
              : null}
            ${hasAbs ? html`<td className=${c.absSel >= 40 ? "bad" : "faint"}>${isNum(c.absSel) ? `${c.absSel.toFixed(0)}%` : "—"}</td>` : null}
          </tr>`;
        })}
      </tbody>
    </table>
  </div>`;
});

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
export function CornerCoach({ split, turnIndex, turnLabel, selected, refNumber, excludedList, model, offHere, onClose }) {
  const hasRef = refNumber !== null && refNumber !== selected;
  const [mode, setMode] = useStored("pcc.cornerMode", "ref");
  const vsRef = mode === "ref" && hasRef;
  const [report, setReport] = useState(null);
  const [error, setError] = useState(null);
  const [coach, setCoach] = useState({ loading: false, text: null, error: null });
  const url = `/api/splits/${split.id}/corners/${turnIndex}`;
  const body = (withCoach) => ({
    lap: selected,
    ref_lap: vsRef ? refNumber : null,
    exclude: excludedList,
    coach: withCoach,
    model: model || null,
  });

  useEffect(() => {
    let cancelled = false;
    setError(null);
    setCoach({ loading: false, text: null, error: null });
    post(url, body(false))
      .then((r) => !cancelled && setReport(r.report))
      .catch((err) => {
        if (cancelled) return;
        setReport(null);
        setError(err.message);
      });
    return () => {
      cancelled = true;
    };
  }, [split.id, turnIndex, selected, refNumber, vsRef, excludedList]);

  async function askCoach() {
    setCoach({ loading: true, text: null, error: null });
    try {
      const r = await post(url, body(true));
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
          onChange=${setMode}
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
                      <td className=${cls}>${d === null ? "—" : fmtDelta(d, digits) + unit}</td>
                    </tr>`;
                  })}
                </tbody>
              </table>
            </div>
            </div>
            <div className="coach-ask">
              ${coach.text
                ? html`<div className="coach-text">
                    <div className="section-label">Coach <span className="tag">${coach.model}</span> <${SpeakButton} text=${coach.text} label="Hear this corner advice" /></div>
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
export const ShiftsCard = React.memo(function ShiftsCard({ shifts, selected, onJump }) {
  const { turns, setHover } = useContext(TurnContext);
  const hoverPct = (pct) => setHover(pct === null ? null : { pct });
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
        <div className="faint fs-12 mt-2">
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
        ${limiter ? html`<p className=${"fs-12 mt-8 " + (limiter.metres >= 20 ? "bad" : "faint")}>${limiter.metres.toFixed(0)} m on the rev limiter this lap.</p>` : null}
        <p className="faint fs-12 mt-8">Which gear is quicker through a corner: click the turn on the map. When your laps used different gears there, the breakdown compares them.</p>
      </div>
    </div>
  </section>`;
});
