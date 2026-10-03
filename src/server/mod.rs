//! The local web server: the API behind the coaching UI, and the static UI files.

mod files;
mod recording;
mod splits;
mod voice;

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::Serialize;
use tower_http::services::{ServeDir, ServeFile};

use crate::coach::{installed_models, InstalledModel, ModelError};
use crate::voice::{Tts, VoiceEngine};

use recording::RecordingState;
use splits::PracticeSplit;

pub fn run_ui_server(port: u16, default_model: String, replay: Option<(PathBuf, f64)>, tts: Tts, voice: Option<String>) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    runtime.block_on(async move {
        let voice = Arc::new(VoiceEngine::new(tts, voice.as_deref()));
        let warming = voice.clone();
        tokio::task::spawn_blocking(move || warming.warm_up());
        let state = Arc::new(AppState {
            voice,
            inner: Mutex::new(Inner {
                default_model,
                replay,
                recording: None,
                splits: Vec::new(),
                next_split_id: 1,
            }),
        });

        let api = Router::new()
            .route("/status", get(get_status))
            .route("/models", get(list_models))
            .route("/recording/start", post(recording::start_recording))
            .route("/recording/stop", post(recording::stop_recording))
            .route("/recording/live", get(recording::get_live_recording))
            .route("/telemetry-files", get(files::list_telemetry_files))
            .route("/splits", get(splits::list_splits))
            .route("/splits/import", post(splits::import_split))
            .route("/splits/:id", get(splits::get_split).delete(splits::delete_split))
            .route("/splits/:id/track", get(splits::get_split_track))
            .route("/splits/:id/track/turns", post(splits::update_track_turns))
            .route("/splits/:id/laps", get(splits::list_split_laps))
            .route("/splits/:id/laps/:lap_number/trace", get(splits::get_lap_trace))
            .route("/splits/:id/corners/:turn", post(splits::analyze_corner))
            .route("/splits/:id/shifts", get(splits::get_shifts))
            .route("/voice", get(voice::get_voice))
            .route("/speak", post(voice::speak))
            .route("/speak/plan", post(voice::speak_plan))
            .fallback(|| async {
                error_response(StatusCode::NOT_FOUND, "API route not found.").into_response()
            });

        let app = Router::new()
            .nest("/api", api)
            .with_state(state)
            // A fallback rather than a nest at "/", so unknown /api paths reach the API's own
            // JSON 404 instead of being answered with index.html.
            .fallback_service(
                ServeDir::new("web")
                    .append_index_html_on_directories(true)
                    .fallback(ServeFile::new("web/index.html")),
            )
            // Have browsers check for a newer app.js/styles.css on every load, so an update
            // shows up on a normal refresh instead of the cached copy.
            .layer(axum::middleware::map_response(|mut response: axum::response::Response| async move {
                response
                    .headers_mut()
                    .insert(axum::http::header::CACHE_CONTROL, axum::http::HeaderValue::from_static("no-cache"));
                response
            }));

        let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await?;
        println!("Pit Crew Coach UI running at http://127.0.0.1:{port}");
        println!("Open that URL in your browser.");
        axum::serve(listener, app).await?;
        Ok(())
    })
}

struct AppState {
    voice: Arc<VoiceEngine>,
    inner: Mutex<Inner>,
}

impl AppState {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().expect("state lock poisoned")
    }

    fn split(&self, id: &str) -> Result<Arc<PracticeSplit>, ApiError> {
        self.lock()
            .splits
            .iter()
            .find(|split| split.id == id)
            .cloned()
            .ok_or_else(|| error_response(StatusCode::NOT_FOUND, "Split not found."))
    }
}

struct Inner {
    default_model: String,
    /// When set, recordings replay this .ibt (at the given speed) instead of reading iRacing.
    replay: Option<(PathBuf, f64)>,
    recording: Option<RecordingState>,
    /// Shared so a request can work on a split without holding the lock.
    splits: Vec<Arc<PracticeSplit>>,
    next_split_id: u64,
}

const NO_TRACK: &str = "No track map for this run.";

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: String,
}

type ApiError = (StatusCode, Json<ErrorBody>);
type ApiResult<T> = Result<Json<T>, ApiError>;

fn error_response(status: StatusCode, message: impl Into<String>) -> ApiError {
    (status, Json(ErrorBody { error: message.into() }))
}

/// Runs CPU-heavy or blocking work off the async runtime.
async fn blocking<T: Send + 'static>(label: &str, work: impl FnOnce() -> T + Send + 'static) -> Result<T, ApiError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|err| error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("{label} failed: {err}")))
}

/// The model the caller asked for, or `fallback` when none (or a blank one) was given.
fn pick_model(requested: Option<String>, fallback: &str) -> String {
    requested.filter(|model| !model.trim().is_empty()).unwrap_or_else(|| fallback.to_string())
}

fn epoch_ms_now() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default()
}

fn modified_ms(meta: &std::fs::Metadata) -> Option<u128> {
    meta.modified().ok()?.duration_since(UNIX_EPOCH).ok().map(|d| d.as_millis())
}

#[derive(Debug, Serialize)]
struct StatusResponse {
    is_recording: bool,
    default_model: String,
    replay_file: Option<String>,
}

async fn get_status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let guard = state.lock();
    Json(StatusResponse {
        is_recording: guard.recording.is_some(),
        default_model: guard.default_model.clone(),
        replay_file: guard.replay.as_ref().map(|(path, _)| path.display().to_string()),
    })
}

#[derive(Debug, Serialize)]
struct ModelsResponse {
    /// False when the `ollama` command can't be found.
    ollama: bool,
    installed: Vec<InstalledModel>,
    error: Option<String>,
}

async fn list_models() -> impl IntoResponse {
    let listed = tokio::task::spawn_blocking(installed_models).await;
    Json(match listed {
        Ok(Ok(installed)) => ModelsResponse { ollama: true, installed, error: None },
        Ok(Err(ModelError::NotInstalled)) => ModelsResponse { ollama: false, installed: Vec::new(), error: None },
        Ok(Err(err)) => ModelsResponse { ollama: true, installed: Vec::new(), error: Some(err.to_string()) },
        Err(err) => ModelsResponse { ollama: true, installed: Vec::new(), error: Some(err.to_string()) },
    })
}
