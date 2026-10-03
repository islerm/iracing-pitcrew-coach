//! Recording a live (or replayed) session and turning it into a split when stopped.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

use anyhow::Result;
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde::{Deserialize, Serialize};

use super::splits::{finalize_split, NewRun, SplitDetailResponse};
use super::{blocking, epoch_ms_now, error_response, pick_model, ApiResult, AppState};
use crate::telemetry::{
    live::{replay_until_stopped, run_live_telemetry_until_stopped},
    trace::{Capture, LapMetrics},
};

pub(super) struct RecordingState {
    started_at_ms: u128,
    model: String,
    stop_signal: Arc<AtomicBool>,
    live_laps: Arc<Mutex<Vec<LapMetrics>>>,
    worker: std::thread::JoinHandle<Result<Capture>>,
}

#[derive(Debug, Deserialize)]
pub(super) struct StartRecordingRequest {
    model: Option<String>,
}

#[derive(Debug, Serialize)]
pub(super) struct StartRecordingResponse {
    started_at_ms: u128,
    model: String,
}

#[derive(Debug, Serialize)]
pub(super) struct LiveRecordingResponse {
    started_at_ms: Option<u128>,
    /// True when the capture thread exited on its own (iRacing not running, or replay finished).
    capture_ended: bool,
    is_replay: bool,
    laps: Vec<LapMetrics>,
}

pub(super) async fn start_recording(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<StartRecordingRequest>,
) -> ApiResult<StartRecordingResponse> {
    let mut guard = state.lock();
    if guard.recording.is_some() {
        return Err(error_response(
            StatusCode::CONFLICT,
            "A recording is already in progress.",
        ));
    }

    let model = pick_model(payload.model, &guard.default_model);
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

pub(super) async fn get_live_recording(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let guard = state.lock();
    let recording = guard.recording.as_ref();
    Json(LiveRecordingResponse {
        started_at_ms: recording.map(|r| r.started_at_ms),
        capture_ended: recording.is_some_and(|r| r.worker.is_finished()),
        is_replay: guard.replay.is_some(),
        laps: recording.map(|r| r.live_laps.lock().map(|laps| laps.clone()).unwrap_or_default()).unwrap_or_default(),
    })
}

pub(super) async fn stop_recording(State(state): State<Arc<AppState>>) -> ApiResult<SplitDetailResponse> {
    let recording = state.lock().recording.take();
    let Some(recording) = recording else {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "No active recording to stop.",
        ));
    };

    recording.stop_signal.store(true, Ordering::Relaxed);
    let join_result = blocking("Join", move || recording.worker.join()).await?;
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
    let (run, track) = blocking("Processing", move || {
        NewRun::from_frames("Live".to_string(), started_at_ms, capture.track, &capture.frames)
    })
    .await?;

    finalize_split(&state, run, track, recording.model).await
}
