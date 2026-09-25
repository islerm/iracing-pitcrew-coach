use crate::types::{LapMetrics, SessionSummary};

pub fn average(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        None
    } else {
        Some(values.iter().sum::<f64>() / values.len() as f64)
    }
}

pub fn summarize_session(laps: &[LapMetrics]) -> SessionSummary {
    let complete_laps: Vec<&LapMetrics> = laps.iter().filter(|lap| lap.is_complete).collect();
    let summary_laps: Vec<&LapMetrics> = if complete_laps.is_empty() {
        Vec::new()
    } else {
        complete_laps
    };

    if summary_laps.is_empty() {
        return SessionSummary {
            fastest_lap: 0,
            fastest_lap_time_s: 0.0,
            average_lap_time_s: 0.0,
            slowest_sector_name: None,
            slowest_sector_time_s: None,
            average_speed_kph: None,
            tyre_temp_avg_c: None,
            tyre_temp_delta_c: None,
            suggestions: vec![
                "No complete laps were recorded in this split, so there is no valid fastest lap yet.".to_string(),
                "Keep recording until you cross the finish line again so the app can compare full laps.".to_string(),
            ],
            corner_notes: Vec::new(),
        };
    }

    let fastest = summary_laps
        .iter()
        .min_by(|a, b| a.lap_time_s.partial_cmp(&b.lap_time_s).unwrap())
        .unwrap();

    let lap_times: Vec<f64> = summary_laps.iter().map(|lap| lap.lap_time_s).collect();
    let average_lap_time = average(&lap_times).unwrap_or_default();

    // Sectors differ in length, so rank them by how much the fastest lap lost against the
    // best time you've set in that same sector, not by raw sector time.
    let mut sector_losses: Vec<(String, f64, f64)> = Vec::new();
    for (index, &value) in fastest.sectors.iter().enumerate() {
        let best = summary_laps
            .iter()
            .filter_map(|lap| lap.sectors.get(index).copied())
            .fold(f64::INFINITY, f64::min);
        sector_losses.push((format!("Sector {}", index + 1), value, value - best));
    }

    let (slowest_sector_name, slowest_sector_time) = sector_losses
        .into_iter()
        .max_by(|a, b| a.2.partial_cmp(&b.2).unwrap())
        .map(|(name, value, _)| (Some(name), Some(value)))
        .unwrap_or((None, None));

    let avg_speed_values: Vec<f64> = summary_laps.iter().filter_map(|lap| lap.avg_speed_kph).collect();
    let avg_temp_values: Vec<f64> = summary_laps.iter().filter_map(|lap| lap.tyre_temp_avg_c).collect();
    let temp_delta_values: Vec<f64> = summary_laps.iter().filter_map(|lap| lap.tyre_temp_delta_c).collect();

    let avg_speed = average(&avg_speed_values);
    let avg_temp = average(&avg_temp_values);
    let temp_delta = temp_delta_values.iter().fold(0.0_f64, |acc, x| acc.max(*x));

    let mut suggestions = Vec::new();
    if let Some(name) = &slowest_sector_name {
        suggestions.push(format!(
            "Focus on {} to recover time: on your fastest lap it was furthest off your best time for that sector.",
            name.to_ascii_lowercase()
        ));
    }
    if let Some(temp) = avg_temp {
        if temp > 95.0 {
            suggestions.push("Your tyre temperature is running hot; look for cleaner exits and less heat build-up in the middle of the lap.".to_string());
        }
    }
    if temp_delta > 8.0 {
        suggestions.push("Tyre temperature spread is high. Adjust your balance and reduce wheel spin on exit to improve consistency.".to_string());
    }
    if summary_laps.iter().any(|lap| lap.lap_time_s - fastest.lap_time_s > 0.3) {
        suggestions.push("The gap to your best lap is coming from a few unstable entries or exits. Smooth throttle and brake inputs to reduce variability.".to_string());
    }
    if laps.iter().any(|lap| !lap.is_complete) {
        suggestions.push("Incomplete laps are excluded from the best-lap calculation, so only full finish-line-to-finish-line laps affect the benchmark.".to_string());
    }
    if suggestions.is_empty() {
        suggestions.push("You are close to your best pace; keep the same rhythm and look for one clean, consistent lap to lock in the lap time.".to_string());
    }

    SessionSummary {
        fastest_lap: fastest.lap_number,
        fastest_lap_time_s: fastest.lap_time_s,
        average_lap_time_s: average_lap_time,
        slowest_sector_name,
        slowest_sector_time_s: slowest_sector_time,
        average_speed_kph: avg_speed,
        tyre_temp_avg_c: avg_temp,
        tyre_temp_delta_c: if temp_delta > 0.0 { Some(temp_delta) } else { None },
        suggestions,
        corner_notes: Vec::new(),
    }
}

/// Adds corner findings to a summary: kept for the coach prompt, and the top ones lead the
/// suggestions list since they're the most specific advice available.
pub fn add_corner_notes(summary: &mut SessionSummary, notes: Vec<String>) {
    let at = usize::from(summary.slowest_sector_name.is_some()).min(summary.suggestions.len());
    for (i, note) in notes.iter().enumerate() {
        summary.suggestions.insert(at + i, note.clone());
    }
    summary.corner_notes = notes;
}

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
    if let Some(temp) = summary.tyre_temp_avg_c {
        lines.push(format!("Average tyre temp: {temp:.1}C"));
    }
    if let Some(spread) = summary.tyre_temp_delta_c {
        lines.push(format!("Tyre temp spread: {spread:.1}C"));
    }
    if !summary.corner_notes.is_empty() {
        lines.push("Corners where the fastest lap lost the most time vs the driver's best in that corner:".to_string());
        lines.extend(summary.corner_notes.iter().map(|note| format!("- {note}")));
    }
    lines.join("\n")
}

pub fn build_prompt(summary: &SessionSummary) -> String {
    let suggestions = summary
        .suggestions
        .iter()
        .map(|s| format!("- {s}"))
        .collect::<Vec<_>>()
        .join("\n");

    if summary.fastest_lap == 0 {
        return format!(
            "You are a race coach for an iRacing practice session. The recording ended before any full lap was completed, so there is no valid benchmark lap yet. Keep the answer under 120 words and be helpful about how to get a clean full lap.\n\n{}\nKey suggestions:\n{}\n\nAnswer with a short coaching summary and next steps for completing a full lap.",
            data_lines(summary),
            suggestions,
        );
    }

    format!(
        "You are a race coach for an iRacing practice session. Use only the data below to deliver concise, actionable coaching. Keep the answer under 180 words and speak like a calm pit lane engineer.\n\n{}\nKey suggestions:\n{}\n\nAnswer with a short coaching summary and 3 concrete improvements.",
        data_lines(summary),
        suggestions,
    )
}
