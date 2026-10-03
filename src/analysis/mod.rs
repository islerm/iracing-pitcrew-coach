//! Turning laps and traces into findings: session summary, corner and handling notes, gears.

pub mod corner;
pub mod gears;
pub mod handling;

use crate::stats::mean;
use crate::telemetry::trace::LapMetrics;
use crate::telemetry::weather::SessionWeather;

#[derive(Debug, Clone)]
pub struct SessionSummary {
    pub fastest_lap: i32,
    pub fastest_lap_time_s: f64,
    pub average_lap_time_s: f64,
    pub slowest_sector_name: Option<String>,
    pub slowest_sector_time_s: Option<f64>,
    pub average_speed_kph: Option<f64>,
    pub suggestions: Vec<String>,
    /// Corner-by-corner findings from the lap traces (see `handling::corner_notes`).
    pub corner_notes: Vec<String>,
    /// Conditions during the run (see `weather::session_weather`).
    pub weather: Option<SessionWeather>,
}

pub fn summarize_session(laps: &[LapMetrics]) -> SessionSummary {
    let complete_laps: Vec<&LapMetrics> = laps.iter().filter(|lap| lap.is_complete).collect();

    if complete_laps.is_empty() {
        return SessionSummary {
            fastest_lap: 0,
            fastest_lap_time_s: 0.0,
            average_lap_time_s: 0.0,
            slowest_sector_name: None,
            slowest_sector_time_s: None,
            average_speed_kph: None,
            suggestions: vec![
                "No complete laps were recorded in this split, so there is no valid fastest lap yet.".to_string(),
                "Keep recording until you cross the finish line again so the app can compare full laps.".to_string(),
            ],
            corner_notes: Vec::new(),
            weather: None,
        };
    }

    let fastest = complete_laps
        .iter()
        .min_by(|a, b| a.lap_time_s.partial_cmp(&b.lap_time_s).unwrap())
        .unwrap();

    let average_lap_time = mean(complete_laps.iter().map(|lap| lap.lap_time_s)).unwrap_or_default();

    // Sectors differ in length, so rank them by how much the fastest lap lost against the
    // best time you've set in that same sector, not by raw sector time.
    let mut sector_losses: Vec<(String, f64, f64)> = Vec::new();
    for (index, &value) in fastest.sectors.iter().enumerate() {
        let best = complete_laps
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


    let avg_speed = mean(complete_laps.iter().filter_map(|lap| lap.avg_speed_kph));

    let mut suggestions = Vec::new();
    if let Some(name) = &slowest_sector_name {
        suggestions.push(format!(
            "Focus on {} to recover time: on your fastest lap it was furthest off your best time for that sector.",
            name.to_ascii_lowercase()
        ));
    }
    if complete_laps.iter().any(|lap| lap.lap_time_s - fastest.lap_time_s > 0.3) {
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
        suggestions,
        corner_notes: Vec::new(),
        weather: None,
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

/// Attaches the run's conditions, with any suggestions they lead to (a big swing in track
/// temperature, a wet track).
pub fn add_weather(summary: &mut SessionSummary, weather: Option<SessionWeather>, laps: &[LapMetrics]) {
    let Some(mut weather) = weather else { return };
    summary.suggestions.extend(crate::telemetry::weather::annotate(&mut weather, laps, summary.fastest_lap));
    summary.weather = Some(weather);
}
