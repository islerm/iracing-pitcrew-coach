//! Voice endpoints: engine status, the spoken-form split of a message, and speech synthesis.

use std::sync::Arc;

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde::{Deserialize, Serialize};

use super::{blocking, error_response, ApiError, ApiResult, AppState};
use crate::voice::{radio::Transmission, VoiceStatus};

const MAX_SPEAK_CHARS: usize = 4000;

/// Text for the voice endpoints, trimmed; empty or overlong text is refused.
fn validate_text(text: &str) -> Result<&str, ApiError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(error_response(StatusCode::BAD_REQUEST, "Text is empty."));
    }
    if text.chars().count() > MAX_SPEAK_CHARS {
        return Err(error_response(StatusCode::BAD_REQUEST, format!("Text is too long (max {MAX_SPEAK_CHARS} characters).")));
    }
    Ok(text)
}

/// One part from /speak/plan, already in spoken form.
#[derive(Debug, Deserialize)]
pub(super) struct SpeakRequest {
    text: String,
    #[serde(default = "default_true")]
    radio: bool,
    /// Where the part sits in the message, so consecutive parts play as one radio transmission.
    #[serde(default = "default_true")]
    first: bool,
    #[serde(default = "default_true")]
    last: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct SpeakPlanRequest {
    text: String,
}

#[derive(Debug, Serialize)]
pub(super) struct SpeakPlanResponse {
    parts: Vec<String>,
}

fn default_true() -> bool {
    true
}

/// Splits a message into sentence-sized parts the UI fetches and plays one after another.
pub(super) async fn speak_plan(State(state): State<Arc<AppState>>, Json(payload): Json<SpeakPlanRequest>) -> ApiResult<SpeakPlanResponse> {
    let text = validate_text(&payload.text)?;
    Ok(Json(SpeakPlanResponse { parts: state.voice.plan(text) }))
}

pub(super) async fn get_voice(State(state): State<Arc<AppState>>) -> ApiResult<VoiceStatus> {
    let voice = state.voice.clone();
    Ok(Json(blocking("Voice status check", move || voice.status()).await?))
}

pub(super) async fn speak(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<SpeakRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let text = validate_text(&payload.text)?.to_string();
    let voice = state.voice.clone();
    let radio = payload.radio;
    let part = Transmission { first: payload.first, last: payload.last };
    let result = blocking("Speech", move || {
        if !voice.is_available() {
            return Err(error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                voice.status().hint.unwrap_or_else(|| "No voice engine available.".into()),
            ));
        }
        voice
            .speak_part(&text, radio, part)
            .map_err(|err| error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("Speech failed: {err:#}")))
    })
    .await?;
    result.map(|wav| ([(axum::http::header::CONTENT_TYPE, "audio/wav")], wav))
}

/// Speaks `text` as a radio call on this PC at the current radio volume. Blocks until it has played.
pub(super) fn speak_on_radio(state: &AppState, text: &str) -> anyhow::Result<()> {
    let mut wav = crate::voice::radio::read_wav(&state.voice.speak_message(text, true)?)?;
    crate::voice::radio::apply_volume(&mut wav, state.radio_volume());
    crate::voice::playback::play_wav(&crate::voice::radio::write_wav(&wav))
}

#[derive(Debug, Deserialize)]
pub(super) struct SetVolumeRequest {
    volume: f32,
}

/// Sets how loud the live radio calls are, and remembers it.
pub(super) async fn set_radio_volume(State(state): State<Arc<AppState>>, Json(payload): Json<SetVolumeRequest>) -> impl IntoResponse {
    let volume = super::settings::clamp_volume(payload.volume);
    state.set_radio_volume(volume);
    if let Err(err) = (super::settings::Settings { radio_volume: volume }).save() {
        eprintln!("Could not save settings: {err}");
    }
    StatusCode::NO_CONTENT
}

const TEST_CALL: &str = "Radio check. 1:22.215, 0.13s quicker. Turn 5 is costing you 0.19s.";

/// Plays a sample radio call at the current volume.
pub(super) async fn test_radio(State(state): State<Arc<AppState>>) -> Result<StatusCode, ApiError> {
    let result = blocking("Radio test", move || {
        if !state.voice.is_available() {
            return Err(error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                state.voice.status().hint.unwrap_or_else(|| "No voice engine available.".into()),
            ));
        }
        speak_on_radio(&state, TEST_CALL)
            .map_err(|err| error_response(StatusCode::INTERNAL_SERVER_ERROR, format!("Radio test failed: {err:#}")))
    })
    .await?;
    result.map(|()| StatusCode::NO_CONTENT)
}
