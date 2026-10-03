//! Practice splits: a recorded or imported run with its laps, traces, track map and coaching,
//! and the endpoints that read or change one.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};

use super::{blocking, epoch_ms_now, error_response, modified_ms, pick_model, ApiError, ApiResult, AppState, NO_TRACK};
use crate::{
    analysis::{
        add_corner_notes, add_weather,
        corner::{corner_report, CornerReport},
        gears::{shift_report, ShiftReport},
        handling::{corner_notes, peak_combined_g},
        summarize_session, SessionSummary,
    },
    coach::{ask_model, generate_feedback, prompt::corner_prompt},
    telemetry::{
        ibt::read_ibt,
        track::{self, TrackMap},
        trace::{build_run, Frame, LapMetrics, LapTrace, TrackInfo, Turn},
        weather::{session_weather, SessionWeather},
    },
};

/// A run's data before the summary and feedback are generated.
pub(super) struct NewRun {
    source: String,
    track_label: String,
    car: String,
    started_at_ms: u128,
    laps: Vec<LapMetrics>,
    traces: Vec<LapTrace>,
    shift_rpm: Option<f64>,
    redline_rpm: Option<f64>,
    /// Handed to the summary when the run is analysed.
    weather: Option<SessionWeather>,
}

impl NewRun {
    /// Split frames into laps and traces, and find the (saved or newly built) track map.
    pub(super) fn from_frames(source: String, started_at_ms: u128, track_info: TrackInfo, frames: &[Frame]) -> (Self, Option<TrackMap>) {
        let (track_info, saved) = track::with_saved(track_info);
        let run = build_run(frames, &track_info);
        let track = track::resolve(&track_info, saved, run.map_points, run.map_from_gps);
        let track_label = match (track_info.display_name.as_str(), track_info.config_name.as_str()) {
            ("", _) => String::new(),
            (name, "") => name.to_string(),
            (name, config) => format!("{name} · {config}"),
        };
        let new_run = NewRun {
            source,
            track_label,
            car: track_info.car,
            started_at_ms,
            laps: run.laps,
            traces: run.traces,
            shift_rpm: track_info.shift_rpm,
            redline_rpm: track_info.redline_rpm,
            weather: session_weather(frames, &track_info.weather),
        };
        (new_run, track)
    }
}

pub(super) struct PracticeSplit {
    pub(super) id: String,
    ended_at_ms: u128,
    model: String,
    run: NewRun,
    /// Behind a lock because renaming turns updates every run on the same track.
    track: Mutex<Option<TrackMap>>,
    summary: SessionSummary,
    feedback: String,
    /// Session peak combined g (98th percentile over all traces), the 100% mark for grip use.
    peak_g: Option<f64>,
}

impl PracticeSplit {
    fn track(&self) -> Option<TrackMap> {
        self.track.lock().expect("track lock poisoned").clone()
    }

    fn require_track(&self) -> Result<TrackMap, ApiError> {
        self.track().ok_or_else(|| error_response(StatusCode::NOT_FOUND, NO_TRACK))
    }

    fn best_lap_speed(&self) -> Option<f64> {
        self.run
            .laps
            .iter()
            .find(|lap| lap.lap_number == self.summary.fastest_lap)
            .and_then(|lap| lap.avg_speed_kph)
    }

    fn overview(&self) -> SplitOverviewResponse {
        SplitOverviewResponse {
            id: self.id.clone(),
            source: self.run.source.clone(),
            track_label: self.run.track_label.clone(),
            car: self.run.car.clone(),
            started_at_ms: self.run.started_at_ms,
            lap_count: self.run.laps.len(),
            complete_lap_count: self.run.laps.iter().filter(|lap| lap.is_complete).count(),
            fastest_lap_time_s: self.summary.fastest_lap_time_s,
        }
    }

    fn detail(&self) -> SplitDetailResponse {
        SplitDetailResponse {
            id: self.id.clone(),
            source: self.run.source.clone(),
            track_label: self.run.track_label.clone(),
            car: self.run.car.clone(),
            started_at_ms: self.run.started_at_ms,
            ended_at_ms: self.ended_at_ms,
            model: self.model.clone(),
            lap_count: self.run.laps.len(),
            has_track: self.track.lock().expect("track lock poisoned").is_some(),
            traced_laps: self.run.traces.iter().map(|trace| trace.lap_number).collect(),
            peak_g: self.peak_g,
            shift_rpm: self.run.shift_rpm,
            weather: self.summary.weather.clone(),
            suggestions: self.summary.suggestions.clone(),
            feedback: self.feedback.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
pub(super) struct SplitOverviewResponse {
    id: String,
    source: String,
    track_label: String,
    car: String,
    started_at_ms: u128,
    lap_count: usize,
    complete_lap_count: usize,
    fastest_lap_time_s: f64,
}

#[derive(Debug, Serialize)]
pub(super) struct SplitDetailResponse {
    id: String,
    source: String,
    track_label: String,
    car: String,
    started_at_ms: u128,
    ended_at_ms: u128,
    model: String,
    lap_count: usize,
    has_track: bool,
    traced_laps: Vec<i32>,
    /// Session peak combined g, the 100% mark for grip use.
    peak_g: Option<f64>,
    /// The car's shift-light RPM, when the source records it.
    shift_rpm: Option<f64>,
    weather: Option<SessionWeather>,
    suggestions: Vec<String>,
    feedback: String,
}

#[derive(Debug, Serialize)]
pub(super) struct LapInsight {
    #[serde(flatten)]
    lap: LapMetrics,
    went_well: Vec<String>,
    went_bad: Vec<String>,
}

fn build_lap_insight(lap: &LapMetrics, best_lap_time: f64, best_avg_speed: Option<f64>) -> LapInsight {
    let delta_to_best_s = lap.lap_time_s - best_lap_time;
    let mut went_well = Vec::new();
    let mut went_bad = Vec::new();

    if delta_to_best_s <= 0.150 {
        went_well.push("Pace was very close to your best lap.".to_string());
    } else {
        went_bad.push(format!(
            "Lost {:.3}s vs your best lap in this split.",
            delta_to_best_s
        ));
    }

    match (lap.avg_speed_kph, best_avg_speed) {
        (Some(speed), Some(best_speed)) if speed >= best_speed * 0.99 => {
            went_well.push("Average speed stayed near your best-lap level.".to_string());
        }
        (Some(speed), Some(best_speed)) => {
            went_bad.push(format!(
                "Average speed was {:.1} kph below the best-lap benchmark.",
                best_speed - speed
            ));
        }
        _ => {}
    }

    match lap.off_track_pcts.len() {
        0 => {}
        1 => went_bad.push("Went off track once: the time may not be representative.".to_string()),
        n => went_bad.push(format!("Went off track {n} times: the time may not be representative.")),
    }

    if went_well.is_empty() {
        went_well.push("No standout strength flagged from this telemetry slice.".to_string());
    }
    if went_bad.is_empty() {
        went_bad.push("No major weakness flagged versus your best lap.".to_string());
    }

    LapInsight { lap: lap.clone(), went_well, went_bad }
}

#[derive(Debug, Deserialize)]
pub(super) struct ImportRequest {
    path: String,
    model: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct UpdateTurnsRequest {
    turns: Vec<Turn>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CornerRequest {
    lap: i32,
    /// Compare against this lap; `None` compares against the typical lap from the others.
    ref_lap: Option<i32>,
    /// Laps the driver left out of the stats: not used as the typical lap or in rankings.
    #[serde(default)]
    exclude: Vec<i32>,
    /// Also ask the coach model for written advice (slow).
    #[serde(default)]
    coach: bool,
    model: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct CornerResponse {
    report: CornerReport,
    feedback: Option<String>,
    model: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ShiftsQuery {
    /// Comma-separated laps left out of the stats.
    exclude: Option<String>,
}

/// Summarizes the laps, asks the coach model for feedback (off the async runtime, since
/// Ollama can take a while) and stores the result as a new split.
pub(super) async fn finalize_split(state: &Arc<AppState>, run: NewRun, track: Option<TrackMap>, model: String) -> ApiResult<SplitDetailResponse> {
    if run.laps.is_empty() {
        return Err(error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "No usable lap data captured. Try recording while driving and for longer.",
        ));
    }
    let id = {
        let mut guard = state.lock();
        guard.next_split_id += 1;
        format!("split-{}", guard.next_split_id - 1)
    };

    let split = blocking("Analysis", move || {
        let mut run = run;
        let mut summary = summarize_session(&run.laps);
        add_weather(&mut summary, run.weather.take(), &run.laps);
        if let Some(track) = &track {
            add_corner_notes(&mut summary, corner_notes(&run.traces, &track.turns, track.length_m));
        }
        // Early/late upshifts and rev-limiter time go to the coach and the "next time" list.
        let length_m = track.as_ref().map(|t| t.length_m).unwrap_or(0.0);
        let traces: Vec<&LapTrace> = run.traces.iter().collect();
        let shifts = shift_report(&traces, run.shift_rpm, run.redline_rpm, length_m);
        summary.suggestions.extend(shifts.notes.into_iter().filter(|n| !n.ends_with("are on time.")));
        let feedback = generate_feedback(&summary, &model);
        let peak_g = peak_combined_g(&run.traces);
        Arc::new(PracticeSplit { id, ended_at_ms: epoch_ms_now(), model, run, track: Mutex::new(track), summary, feedback, peak_g })
    })
    .await?;

    let response = split.detail();
    state.lock().splits.push(split);
    Ok(Json(response))
}

pub(super) async fn import_split(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<ImportRequest>,
) -> ApiResult<SplitDetailResponse> {
    let path = PathBuf::from(payload.path.trim().trim_matches('"'));
    if path.as_os_str().is_empty() {
        return Err(error_response(StatusCode::BAD_REQUEST, "Enter an .ibt file or folder path."));
    }
    if !path.exists() {
        return Err(error_response(
            StatusCode::NOT_FOUND,
            format!("Path not found: {}", path.display()),
        ));
    }
    if path.is_dir() {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "That's a folder — pick a single .ibt file from it.",
        ));
    }
    if !path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("ibt")) {
        return Err(error_response(StatusCode::BAD_REQUEST, "Unsupported file type: choose an .ibt file."));
    }

    let source = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string());
    let (run, track) = blocking("Import", move || -> Result<(NewRun, Option<TrackMap>)> {
        let started_at_ms = std::fs::metadata(&path).ok().and_then(|meta| modified_ms(&meta)).unwrap_or_else(epoch_ms_now);
        let data = read_ibt(&path)?;
        Ok(NewRun::from_frames(source, started_at_ms, data.track, &data.frames))
    })
    .await?
    .map_err(|err| error_response(StatusCode::UNPROCESSABLE_ENTITY, err.to_string()))?;

    let model = pick_model(payload.model, &state.lock().default_model);
    finalize_split(&state, run, track, model).await
}

pub(super) async fn get_split_track(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> ApiResult<TrackMap> {
    Ok(Json(state.split(&id)?.require_track()?))
}

pub(super) async fn update_track_turns(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(payload): Json<UpdateTurnsRequest>,
) -> ApiResult<TrackMap> {
    let mut turns: Vec<Turn> = payload
        .turns
        .into_iter()
        .filter(|turn| (0.0..=1.0).contains(&turn.pct))
        .map(|turn| Turn { label: turn.label.trim().chars().take(12).collect(), pct: turn.pct })
        .collect();
    turns.sort_by(|a, b| a.pct.partial_cmp(&b.pct).unwrap_or(std::cmp::Ordering::Equal));

    let target = state.split(&id)?;
    let track_name = target.require_track()?.track_name;
    let splits = state.lock().splits.clone();
    // Other runs on the same track share the saved labels.
    let mut updated = None;
    for split in &splits {
        let mut guard = split.track.lock().expect("track lock poisoned");
        if let Some(map) = guard.as_mut().filter(|map| map.track_name == track_name) {
            map.turns = turns.clone();
            if split.id == id {
                updated = Some(map.clone());
            }
        }
    }
    let mut map = updated.ok_or_else(|| error_response(StatusCode::NOT_FOUND, NO_TRACK))?;
    // Persist the labels (file IO) with no lock held.
    let map = blocking("Saving turns", move || {
        track::update_turns(&mut map, turns);
        map
    })
    .await?;
    Ok(Json(map))
}

pub(super) async fn get_lap_trace(
    State(state): State<Arc<AppState>>,
    Path((id, lap_number)): Path<(String, i32)>,
) -> ApiResult<LapTrace> {
    let split = state.split(&id)?;
    split
        .run
        .traces
        .iter()
        .find(|trace| trace.lap_number == lap_number)
        .cloned()
        .map(Json)
        .ok_or_else(|| error_response(StatusCode::NOT_FOUND, "No telemetry trace for this lap."))
}

pub(super) async fn list_splits(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let splits = state.lock().splits.clone();
    Json(splits.iter().map(|split| split.overview()).collect::<Vec<_>>())
}

pub(super) async fn get_split(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> ApiResult<SplitDetailResponse> {
    Ok(Json(state.split(&id)?.detail()))
}

pub(super) async fn delete_split(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let mut guard = state.lock();
    let before = guard.splits.len();
    guard.splits.retain(|split| split.id != id);
    if guard.splits.len() == before {
        return Err(error_response(StatusCode::NOT_FOUND, "Split not found."));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn list_split_laps(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> ApiResult<Vec<LapInsight>> {
    let split = state.split(&id)?;
    let (best_time, best_speed) = (split.summary.fastest_lap_time_s, split.best_lap_speed());
    Ok(Json(split.run.laps.iter().map(|lap| build_lap_insight(lap, best_time, best_speed)).collect()))
}

/// Entry/exit breakdown of one corner on one lap, optionally with written advice from the
/// coach model.
pub(super) async fn analyze_corner(
    State(state): State<Arc<AppState>>,
    Path((id, turn)): Path<(String, usize)>,
    Json(payload): Json<CornerRequest>,
) -> ApiResult<CornerResponse> {
    let split = state.split(&id)?;
    let track = split.require_track()?;
    let model = pick_model(payload.model.clone(), &split.model);
    let CornerRequest { lap, ref_lap, exclude, coach, .. } = payload;

    let report = blocking("Corner analysis", move || {
        let keep = |n: i32| n == lap || Some(n) == ref_lap || !exclude.contains(&n);
        let traces: Vec<&LapTrace> = split.run.traces.iter().filter(|t| keep(t.lap_number)).collect();
        corner_report(&traces, &track.turns, track.length_m, split.peak_g, turn, lap, ref_lap)
    })
    .await?
    .ok_or_else(|| {
        error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Can't compare this corner: the lap needs a telemetry trace and there must be another lap to compare with.",
        )
    })?;

    if !coach {
        return Ok(Json(CornerResponse { report, feedback: None, model: None }));
    }

    let prompt = corner_prompt(&report);
    let (feedback, model) = blocking("Coach", move || (ask_model(&prompt, &model), model)).await?;
    let feedback = feedback.map_err(|err| error_response(StatusCode::SERVICE_UNAVAILABLE, err.to_string()))?;
    Ok(Json(CornerResponse { report, feedback: Some(feedback), model: Some(model) }))
}

/// Upshift RPMs for the counted laps, judged against the car's shift light.
pub(super) async fn get_shifts(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<ShiftsQuery>,
) -> ApiResult<ShiftReport> {
    let split = state.split(&id)?;
    let exclude: Vec<i32> = query
        .exclude
        .unwrap_or_default()
        .split(',')
        .filter_map(|v| v.trim().parse().ok())
        .collect();
    let length_m = split.track().map(|t| t.length_m).unwrap_or(0.0);
    let report = blocking("Shift analysis", move || {
        let traces: Vec<&LapTrace> = split.run.traces.iter().filter(|t| !exclude.contains(&t.lap_number)).collect();
        shift_report(&traces, split.run.shift_rpm, split.run.redline_rpm, length_m)
    })
    .await?;
    Ok(Json(report))
}
