//! Lists the .ibt files the UI can import.

use std::path::PathBuf;

use axum::{extract::Query, Json};
use serde::{Deserialize, Serialize};

use super::{blocking, modified_ms, ApiResult};

/// iRacing's telemetry folder. `PCC_TELEMETRY_DIR` wins if set; otherwise it's
/// `Documents\iRacing\telemetry`, checking OneDrive-redirected Documents first (OneDrive
/// sets the `OneDrive*` variables to its root, whatever the folder is called).
fn default_telemetry_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("PCC_TELEMETRY_DIR").map(PathBuf::from) {
        if dir.is_dir() {
            return Some(dir);
        }
        eprintln!("PCC_TELEMETRY_DIR is set but {} is not a folder; ignoring it.", dir.display());
    }

    let mut documents: Vec<PathBuf> = ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"]
        .iter()
        .filter_map(std::env::var_os)
        .map(|root| PathBuf::from(root).join("Documents"))
        .collect();
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")).map(PathBuf::from) {
        documents.push(home.join("OneDrive").join("Documents"));
        documents.push(home.join("Documents"));
    }
    documents
        .into_iter()
        .map(|docs| docs.join("iRacing").join("telemetry"))
        .find(|dir| dir.is_dir())
}

#[derive(Debug, Deserialize)]
pub(super) struct TelemetryFilesQuery {
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
pub(super) struct TelemetryFilesResponse {
    dir: Option<String>,
    files: Vec<TelemetryFile>,
}

pub(super) async fn list_telemetry_files(Query(query): Query<TelemetryFilesQuery>) -> ApiResult<TelemetryFilesResponse> {
    let response = blocking("Listing telemetry files", move || {
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
                if !path.extension()?.to_str()?.eq_ignore_ascii_case("ibt") {
                    return None;
                }
                let meta = entry.metadata().ok()?;
                Some(TelemetryFile {
                    name: path.file_name()?.to_string_lossy().to_string(),
                    path: path.display().to_string(),
                    size_bytes: meta.len(),
                    modified_ms: modified_ms(&meta).unwrap_or_default(),
                })
            })
            .collect();
        files.sort_by_key(|f| std::cmp::Reverse(f.modified_ms));
        files.truncate(200);
        TelemetryFilesResponse { dir: dir.map(|d| d.display().to_string()), files }
    })
    .await?;
    Ok(Json(response))
}
