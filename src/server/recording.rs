//! Recording a live (or replayed) session and turning it into a split when stopped.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::sync::mpsc::Receiver;
use std::time::Duration;

use anyhow::Result;
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde::{Deserialize, Serialize};

use super::splits::{finalize_split, NewRun, SplitDetailResponse};
use super::{blocking, epoch_ms_now, error_response, pick_model, ApiResult, AppState, Inner};
use crate::coach::radio_call::{lap_call, pit_debrief, CallKind, LapCall};
use crate::telemetry::{
    live::{replay_until_stopped, run_live_telemetry_until_stopped, RadioCue, SessionProbe},
    trace::{Capture, LapMetrics},
};

pub(super) struct RecordingState {
    started_at_ms: u128,
    model: String,
    /// Started by the auto-recorder, which also saves it once the driver leaves the car.
    auto: bool,
    stop_signal: Arc<AtomicBool>,
    live_laps: Arc<Mutex<Vec<LapMetrics>>>,
    radio_calls: Arc<Mutex<Vec<LapCall>>>,
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
    is_auto: bool,
    laps: Vec<LapMetrics>,
    radio_calls: Vec<LapCall>,
}

/// Starts capturing on a worker thread. The caller makes sure no recording is running.
fn begin(state: &Arc<AppState>, guard: &mut Inner, model: String, auto: bool) -> u128 {
    let started_at_ms = epoch_ms_now();
    let stop_signal = Arc::new(AtomicBool::new(false));
    let live_laps = Arc::new(Mutex::new(Vec::new()));
    let radio_calls = Arc::new(Mutex::new(Vec::new()));
    let (laps_done, finished_laps) = std::sync::mpsc::channel::<RadioCue>();
    spawn_radio(Arc::clone(state), Arc::clone(&radio_calls), finished_laps);
    let worker = {
        let stop_signal = Arc::clone(&stop_signal);
        let live_laps = Arc::clone(&live_laps);
        match guard.replay.clone() {
            Some((path, speed)) => {
                std::thread::spawn(move || replay_until_stopped(&path, speed, stop_signal, live_laps, Some(laps_done)))
            }
            None => std::thread::spawn(move || {
                run_live_telemetry_until_stopped(stop_signal, live_laps, auto, Some(laps_done))
            }),
        }
    };

    guard.recording = Some(RecordingState { started_at_ms, model, auto, stop_signal, live_laps, radio_calls, worker });
    started_at_ms
}

/// The pit engineer: after each finished lap, and on entering the pit lane, works out what to
/// say, lists it for the UI and (when enabled) says it out loud. Ends when the worker drops
/// its sender.
fn spawn_radio(state: Arc<AppState>, calls: Arc<Mutex<Vec<LapCall>>>, cues: Receiver<RadioCue>) {
    std::thread::Builder::new()
        .name("radio".into())
        .spawn(move || {
            while let Ok(first) = cues.recv() {
                // A backlog (slow speech) shouldn't queue up stale lap calls: only the newest
                // counts. A pit debrief is never dropped, and comes after the lap call.
                let (mut lap, mut pit) = (None, None);
                for cue in std::iter::once(first).chain(cues.try_iter()) {
                    match cue {
                        RadioCue::LapDone(capture) => lap = Some(capture),
                        RadioCue::PitEntry(capture) => pit = Some(capture),
                    }
                }
                for (capture, kind) in [(lap, CallKind::Lap), (pit, CallKind::Pit)] {
                    let Some(capture) = capture else { continue };
                    let (track, saved) = crate::telemetry::track::with_saved(capture.track.clone());
                    let run = crate::telemetry::trace::build_run(&capture.frames, &track);
                    let turns = match saved.map(|map| map.turns).filter(|turns| !turns.is_empty()) {
                        Some(turns) => turns,
                        None => run
                            .map_points
                            .as_ref()
                            .map(|points| crate::telemetry::trace::detect_turns(points, track.length_m))
                            .unwrap_or_default(),
                    };
                    let call = match kind {
                        CallKind::Lap => lap_call(&run, &turns, track.length_m),
                        // Straight back in from an out-lap: nothing to debrief.
                        CallKind::Pit if !run.laps.iter().any(|l| l.is_complete) => None,
                        CallKind::Pit => pit_debrief(&run, &turns, track.length_m),
                    };
                    let Some(call) = call else { continue };
                    {
                        let Ok(mut list) = calls.lock() else { continue };
                        if list.iter().any(|c| c.lap_number == call.lap_number && c.kind == call.kind) {
                            continue;
                        }
                        list.push(call.clone());
                    }
                    if state.live_radio.load(Ordering::Relaxed) && state.voice.is_available() {
                        if let Err(err) = super::voice::speak_on_radio(&state, &call.text) {
                            eprintln!("Radio call for lap {} failed: {err:#}", call.lap_number);
                        }
                    }
                }
            }
        })
        .map(|_| ())
        .unwrap_or_else(|err| eprintln!("Could not start the radio thread: {err}"));
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
    let started_at_ms = begin(&state, &mut guard, model.clone(), false);
    Ok(Json(StartRecordingResponse { started_at_ms, model }))
}

pub(super) async fn get_live_recording(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let guard = state.lock();
    let recording = guard.recording.as_ref();
    Json(LiveRecordingResponse {
        started_at_ms: recording.map(|r| r.started_at_ms),
        capture_ended: recording.is_some_and(|r| r.worker.is_finished()),
        is_replay: guard.replay.is_some(),
        is_auto: recording.is_some_and(|r| r.auto),
        laps: recording.map(|r| r.live_laps.lock().map(|laps| laps.clone()).unwrap_or_default()).unwrap_or_default(),
        radio_calls: recording.map(|r| r.radio_calls.lock().map(|calls| calls.clone()).unwrap_or_default()).unwrap_or_default(),
    })
}

pub(super) async fn stop_recording(State(state): State<Arc<AppState>>) -> ApiResult<SplitDetailResponse> {
    let recording = {
        let mut guard = state.lock();
        // Stopping by hand while still in the car shouldn't start a new recording straight
        // away: wait until the driver has been out of the car first.
        guard.auto_held = true;
        guard.recording.take()
    };
    let Some(recording) = recording else {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "No active recording to stop.",
        ));
    };
    finish(&state, recording).await
}

/// Stops the worker and turns what it captured into a split.
async fn finish(state: &Arc<AppState>, recording: RecordingState) -> ApiResult<SplitDetailResponse> {
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

    let live_calls = recording.radio_calls.lock().map(|calls| calls.clone()).unwrap_or_default();
    finalize_split(state, run, track, recording.model, live_calls).await
}

/// Starts a recording whenever the player gets in the car in iRacing, and saves it once
/// they've been out of the car for a few seconds, so nobody has to press Start or Stop.
pub(super) fn spawn_auto_recorder(state: Arc<AppState>, runtime: tokio::runtime::Handle) {
    std::thread::spawn(move || {
        let mut probe = SessionProbe::default();
        loop {
            std::thread::sleep(Duration::from_secs(1));

            let ended = {
                let mut guard = state.lock();
                let done = guard.recording.as_ref().is_some_and(|r| r.auto && r.worker.is_finished());
                if done { guard.recording.take() } else { None }
            };
            if let Some(recording) = ended {
                // A trip out of the pits and straight back in isn't worth a run.
                let timed = recording.live_laps.lock().map(|laps| laps.iter().any(|lap| lap.is_complete)).unwrap_or(false);
                if !timed {
                    let _ = recording.worker.join();
                } else if let Err((_, Json(body))) = runtime.block_on(finish(&state, recording)) {
                    eprintln!("Could not save the auto-recorded run: {}", body.error);
                }
                continue;
            }
            if state.lock().recording.is_some() {
                continue;
            }

            let driving = probe.driving();
            let mut guard = state.lock();
            if !driving {
                guard.auto_held = false;
            } else if !guard.auto_held && guard.recording.is_none() {
                let model = guard.default_model.clone();
                begin(&state, &mut guard, model, true);
            }
        }
    });
}
