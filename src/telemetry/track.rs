//! Saved track maps in `data/tracks/<track>.json`.
//!
//! The first run on a track creates the file. After that the saved map is reused, so turn
//! labels can be edited by hand (e.g. rename "8" to "10a") and survive later runs. A map
//! built from GPS (.ibt imports) replaces one dead-reckoned from live heading data, but
//! keeps the saved turns.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::telemetry::trace::{detect_turns, TrackInfo, Turn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackMap {
    pub track_name: String,
    /// iRacing's `TrackID`: with `track_name` it locates the official map, whose turn numbers
    /// the UI copies.
    #[serde(default)]
    pub track_id: Option<u32>,
    /// Where `turns` came from: "detected" (from the outline's curvature), "official"
    /// (iRacing's map) or "manual" (edited by hand). Only detected turns are replaced
    /// automatically.
    #[serde(default = "detected")]
    pub turns_source: String,
    /// Clockwise rotation (degrees) the UI draws the map with, so it can match the sim's.
    #[serde(default)]
    pub rotation_deg: f64,
    pub length_m: f64,
    /// "gps" or "heading".
    pub source: String,
    #[serde(default)]
    pub sector_pcts: Vec<f64>,
    pub turns: Vec<Turn>,
    /// Outline in metres (x east, y north), evenly spaced by lap distance from the start line.
    pub points: Vec<[f32; 2]>,
}

/// Lower-case alphanumeric-and-dash form of a track name, used for file names. Empty when
/// the name has nothing usable.
pub fn slug(name: &str) -> String {
    let slug: String = name
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    slug.trim_matches('-').to_string()
}

fn detected() -> String {
    "detected".to_string()
}

fn track_path(track_name: &str) -> Option<PathBuf> {
    let slug = slug(track_name);
    (!slug.is_empty()).then(|| PathBuf::from("data").join("tracks").join(format!("{slug}.json")))
}

fn load_saved(track_name: &str) -> Option<TrackMap> {
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

/// Persist a map after its turns or view settings were changed.
pub fn store(map: &TrackMap) {
    if !map.track_name.is_empty() {
        save(map);
    }
}

/// Loads the track's saved map (once) and fills in sector boundaries from it when the
/// source (live telemetry) lacks them. Hand the map to `resolve` so the file isn't read twice.
pub fn with_saved(mut track: TrackInfo) -> (TrackInfo, Option<TrackMap>) {
    let saved = load_saved(&track.track_name);
    if track.sector_pcts.len() < 2 {
        if let Some(saved) = &saved {
            track.sector_pcts = saved.sector_pcts.clone();
        }
    }
    (track, saved)
}

/// The map to show for a run: the saved one (from `with_saved`), upgraded or created from
/// this run's outline as needed.
pub fn resolve(
    track: &TrackInfo,
    saved: Option<TrackMap>,
    map_points: Option<Vec<[f32; 2]>>,
    map_from_gps: bool,
) -> Option<TrackMap> {
    let Some(points) = map_points else { return saved };
    let source = if map_from_gps { "gps" } else { "heading" };

    match saved {
        Some(mut saved) => {
            let mut changed = false;
            if saved.source != "gps" && source == "gps" {
                saved.points = points;
                saved.source = source.to_string();
                changed = true;
            }
            if saved.track_id.is_none() && track.track_id.is_some() {
                saved.track_id = track.track_id;
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
                track_id: track.track_id,
                turns_source: detected(),
                rotation_deg: 0.0,
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
