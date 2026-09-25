use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use tower_http::services::{ServeDir, ServeFile};

use crate::{
    analysis::{add_corner_notes, summarize_session},
    handling::{corner_notes, peak_combined_g},
    coach::generate_feedback,
    ibt::read_ibt,
    io::{default_telemetry_dir, parse_csv_laps},
    live::{replay_until_stopped, run_live_telemetry_until_stopped, LiveCapture},
    trace::{build_run, Frame, LapTrace, TrackInfo, Turn},
    track::{self, TrackMap},
    types::{LapMetrics, SessionSummary},
};

pub fn run_ui_server(port: u16, default_model: String, replay: Option<(PathBuf, f64)>) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    runtime.block_on(async move {
        let state = Arc::new(AppState {
            inner: Mutex::new(AppStateInner {
                default_model,
                replay,
                recording: None,
                splits: Vec::new(),
                next_split_id: 1,
            }),
        });

        let api = Router::new()
            .route("/status", get(get_status))
            .route("/recording/start", post(start_recording))
            .route("/recording/stop", post(stop_recording))
            .route("/recording/live", get(get_live_recording))
            .route("/telemetry-files", get(list_telemetry_files))
            .route("/splits", get(list_splits))
            .route("/splits/import", post(import_split))
            .route("/splits/:id", get(get_split).delete(delete_split))
            .route("/splits/:id/track", get(get_split_track))
            .route("/splits/:id/track/turns", post(update_track_turns))
            .route("/splits/:id/laps", get(list_split_laps))
            .route("/splits/:id/laps/:lap_number", get(get_split_lap))
            .route("/splits/:id/laps/:lap_number/trace", get(get_lap_trace))
            .route("/splits/:id/compare", get(compare_split_laps))
            .fallback(|| async {
                error_response(StatusCode::NOT_FOUND, "API route not found.").into_response()
            });

        let app = Router::new()
            .nest("/api", api)
            .with_state(state)
            .nest_service(
                "/",
                ServeDir::new("web")
                    .append_index_html_on_directories(true)
                    .fallback(ServeFile::new("web/index.html")),
            );

        let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
        println!("Pit Crew Coach UI running at http://127.0.0.1:{port}");
        println!("Open that URL in your browser.");
        axum::serve(listener, app).await?;
        Ok(())
    })
}

struct AppState {
    inner: Mutex<AppStateInner>,
}

struct AppStateInner {
    default_model: String,
    /// When set, recordings replay this .ibt (at the given speed) instead of reading iRacing.
    replay: Option<(PathBuf, f64)>,
    recording: Option<RecordingState>,
    splits: Vec<PracticeSplit>,
    next_split_id: u64,
}

struct RecordingState {
    started_at_ms: u128,
    model: String,
    stop_signal: Arc<AtomicBool>,
    live_laps: Arc<Mutex<Vec<LapMetrics>>>,
    worker: std::thread::JoinHandle<Result<LiveCapture>>,
}

struct PracticeSplit {
    id: String,
    source: String,
    track_label: String,
    car: String,
    started_at_ms: u128,
    ended_at_ms: u128,
    model: String,
    laps: Vec<LapMetrics>,
    traces: Vec<LapTrace>,
    track: Option<TrackMap>,
    summary: SessionSummary,
    feedback: String,
}

/// A run's data before summary/feedback are generated.
struct NewRun {
    source: String,
    track_label: String,
    car: String,
    started_at_ms: u128,
    laps: Vec<LapMetrics>,
    traces: Vec<LapTrace>,
    track: Option<TrackMap>,
}

impl NewRun {
    /// Split frames into laps and traces, and attach the (saved or newly built) track map.
    fn from_frames(source: String, started_at_ms: u128, track_info: TrackInfo, frames: &[Frame]) -> Self {
        let track_info = track::with_saved_sectors(track_info);
        let run = build_run(frames, &track_info);
        let track = track::resolve(&track_info, &run);
        let track_label = match (track_info.display_name.as_str(), track_info.config_name.as_str()) {
            ("", _) => String::new(),
            (name, "") => name.to_string(),
            (name, config) => format!("{name} · {config}"),
        };
        NewRun {
            source,
            track_label,
            car: track_info.car,
            started_at_ms,
            laps: run.laps,
            traces: run.traces,
            track,
        }
    }
}

impl PracticeSplit {
    fn best_lap_speed(&self) -> Option<f64> {
        self.laps
            .iter()
            .find(|lap| lap.lap_number == self.summary.fastest_lap)
            .and_then(|lap| lap.avg_speed_kph)
    }

    fn lap(&self, lap_number: i32) -> Option<&LapMetrics> {
        self.laps.iter().find(|lap| lap.lap_number == lap_number)
    }

    fn insight(&self, lap: &LapMetrics) -> LapInsight {
        build_lap_insight(lap, self.summary.fastest_lap_time_s, self.best_lap_speed())
    }
}

#[derive(Debug, Serialize)]
struct ApiError {
    error: String,
}

#[derive(Debug, Serialize)]
struct StatusResponse {
    is_recording: bool,
    split_count: usize,
    default_model: String,
    replay_file: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StartRecordingRequest {
    model: Option<String>,
}

#[derive(Debug, Serialize)]
struct StartRecordingResponse {
    started_at_ms: u128,
    model: String,
}

#[derive(Debug, Serialize)]
struct LiveRecordingResponse {
    is_recording: bool,
    started_at_ms: Option<u128>,
    now_ms: u128,
    /// True when the capture thread exited on its own (iRacing not running, or replay finished).
    capture_ended: bool,
    is_replay: bool,
    laps: Vec<LapMetrics>,
}

#[derive(Debug, Deserialize)]
struct ImportRequest {
    path: String,
    model: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TelemetryFilesQuery {
    dir: Option<String>,
}

#[derive(Debug, Serialize)]
struct TelemetryFile {
    path: String,
    name: String,
    size_bytes: u64,
    modified_ms: u128,
}

#[derive(Debug, Serialize)]
struct TelemetryFilesResponse {
    dir: Option<String>,
    files: Vec<TelemetryFile>,
}

#[derive(Debug, Serialize)]
struct SplitOverviewResponse {
    id: String,
    source: String,
    track_label: String,
    car: String,
    started_at_ms: u128,
    ended_at_ms: u128,
    model: String,
    lap_count: usize,
    complete_lap_count: usize,
    fastest_lap: i32,
    fastest_lap_time_s: f64,
    average_lap_time_s: f64,
}

#[derive(Debug, Serialize)]
struct SplitDetailResponse {
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
    /// Session peak combined g (98th percentile over all traces), the 100% mark for grip use.
    peak_g: Option<f64>,
    summary: SessionSummary,
    suggestions: Vec<String>,
    feedback: String,
}

#[derive(Debug, Serialize)]
struct LapInsight {
    lap_number: i32,
    lap_time_s: f64,
    is_complete: bool,
    delta_to_best_s: f64,
    sectors: Vec<f64>,
    avg_speed_kph: Option<f64>,
    tyre_temp_avg_c: Option<f64>,
    tyre_temp_delta_c: Option<f64>,
    went_well: Vec<String>,
    went_bad: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct CompareQuery {
    lap_a: i32,
    lap_b: i32,
}

#[derive(Debug, Serialize)]
struct LapComparisonResponse {
    lap_a: LapInsight,
    lap_b: LapInsight,
    summary: String,
}

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ApiError>)>;

fn error_response(status: StatusCode, message: impl Into<String>) -> (StatusCode, Json<ApiError>) {
    (status, Json(ApiError { error: message.into() }))
}

fn epoch_ms_now() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default()
}

fn split_overview(split: &PracticeSplit) -> SplitOverviewResponse {
    SplitOverviewResponse {
        id: split.id.clone(),
        source: split.source.clone(),
        track_label: split.track_label.clone(),
        car: split.car.clone(),
        started_at_ms: split.started_at_ms,
        ended_at_ms: split.ended_at_ms,
        model: split.model.clone(),
        lap_count: split.laps.len(),
        complete_lap_count: split.laps.iter().filter(|lap| lap.is_complete).count(),
        fastest_lap: split.summary.fastest_lap,
        fastest_lap_time_s: split.summary.fastest_lap_time_s,
        average_lap_time_s: split.summary.average_lap_time_s,
    }
}

fn split_detail(split: &PracticeSplit) -> SplitDetailResponse {
    SplitDetailResponse {
        id: split.id.clone(),
        source: split.source.clone(),
        track_label: split.track_label.clone(),
        car: split.car.clone(),
        started_at_ms: split.started_at_ms,
        ended_at_ms: split.ended_at_ms,
        model: split.model.clone(),
        lap_count: split.laps.len(),
        has_track: split.track.is_some(),
        traced_laps: split.traces.iter().map(|trace| trace.lap_number).collect(),
        peak_g: peak_combined_g(&split.traces),
        summary: split.summary.clone(),
        suggestions: split.summary.suggestions.clone(),
        feedback: split.feedback.clone(),
    }
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

    if let Some(temp) = lap.tyre_temp_avg_c {
        if temp <= 95.0 {
            went_well.push("Tyre average temperature stayed in a controlled window.".to_string());
        } else if temp >= 100.0 {
            went_bad.push("Tyre average temperature ran hot; grip likely dropped over the lap.".to_string());
        }
    }

    if let Some(temp_delta) = lap.tyre_temp_delta_c {
        if temp_delta <= 6.0 {
            went_well.push("Tyre temperature spread stayed stable.".to_string());
        } else if temp_delta >= 10.0 {
            went_bad.push("Tyre temperature spread was high; balance or traction was inconsistent.".to_string());
        }
    }

    if went_well.is_empty() {
        went_well.push("No standout strength flagged from this telemetry slice.".to_string());
    }
    if went_bad.is_empty() {
        went_bad.push("No major weakness flagged versus your best lap.".to_string());
    }

    LapInsight {
        lap_number: lap.lap_number,
        lap_time_s: lap.lap_time_s,
        is_complete: lap.is_complete,
        delta_to_best_s,
        sectors: lap.sectors.clone(),
        avg_speed_kph: lap.avg_speed_kph,
        tyre_temp_avg_c: lap.tyre_temp_avg_c,
        tyre_temp_delta_c: lap.tyre_temp_delta_c,
        went_well,
        went_bad,
    }
}

/// Summarizes the laps, asks the coach model for feedback (off the async runtime, since
/// Ollama can take a while) and stores the result as a new split.
async fn finalize_split(state: &Arc<AppState>, run: NewRun, model: String) -> ApiResult<SplitDetailResponse> {
    if run.laps.is_empty() {
        return Err(error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "No usable lap data captured. Try recording while driving and for longer.",
        ));
    }

    let (run, summary, feedback, model) = tokio::task::spawn_blocking(move || {
        let mut summary = summarize_session(&run.laps);
        if let Some(track) = &run.track {
            add_corner_notes(&mut summary, corner_notes(&run.traces, &track.turns, track.length_m));
        }
        let feedback = generate_feedback(&summary, &model);
        (run, summary, feedback, model)
    })
    .await
    .map_err(|err| error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("Analysis failed: {err}")))?;

    let mut guard = state.inner.lock().expect("state lock poisoned");
    let split_id = format!("split-{}", guard.next_split_id);
    guard.next_split_id += 1;

    let split = PracticeSplit {
        id: split_id,
        source: run.source,
        track_label: run.track_label,
        car: run.car,
        started_at_ms: run.started_at_ms,
        ended_at_ms: epoch_ms_now(),
        model,
        laps: run.laps,
        traces: run.traces,
        track: run.track,
        summary,
        feedback,
    };

    let response = split_detail(&split);
    guard.splits.push(split);
    Ok(Json(response))
}

fn find_split<'a>(
    splits: &'a [PracticeSplit],
    id: &str,
) -> Result<&'a PracticeSplit, (StatusCode, Json<ApiError>)> {
    splits
        .iter()
        .find(|split| split.id == id)
        .ok_or_else(|| error_response(StatusCode::NOT_FOUND, "Split not found."))
}

async fn get_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let guard = state.inner.lock().expect("state lock poisoned");
    Json(StatusResponse {
        is_recording: guard.recording.is_some(),
        split_count: guard.splits.len(),
        default_model: guard.default_model.clone(),
        replay_file: guard.replay.as_ref().map(|(path, _)| path.display().to_string()),
    })
}

async fn start_recording(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<StartRecordingRequest>,
) -> ApiResult<StartRecordingResponse> {
    let mut guard = state.inner.lock().expect("state lock poisoned");
    if guard.recording.is_some() {
        return Err(error_response(
            StatusCode::CONFLICT,
            "A recording is already in progress.",
        ));
    }

    let model = payload
        .model
        .filter(|model| !model.trim().is_empty())
        .unwrap_or_else(|| guard.default_model.clone());
    let started_at_ms = epoch_ms_now();
    let stop_signal = Arc::new(AtomicBool::new(false));
    let live_laps = Arc::new(Mutex::new(Vec::new()));
    let worker = {
        let stop_signal = Arc::clone(&stop_signal);
        let live_laps = Arc::clone(&live_laps);
        match guard.replay.clone() {
            Some((path, speed)) => std::thread::spawn(move || replay_until_stopped(&path, speed, stop_signal, live_laps)),
            None => std::thread::spawn(move || run_live_telemetry_until_stopped(stop_signal, live_laps)),
        }
    };

    guard.recording = Some(RecordingState {
        started_at_ms,
        model: model.clone(),
        stop_signal,
        live_laps,
        worker,
    });

    Ok(Json(StartRecordingResponse { started_at_ms, model }))
}

async fn get_live_recording(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let guard = state.inner.lock().expect("state lock poisoned");
    let (started_at_ms, laps) = match &guard.recording {
        Some(recording) => (
            Some(recording.started_at_ms),
            recording.live_laps.lock().map(|laps| laps.clone()).unwrap_or_default(),
        ),
        None => (None, Vec::new()),
    };
    Json(LiveRecordingResponse {
        is_recording: guard.recording.is_some(),
        started_at_ms,
        now_ms: epoch_ms_now(),
        capture_ended: guard
            .recording
            .as_ref()
            .map(|recording| recording.worker.is_finished())
            .unwrap_or(false),
        is_replay: guard.replay.is_some(),
        laps,
    })
}

async fn stop_recording(State(state): State<Arc<AppState>>) -> ApiResult<SplitDetailResponse> {
    let recording = {
        let mut guard = state.inner.lock().expect("state lock poisoned");
        guard.recording.take()
    };

    let Some(recording) = recording else {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "No active recording to stop.",
        ));
    };

    recording.stop_signal.store(true, Ordering::Relaxed);
    let join_result = tokio::task::spawn_blocking(move || recording.worker.join())
        .await
        .map_err(|err| error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("Join failed: {err}")))?;

    let capture = match join_result {
        Ok(Ok(capture)) => capture,
        Ok(Err(err)) => return Err(error_response(StatusCode::BAD_REQUEST, err.to_string())),
        Err(_) => {
            return Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Recording thread panicked.",
            ))
        }
    };

    let started_at_ms = recording.started_at_ms;
    let run = tokio::task::spawn_blocking(move || {
        NewRun::from_frames("Live".to_string(), started_at_ms, capture.track, &capture.frames)
    })
    .await
    .map_err(|err| error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("Processing failed: {err}")))?;

    finalize_split(&state, run, recording.model).await
}

async fn list_telemetry_files(Query(query): Query<TelemetryFilesQuery>) -> impl IntoResponse {
    let dir = query
        .dir
        .map(|d| PathBuf::from(d.trim().trim_matches('"')))
        .filter(|d| !d.as_os_str().is_empty())
        .or_else(default_telemetry_dir)
        // No iRacing on this machine (e.g. macOS): offer the committed test fixtures.
        .or_else(|| Some(PathBuf::from("tests/fixtures")).filter(|dir| dir.is_dir()));
    let mut files: Vec<TelemetryFile> = dir
        .as_ref()
        .and_then(|dir| std::fs::read_dir(dir).ok())
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let path = entry.path();
            let ext = path.extension()?.to_str()?.to_ascii_lowercase();
            if ext != "ibt" && ext != "csv" {
                return None;
            }
            let meta = entry.metadata().ok()?;
            Some(TelemetryFile {
                name: path.file_name()?.to_string_lossy().to_string(),
                path: path.display().to_string(),
                size_bytes: meta.len(),
                modified_ms: meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_millis())
                    .unwrap_or_default(),
            })
        })
        .collect();
    files.sort_by(|a, b| b.modified_ms.cmp(&a.modified_ms));
    files.truncate(200);
    Json(TelemetryFilesResponse {
        dir: dir.map(|d| d.display().to_string()),
        files,
    })
}

async fn import_split(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<ImportRequest>,
) -> ApiResult<SplitDetailResponse> {
    let path = PathBuf::from(payload.path.trim().trim_matches('"'));
    if path.as_os_str().is_empty() {
        return Err(error_response(StatusCode::BAD_REQUEST, "Enter a .csv/.ibt file or folder path."));
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
            "That's a folder — pick a single .ibt or .csv file from it.",
        ));
    }

    let source = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| path.display().to_string());
    let started_at_ms = std::fs::metadata(&path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis())
        .unwrap_or_else(epoch_ms_now);
    let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or("").to_ascii_lowercase();

    let run = tokio::task::spawn_blocking(move || -> Result<NewRun> {
        match extension.as_str() {
            "ibt" => {
                let data = read_ibt(&path)?;
                Ok(NewRun::from_frames(source, started_at_ms, data.track, &data.frames))
            }
            "csv" => Ok(NewRun {
                source,
                track_label: String::new(),
                car: String::new(),
                started_at_ms,
                laps: parse_csv_laps(&path)?,
                traces: Vec::new(),
                track: None,
            }),
            _ => anyhow::bail!("Unsupported file type — choose an .ibt or .csv file."),
        }
    })
    .await
    .map_err(|err| error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("Import failed: {err}")))?
    .map_err(|err| error_response(StatusCode::UNPROCESSABLE_ENTITY, err.to_string()))?;

    let model = {
        let guard = state.inner.lock().expect("state lock poisoned");
        payload
            .model
            .filter(|model| !model.trim().is_empty())
            .unwrap_or_else(|| guard.default_model.clone())
    };
    finalize_split(&state, run, model).await
}

async fn get_split_track(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> ApiResult<TrackMap> {
    let guard = state.inner.lock().expect("state lock poisoned");
    let split = find_split(&guard.splits, &id)?;
    split
        .track
        .clone()
        .map(Json)
        .ok_or_else(|| error_response(StatusCode::NOT_FOUND, "No track map for this run."))
}

#[derive(Debug, Deserialize)]
struct UpdateTurnsRequest {
    turns: Vec<Turn>,
}

async fn update_track_turns(
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

    let mut guard = state.inner.lock().expect("state lock poisoned");
    let track_name = {
        let split = guard
            .splits
            .iter_mut()
            .find(|split| split.id == id)
            .ok_or_else(|| error_response(StatusCode::NOT_FOUND, "Split not found."))?;
        let map = split
            .track
            .as_mut()
            .ok_or_else(|| error_response(StatusCode::NOT_FOUND, "No track map for this run."))?;
        track::update_turns(map, turns.clone());
        map.track_name.clone()
    };
    // Other runs on the same track share the saved labels.
    for split in guard.splits.iter_mut() {
        if let Some(map) = split.track.as_mut().filter(|map| map.track_name == track_name) {
            map.turns = turns.clone();
        }
    }
    let split = find_split(&guard.splits, &id)?;
    Ok(Json(split.track.clone().expect("checked above")))
}

async fn get_lap_trace(
    State(state): State<Arc<AppState>>,
    Path((id, lap_number)): Path<(String, i32)>,
) -> ApiResult<LapTrace> {
    let guard = state.inner.lock().expect("state lock poisoned");
    let split = find_split(&guard.splits, &id)?;
    split
        .traces
        .iter()
        .find(|trace| trace.lap_number == lap_number)
        .cloned()
        .map(Json)
        .ok_or_else(|| error_response(StatusCode::NOT_FOUND, "No telemetry trace for this lap."))
}

async fn list_splits(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let guard = state.inner.lock().expect("state lock poisoned");
    let list: Vec<SplitOverviewResponse> = guard.splits.iter().map(split_overview).collect();
    Json(list)
}

async fn get_split(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> ApiResult<SplitDetailResponse> {
    let guard = state.inner.lock().expect("state lock poisoned");
    Ok(Json(split_detail(find_split(&guard.splits, &id)?)))
}

async fn delete_split(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ApiError>)> {
    let mut guard = state.inner.lock().expect("state lock poisoned");
    let before = guard.splits.len();
    guard.splits.retain(|split| split.id != id);
    if guard.splits.len() == before {
        return Err(error_response(StatusCode::NOT_FOUND, "Split not found."));
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn list_split_laps(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> ApiResult<Vec<LapInsight>> {
    let guard = state.inner.lock().expect("state lock poisoned");
    let split = find_split(&guard.splits, &id)?;
    Ok(Json(split.laps.iter().map(|lap| split.insight(lap)).collect()))
}

async fn get_split_lap(
    State(state): State<Arc<AppState>>,
    Path((id, lap_number)): Path<(String, i32)>,
) -> ApiResult<LapInsight> {
    let guard = state.inner.lock().expect("state lock poisoned");
    let split = find_split(&guard.splits, &id)?;
    let lap = split
        .lap(lap_number)
        .ok_or_else(|| error_response(StatusCode::NOT_FOUND, "Lap not found in this split."))?;
    Ok(Json(split.insight(lap)))
}

async fn compare_split_laps(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(query): Query<CompareQuery>,
) -> ApiResult<LapComparisonResponse> {
    let guard = state.inner.lock().expect("state lock poisoned");
    let split = find_split(&guard.splits, &id)?;

    let lap_a = split
        .lap(query.lap_a)
        .ok_or_else(|| error_response(StatusCode::NOT_FOUND, "Lap A not found in this split."))?;
    let lap_b = split
        .lap(query.lap_b)
        .ok_or_else(|| error_response(StatusCode::NOT_FOUND, "Lap B not found in this split."))?;

    let summary = if lap_a.lap_time_s < lap_b.lap_time_s {
        format!(
            "Lap {} was {:.3}s quicker than lap {}.",
            lap_a.lap_number,
            lap_b.lap_time_s - lap_a.lap_time_s,
            lap_b.lap_number
        )
    } else if lap_b.lap_time_s < lap_a.lap_time_s {
        format!(
            "Lap {} was {:.3}s quicker than lap {}.",
            lap_b.lap_number,
            lap_a.lap_time_s - lap_b.lap_time_s,
            lap_a.lap_number
        )
    } else {
        format!(
            "Lap {} and lap {} were equal on lap time.",
            lap_a.lap_number, lap_b.lap_number
        )
    };

    Ok(Json(LapComparisonResponse {
        lap_a: split.insight(lap_a),
        lap_b: split.insight(lap_b),
        summary,
    }))
}