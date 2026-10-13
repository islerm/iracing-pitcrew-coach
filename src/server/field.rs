//! The other drivers in a run's session: saving them to the store, the class leaderboard and
//! the corner-by-corner comparison with the quickest cars (`GET /api/splits/:id/field`).

use std::sync::Arc;

use axum::extract::{Path, State};
use serde::Serialize;

use super::splits::{NewRun, PracticeSplit};
use super::{blocking, ApiResult, AppState};
use crate::field::compare::{
    compare, library_references, pick_references, radio_line, your_best, FieldComparison,
};
use crate::field::{Field, FieldDriver, OpponentLap};
use crate::store::SessionKey;
use crate::telemetry::trace::{LapMetrics, LapTrace, RunData, Turn};

/// How many earlier-session laps to consider as references when this session has none.
const LIBRARY_LIMIT: usize = 50;

const FILE_NOTE: &str = "Telemetry files don't contain other cars' data, so only the session results are shown. Record live in a session with other drivers to compare corner by corner.";
const NO_REFERENCES_NOTE: &str = "No other cars in your class set a clean lap in similar conditions while you were recording.";
const NO_MAP_NOTE: &str = "Corner-by-corner comparison needs a track map for this run.";

/// Saves the run's roster and recorded laps to the store. Never fails the run: problems are
/// logged. Blocking (database IO), so call it off the async runtime.
pub(super) fn save_run_field(state: &AppState, run: &NewRun) -> Option<i64> {
    let store = state.store.as_ref()?;
    if run.track_name.is_empty() {
        return None;
    }
    let mut store = store.lock().ok()?;
    let roster = &run.field.roster;
    let key = SessionKey {
        sub_session_id: roster.sub_session_id,
        session_num: roster.session_num,
        session_type: &run.session_type,
        track_name: &run.track_name,
        track_id: run.track_id,
        started_at_ms: run.started_at_ms,
    };
    let saved = store.upsert_session(&key).and_then(|id| store.save_field(id, &run.field).map(|()| id));
    match saved {
        Ok(id) => Some(id),
        Err(err) => {
            eprintln!("Could not save the session's field to the store: {err:#}");
            None
        }
    }
}

/// The comparison with the field for the player's best lap, against this session's quickest
/// class rivals or, when there are none, `library()`'s laps from earlier sessions. Returns
/// the source of the references too ("session" or "library").
fn field_comparison(
    field: &Field,
    laps: &[LapMetrics],
    traces: &[LapTrace],
    turns: &[Turn],
    length_m: f64,
    library: impl FnOnce() -> Vec<(OpponentLap, FieldDriver)>,
) -> Option<(FieldComparison, &'static str)> {
    let (trace, metrics) = your_best(laps, traces)?;
    if turns.is_empty() || length_m <= 0.0 {
        return None;
    }
    let refs = pick_references(&field.laps, &field.roster, metrics);
    if !refs.is_empty() {
        return compare(trace, metrics, &refs, turns, length_m).map(|c| (c, "session"));
    }
    let library = library();
    let refs = library_references(&library, metrics);
    compare(trace, metrics, &refs, turns, length_m).map(|c| (c, "library"))
}

/// One sentence about the field for the pit debrief, from this session's laps only.
pub(super) fn pit_field_line(field: &Field, run: &RunData, turns: &[Turn], length_m: f64) -> Option<String> {
    let (cmp, _) = field_comparison(field, &run.laps, &run.traces, turns, length_m, Vec::new)?;
    radio_line(&cmp)
}

#[derive(Debug, Serialize, PartialEq)]
struct LeaderRow {
    car_idx: i32,
    name: String,
    /// Short class name ("GT3 Class"), shown when several classes are listed.
    class_name: String,
    irating: i64,
    license: String,
    car: String,
    is_player: bool,
    best_lap_s: Option<f64>,
    laps: Option<i32>,
    /// Behind the class's best lap (0 for the best).
    gap_s: Option<f64>,
}

fn usable(time: f64) -> Option<f64> {
    (time.is_finite() && time > 0.0).then_some(time)
}

fn faster(a: Option<f64>, b: Option<f64>) -> Option<f64> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// The player's class, ordered by best lap, quickest first and drivers without a time last.
/// When the roster doesn't say who the player is, every class is listed, grouped by class
/// (quickest class first). Gaps are always to the best in the driver's own class: in a
/// multi-class field a GTP's lap says nothing about a GT3 driver's.
fn build_leaderboard(field: &Field, player_best: Option<f64>) -> Vec<LeaderRow> {
    let roster = &field.roster;
    let all = roster.player().is_none();
    let mut rows: Vec<(&FieldDriver, LeaderRow)> = roster
        .drivers
        .iter()
        .filter(|d| all || d.is_player || d.car_idx == roster.player_car_idx || roster.in_player_class(d.car_idx))
        .map(|d| {
            let is_player = d.car_idx == roster.player_car_idx;
            let result = roster.results.iter().find(|r| r.car_idx == d.car_idx);
            let from_result = result.and_then(|r| r.fastest_time_s).and_then(usable);
            let best_lap_s = if is_player {
                player_best.and_then(usable).or(from_result)
            } else {
                let recorded = field
                    .laps
                    .iter()
                    .filter(|l| l.car_idx == d.car_idx && l.valid)
                    .filter_map(|l| usable(l.lap_time_s))
                    .min_by(f64::total_cmp);
                faster(from_result, recorded)
            };
            let row = LeaderRow {
                car_idx: d.car_idx,
                name: d.name.clone(),
                class_name: d.class_name.clone(),
                irating: d.irating,
                license: d.license.clone(),
                car: d.car.clone(),
                is_player,
                best_lap_s,
                laps: result.map(|r| r.laps_complete),
                gap_s: None,
            };
            (d, row)
        })
        .collect();

    // Each driver's class best, then gaps within the class.
    let class_best: Vec<Option<f64>> = rows
        .iter()
        .map(|(d, _)| rows.iter().filter(|(o, _)| o.same_class(d)).filter_map(|(_, r)| r.best_lap_s).min_by(f64::total_cmp))
        .collect();
    for ((_, row), best) in rows.iter_mut().zip(&class_best) {
        row.gap_s = row.best_lap_s.zip(*best).map(|(t, best)| t - best);
    }
    let order = |t: Option<f64>| t.unwrap_or(f64::INFINITY);
    let mut keyed: Vec<(f64, LeaderRow)> = rows.into_iter().zip(class_best).map(|((_, row), best)| (order(best), row)).collect();
    // Quickest class first (a single class in the usual case), then by lap within it.
    keyed.sort_by(|(ca, a), (cb, b)| ca.total_cmp(cb).then_with(|| a.class_name.cmp(&b.class_name)).then_with(|| order(a.best_lap_s).total_cmp(&order(b.best_lap_s))));
    keyed.into_iter().map(|(_, row)| row).collect()
}

/// What to tell the driver about the limits of the data, when there are any.
fn source_note(source: &str, has_comparison: bool, has_map: bool) -> &'static str {
    if source != "Live" {
        FILE_NOTE
    } else if has_comparison {
        ""
    } else if has_map {
        NO_REFERENCES_NOTE
    } else {
        NO_MAP_NOTE
    }
}

#[derive(Debug, Serialize)]
pub(super) struct FieldResponse {
    class_name: String,
    source_note: &'static str,
    leaderboard: Vec<LeaderRow>,
    comparison: Option<FieldComparison>,
    /// "session" or "library"; `None` without a comparison.
    reference_source: Option<&'static str>,
}

fn build_response(split: &PracticeSplit, library: impl FnOnce() -> Vec<(OpponentLap, FieldDriver)>) -> FieldResponse {
    let run = &split.run;
    let player_best = your_best(&run.laps, &run.traces).map(|(_, m)| m.lap_time_s);
    let track = split.track();
    let found = track
        .as_ref()
        .and_then(|map| field_comparison(&run.field, &run.laps, &run.traces, &map.turns, map.length_m, library));
    let (comparison, reference_source) = match found {
        Some((cmp, source)) => (Some(cmp), Some(source)),
        None => (None, None),
    };
    FieldResponse {
        class_name: run.field.roster.player().map(|d| d.class_name.clone()).unwrap_or_default(),
        source_note: source_note(&run.source, comparison.is_some(), track.is_some()),
        leaderboard: build_leaderboard(&run.field, player_best),
        comparison,
        reference_source,
    }
}

pub(super) async fn get_field(State(state): State<Arc<AppState>>, Path(id): Path<String>) -> ApiResult<FieldResponse> {
    let split = state.split(&id)?;
    let response = blocking("Field comparison", move || {
        let library = || {
            let (Some(store), Some(me)) = (state.store.as_ref(), split.run.field.roster.player()) else {
                return Vec::new();
            };
            // Without a known class, earlier sessions' laps could be from any class.
            if me.class_id <= 0 {
                return Vec::new();
            }
            let Ok(store) = store.lock() else { return Vec::new() };
            store
                .library_laps(&split.run.track_name, me.class_id, split.session_id, LIBRARY_LIMIT)
                .unwrap_or_else(|err| {
                    eprintln!("Could not read the lap library: {err:#}");
                    Vec::new()
                })
        };
        build_response(&split, library)
    })
    .await?;
    Ok(axum::Json(response))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::field::{FieldResult, Roster};

    fn driver(car_idx: i32, class_id: i64, is_player: bool) -> FieldDriver {
        FieldDriver { car_idx, name: format!("Driver {car_idx}"), class_id, irating: 2000, is_player, ..Default::default() }
    }

    fn result(car_idx: i32, time: Option<f64>) -> FieldResult {
        FieldResult { car_idx, fastest_time_s: time, laps_complete: 5, ..Default::default() }
    }

    fn opp(car_idx: i32, lap_time_s: f64, valid: bool) -> OpponentLap {
        let n = 100;
        OpponentLap {
            car_idx,
            lap_number: 3,
            lap_time_s,
            valid,
            track_wetness: Some(1),
            time_s: (0..=n).map(|i| (lap_time_s * i as f64 / n as f64) as f32).collect(),
            gear: vec![3; n + 1],
        }
    }

    fn field() -> Field {
        Field {
            roster: Roster {
                player_car_idx: 0,
                drivers: vec![driver(0, 1, true), driver(1, 1, false), driver(2, 1, false), driver(3, 2, false), driver(4, 1, false)],
                results: vec![result(1, Some(92.0)), result(2, None), result(3, Some(80.0)), result(4, None)],
                ..Default::default()
            },
            laps: vec![opp(2, 90.5, true), opp(2, 85.0, false), opp(4, 95.0, true)],
        }
    }

    #[test]
    fn multi_class_leaderboards_never_mix_classes() {
        let gt3 = |car_idx: i32, is_player: bool| FieldDriver { class_name: "GT3".into(), ..driver(car_idx, 3, is_player) };
        let gtp = |car_idx: i32| FieldDriver { class_name: "GTP".into(), ..driver(car_idx, 1, false) };
        let mut field = Field {
            roster: Roster {
                player_car_idx: 0,
                drivers: vec![gt3(0, true), gt3(1, false), gtp(2), gtp(3)],
                results: vec![result(1, Some(100.0)), result(2, Some(90.0)), result(3, Some(91.0))],
                ..Default::default()
            },
            laps: Vec::new(),
        };

        // The player known: only their class, gaps to the GT3 best, not the GTPs'.
        let rows = build_leaderboard(&field, Some(101.0));
        assert_eq!(rows.iter().map(|r| r.car_idx).collect::<Vec<_>>(), vec![1, 0]);
        assert_eq!(rows[1].gap_s, Some(1.0));

        // The player unknown: every class, grouped quickest class first, gaps within each.
        field.roster.player_car_idx = -1;
        for d in &mut field.roster.drivers {
            d.is_player = false;
        }
        let rows = build_leaderboard(&field, None);
        let summary: Vec<(i32, Option<f64>)> = rows.iter().map(|r| (r.car_idx, r.gap_s)).collect();
        assert_eq!(summary, vec![(2, Some(0.0)), (3, Some(1.0)), (1, Some(0.0)), (0, None)]);
    }

    #[test]
    fn leaderboard_is_the_players_class_sorted_with_gaps() {
        let rows = build_leaderboard(&field(), Some(91.0));
        let order: Vec<i32> = rows.iter().map(|r| r.car_idx).collect();
        // Car 3 is in another class; 2's invalid 85.0 lap doesn't count.
        assert_eq!(order, vec![2, 0, 1, 4]);
        assert_eq!(rows[0].best_lap_s, Some(90.5));
        assert_eq!(rows[0].gap_s, Some(0.0));
        assert!(rows[1].is_player && (rows[1].gap_s.unwrap() - 0.5).abs() < 1e-9);
        assert_eq!(rows[2].best_lap_s, Some(92.0));
        assert_eq!(rows[3].best_lap_s, Some(95.0));
    }

    #[test]
    fn leaderboard_keeps_drivers_without_times_last_and_falls_back_to_results_for_the_player() {
        let mut f = field();
        f.laps.clear();
        f.roster.results.push(result(0, Some(93.0)));
        let rows = build_leaderboard(&f, None);
        let order: Vec<i32> = rows.iter().map(|r| r.car_idx).collect();
        assert_eq!(order, vec![1, 0, 2, 4]);
        assert_eq!(rows[1].best_lap_s, Some(93.0));
        assert_eq!(rows[2].best_lap_s, None);
        assert_eq!(rows[2].gap_s, None);
    }

    #[test]
    fn leaderboard_takes_the_quicker_of_results_and_recorded_laps() {
        let mut f = field();
        f.roster.results[0].fastest_time_s = Some(89.0);
        let rows = build_leaderboard(&f, Some(100.0));
        assert_eq!(rows.iter().find(|r| r.car_idx == 1).unwrap().best_lap_s, Some(89.0));
    }

    #[test]
    fn source_note_explains_the_limits() {
        assert_eq!(source_note("run.ibt", false, true), FILE_NOTE);
        assert_eq!(source_note("Live", true, true), "");
        assert_eq!(source_note("Live", false, true), NO_REFERENCES_NOTE);
        assert_eq!(source_note("Live", false, false), NO_MAP_NOTE);
    }

    fn my_lap() -> (Vec<LapMetrics>, Vec<LapTrace>) {
        let n = 100;
        // Slower than the references everywhere.
        let time_s: Vec<f32> = (0..=n).map(|i| 100.0 * i as f32 / n as f32).collect();
        let trace = LapTrace {
            lap_number: 4,
            speed_kph: vec![0.0; n + 1],
            throttle: vec![0.0; n + 1],
            brake: vec![0.0; n + 1],
            gear: vec![3; n + 1],
            steer_deg: vec![0.0; n + 1],
            lat_g: Vec::new(),
            long_g: Vec::new(),
            yaw_rate_dps: Vec::new(),
            abs: Vec::new(),
            balance_deg: Vec::new(),
            rpm: Vec::new(),
            alt_m: Vec::new(),
            time_s,
        };
        let metrics = LapMetrics {
            lap_number: 4,
            lap_time_s: 100.0,
            is_complete: true,
            sectors: Vec::new(),
            avg_speed_kph: None,
            off_track_pcts: Vec::new(),
            incidents: None,
            air_temp_c: None,
            track_temp_c: None,
            track_wetness: Some(1),
            top_speed_kph: None,
            full_throttle_pct: None,
            braking_pct: None,
        };
        (vec![metrics], vec![trace])
    }

    #[test]
    fn comparison_uses_this_sessions_class_rivals() {
        let (laps, traces) = my_lap();
        let turns = vec![Turn { label: "1".into(), pct: 0.25 }, Turn { label: "2".into(), pct: 0.75 }];
        let mut library_asked = false;
        let found = field_comparison(&field(), &laps, &traces, &turns, 2000.0, || {
            library_asked = true;
            Vec::new()
        });
        let (cmp, source) = found.expect("comparison");
        assert_eq!(source, "session");
        assert!(!library_asked);
        assert_eq!(cmp.your_lap, 4);
        // Cars 2 (90.5) and 4 (95.0); car 1 has no recorded lap.
        let names: Vec<&str> = cmp.references.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["Driver 2", "Driver 4"]);
        assert!(cmp.references.iter().all(|r| r.this_session));
        assert_eq!(cmp.corners.len(), 2);
        assert!(cmp.corners.iter().all(|c| c.gap_s > 0.0));
        assert!(pit_field_line(&field(), &RunData { laps, traces, map_points: None, map_from_gps: false }, &turns, 2000.0)
            .is_some_and(|l| l.contains("Turn")));
    }

    #[test]
    fn comparison_falls_back_to_the_library_and_needs_turns() {
        let (laps, traces) = my_lap();
        let mut f = field();
        f.laps.clear();
        let turns = vec![Turn { label: "1".into(), pct: 0.5 }];
        let lib = || vec![(opp(7, 91.0, true), driver(7, 1, false))];
        let (cmp, source) = field_comparison(&f, &laps, &traces, &turns, 2000.0, lib).expect("comparison");
        assert_eq!(source, "library");
        assert!(!cmp.references[0].this_session);
        assert!(field_comparison(&f, &laps, &traces, &[], 2000.0, lib).is_none());
        assert!(field_comparison(&f, &laps, &traces, &turns, 2000.0, Vec::new).is_none());
    }
}
