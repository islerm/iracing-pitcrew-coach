//! The prompts sent to the coach model.

use crate::analysis::corner::CornerReport;
use crate::analysis::SessionSummary;

/// Measurements available for this run, one per line. Missing data is left out rather than
/// sent as zeros, so the model doesn't coach on values that were never measured.
fn data_lines(summary: &SessionSummary) -> String {
    let mut lines = Vec::new();
    if summary.fastest_lap != 0 {
        lines.push(format!("Fastest lap: {:.3}s (lap {})", summary.fastest_lap_time_s, summary.fastest_lap));
        lines.push(format!("Average lap time: {:.3}s", summary.average_lap_time_s));
    }
    if let (Some(name), Some(time)) = (&summary.slowest_sector_name, summary.slowest_sector_time_s) {
        lines.push(format!(
            "Weakest sector on the fastest lap (most time lost vs your best in that sector): {name} at {time:.3}s"
        ));
    }
    if let Some(speed) = summary.average_speed_kph {
        lines.push(format!("Average speed: {speed:.1} kph"));
    }
    if let Some(weather) = &summary.weather {
        let text = crate::telemetry::weather::describe(weather);
        if !text.is_empty() {
            lines.push(format!("Conditions: {text}"));
        }
        if let Some(temp) = weather.fastest_lap_track_temp_c {
            lines.push(format!("Track temp during the fastest lap: {temp:.1}C"));
        }
    }
    if !summary.corner_notes.is_empty() {
        lines.push("Corners where the fastest lap lost the most time vs the driver's best in that corner:".to_string());
        lines.extend(summary.corner_notes.iter().map(|note| format!("- {note}")));
    }
    lines.join("\n")
}

/// How the coach talks. The feedback is read out by a voice playing the driver's race engineer
/// over team radio, so it has to sound spoken, not written.
pub const RADIO_STYLE: &str = "Talk like a real race engineer on team radio: warm, calm and direct, speaking to the driver as \"you\". \
Use short spoken sentences and contractions, one idea per sentence. Round numbers the way engineers say them: \
time in tenths (\"about three tenths\"), speeds and distances as whole numbers. Plain text only: no markdown, no headings, \
no bullet points or numbered lists, no emojis, no parentheses. Don't open with a greeting like \"Alright\" or \"Let's review the data\" \
and don't sign off; get straight to it.";

pub fn run_prompt(summary: &SessionSummary) -> String {
    let suggestions = summary
        .suggestions
        .iter()
        .map(|s| format!("- {s}"))
        .collect::<Vec<_>>()
        .join("\n");

    if summary.fastest_lap == 0 {
        return format!(
            "You are the race engineer for a driver in an iRacing practice session. The recording ended before any full lap was completed, so there is no valid benchmark lap yet. {RADIO_STYLE}\n\n{}\nKey suggestions:\n{}\n\nIn under 80 words, tell the driver what to do to get a clean full lap in.",
            data_lines(summary),
            suggestions,
        );
    }

    format!(
        "You are the race engineer for a driver in an iRacing practice session, debriefing them on the radio. Use only the data below and don't invent numbers or targets. {RADIO_STYLE}\n\n{}\nKey suggestions:\n{}\n\nIn under 140 words: one or two sentences on how the run went, then the three changes that will find the most time, \
each as a couple of spoken sentences naming the corner or sector and what to do differently. Connect them the way you'd talk (\"First...\", \"Then...\", \"And last...\").",
        data_lines(summary),
        suggestions,
    )
}

/// Prompt for the coach model: the findings first (small local models follow those far better
/// than raw numbers), then the measurements, with which direction is better spelled out.
pub fn corner_prompt(report: &CornerReport) -> String {
    let (s, b) = (&report.sel, &report.base);
    let vs = &report.baseline_label;
    let opt = |v: Option<f64>, unit: &str| v.map(|v| format!("{v:.0}{unit}")).unwrap_or_else(|| "n/a".to_string());
    let row = |name: &str, sv: String, bv: String, hint: &str| format!("- {name}: {sv} on lap {} vs {bv} on {vs}{hint}", report.lap);
    let mut lines = vec![
        row("Entry time (turn-in zone to apex)", format!("{:.3}s", s.entry_time_s), format!("{:.3}s", b.entry_time_s), " (lower is better)"),
        row("Exit time (apex to the next straight)", format!("{:.3}s", s.exit_time_s), format!("{:.3}s", b.exit_time_s), " (lower is better)"),
        row("Speed when braking starts", format!("{:.0} kph", s.entry_speed_kph), format!("{:.0} kph", b.entry_speed_kph), ""),
        row("Brake point, metres before the apex", opt(s.brake_point_m, " m"), opt(b.brake_point_m, " m"), " (smaller = later braking)"),
        row("Minimum speed", format!("{:.0} kph", s.min_speed_kph), format!("{:.0} kph", b.min_speed_kph), " (higher is better)"),
        row("Throttle pickup, metres after the apex", opt(s.throttle_on_m, " m"), opt(b.throttle_on_m, " m"), " (smaller = earlier, better)"),
        row("Full throttle, metres after the apex", opt(s.full_throttle_m, " m"), opt(b.full_throttle_m, " m"), " (smaller = earlier, better)"),
        row("Exit speed", format!("{:.0} kph", s.exit_speed_kph), format!("{:.0} kph", b.exit_speed_kph), " (higher is better)"),
        row("Coasting, no pedals", format!("{:.0} m", s.coast_m), format!("{:.0} m", b.coast_m), " (less is better)"),
        row("Gear at the slowest point", s.apex_gear.to_string(), b.apex_gear.to_string(), ""),
    ];
    for g in &report.gear_options {
        lines.push(format!("- Laps taking the apex in gear {}: {} lap(s), best {:.3}s through the corner", g.gear, g.laps.len(), g.best_time_s));
    }
    if s.abs_pct.is_some() {
        lines.push(row("ABS share of braking", opt(s.abs_pct, "%"), opt(b.abs_pct, "%"), " (less is better)"));
    }
    if s.entry_balance_deg.is_some() {
        let hint = " (positive = understeer, negative = oversteer; closer to zero is more neutral)";
        lines.push(row("Entry balance", opt(s.entry_balance_deg, "°"), opt(b.entry_balance_deg, "°"), hint));
        lines.push(row("Exit balance", opt(s.exit_balance_deg, "°"), opt(b.exit_balance_deg, "°"), hint));
    }
    for (name, r) in [("entry", &report.entry_rank), ("exit", &report.exit_rank)] {
        if let Some(r) = r {
            lines.push(format!("- Lap {}'s {name} ranks {} of {} laps (quickest: lap {}).", report.lap, r.rank, r.of, r.best_lap));
        }
    }
    let list = |items: &[String]| if items.is_empty() { "nothing notable".to_string() } else { items.join("; ") };
    let terrain = if report.terrain_notes.is_empty() {
        String::new()
    } else {
        format!("The track here (use this to explain why, e.g. where the car is light or loaded):\n{}\n\n", report.terrain_notes.iter().map(|n| format!("- {n}")).collect::<Vec<_>>().join("\n"))
    };

    format!(
        "You are a race engineer coaching an iRacing driver through Turn {turn}. You are comparing lap {lap} with {vs}. \
Use only the facts below and don't invent numbers or targets.\n\n\
Findings:\n- Entry, went well: {ew}\n- Entry, to work on: {eb}\n- Exit, went well: {xw}\n- Exit, to work on: {xb}\n\n\
{terrain}\
Measurements:\n{data}\n\n\
Reply in under 120 words as two short paragraphs starting \"Entry:\" and \"Exit:\". In each, name one thing to keep doing and one concrete change \
for the next lap, based on the findings. Call the comparison \"{vs}\", never \"last lap\". {style}",
        turn = report.turn,
        lap = report.lap,
        ew = list(&report.entry.went_well),
        eb = list(&report.entry.to_work_on),
        xw = list(&report.exit.went_well),
        xb = list(&report.exit.to_work_on),
        data = lines.join("\n"),
        style = RADIO_STYLE,
    )
}
