//! Saved track maps in `data/tracks/<track>.json`.
//!
//! The first run on a track creates the file. After that the saved map is reused, so turn
//! labels can be edited by hand (e.g. rename "8" to "10a") and survive later runs. A map
//! built from GPS (.ibt imports) replaces one dead-reckoned from live heading data, but
//! keeps the saved turns.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::trace::{detect_turns, RunData, TrackInfo, Turn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackMap {
    pub track_name: String,
    pub display_name: String,
    pub config_name: String,
    pub length_m: f64,
    /// "gps" or "heading".
    pub source: String,
    #[serde(default)]
    pub sector_pcts: Vec<f64>,
    pub turns: Vec<Turn>,
    /// Outline in metres (x east, y north), evenly spaced by lap distance from the start line.
    pub points: Vec<[f32; 2]>,
}

fn track_path(track_name: &str) -> Option<PathBuf> {
    let slug: String = track_name
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let slug = slug.trim_matches('-').to_string();
    (!slug.is_empty()).then(|| PathBuf::from("data").join("tracks").join(format!("{slug}.json")))
}

pub fn load_saved(track_name: &str) -> Option<TrackMap> {
    let path = track_path(track_name)?;
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn save(map: &TrackMap) {
    let Some(path) = track_path(&map.track_name) else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match serde_json::to_string_pretty(map) {
        Ok(text) => {
            if let Err(err) = std::fs::write(&path, text) {
                eprintln!("Could not save track map {}: {err}", path.display());
            }
        }
        Err(err) => eprintln!("Could not serialize track map: {err}"),
    }
}

/// Replace a track's turn labels and persist them.
pub fn update_turns(map: &mut TrackMap, turns: Vec<Turn>) {
    map.turns = turns;
    if !map.track_name.is_empty() {
        save(map);
    }
}

/// Fill in sector boundaries from a saved map when the source (live telemetry) lacks them.
pub fn with_saved_sectors(mut track: TrackInfo) -> TrackInfo {
    if track.sector_pcts.len() < 2 {
        if let Some(saved) = load_saved(&track.track_name) {
            track.sector_pcts = saved.sector_pcts;
        }
    }
    track
}

/// The map to show for a run: the saved one, upgraded or created from this run as needed.
pub fn resolve(track: &TrackInfo, run: &RunData) -> Option<TrackMap> {
    let saved = load_saved(&track.track_name);
    let Some(points) = run.map_points.clone() else { return saved };
    let source = if run.map_from_gps { "gps" } else { "heading" };

    match saved {
        Some(mut saved) => {
            let mut changed = false;
            if saved.source != "gps" && source == "gps" {
                saved.points = points;
                saved.source = source.to_string();
                changed = true;
            }
            if saved.sector_pcts.len() < 2 && track.sector_pcts.len() > 1 {
                saved.sector_pcts = track.sector_pcts.clone();
                changed = true;
            }
            if changed {
                save(&saved);
            }
            Some(saved)
        }
        None => {
            let length_m = if track.length_m > 0.0 {
                track.length_m
            } else {
                points.windows(2).map(|w| ((w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])) as f64).sum()
            };
            let map = TrackMap {
                track_name: track.track_name.clone(),
                display_name: track.display_name.clone(),
                config_name: track.config_name.clone(),
                length_m,
                source: source.to_string(),
                sector_pcts: track.sector_pcts.clone(),
                turns: detect_turns(&points, length_m),
                points,
            };
            if !track.track_name.is_empty() {
                save(&map);
            }
            Some(map)
        }
    }
}
