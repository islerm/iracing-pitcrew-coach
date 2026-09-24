use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use csv::ReaderBuilder;

use crate::ibt::read_ibt;
use crate::trace::build_run;
use crate::types::{LapMetrics, LapRecord};

pub fn normalize_path(input: &Path) -> PathBuf {
    input.to_path_buf()
}

pub fn parse_ibt_laps(path: &Path) -> Result<Vec<LapMetrics>> {
    let data = read_ibt(path)?;
    let run = build_run(&data.frames, &data.track);
    if run.laps.is_empty() {
        anyhow::bail!("no valid lap data found in {}", path.display());
    }
    Ok(run.laps)
}

pub fn parse_csv_laps(path: &Path) -> Result<Vec<LapMetrics>> {
    let mut reader = ReaderBuilder::new().from_path(path)
        .with_context(|| format!("failed to open telemetry CSV: {}", path.display()))?;

    let mut laps = Vec::new();
    for record in reader.deserialize::<LapRecord>() {
        let row = record.with_context(|| format!("invalid row in {}", path.display()))?;

        let lap_number = row.lap_number.unwrap_or_default();
        let lap_time_s = row.lap_time_s.or(row.lap_time_alt).unwrap_or(0.0);
        if lap_number <= 0 || lap_time_s <= 0.0 {
            continue;
        }

        let sector_1 = row.sector_1_s.or(row.sector_1_alt);
        let sector_2 = row.sector_2_s.or(row.sector_2_alt);
        let sector_3 = row.sector_3_s.or(row.sector_3_alt);
        let avg_speed = row.avg_speed_kph.or(row.average_speed_kph);
        let temp_avg = row.tyre_temp_avg_c.or(row.tyre_temp_avg_alt);
        let temp_delta = row.tyre_temp_delta_c.or(row.temp_delta_c);

        laps.push(LapMetrics {
            lap_number,
            lap_time_s,
            is_complete: true,
            sectors: match (sector_1, sector_2, sector_3) {
                (Some(s1), Some(s2), Some(s3)) => vec![s1, s2, s3],
                _ => Vec::new(),
            },
            avg_speed_kph: avg_speed,
            tyre_temp_avg_c: temp_avg,
            tyre_temp_delta_c: temp_delta,
        });
    }

    if laps.is_empty() {
        anyhow::bail!("no valid lap data found in {}", path.display());
    }

    Ok(laps)
}

pub fn maybe_scan_directory(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir).with_context(|| format!("failed to read telemetry directory: {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
        if matches!(extension.to_ascii_lowercase().as_str(), "csv" | "ibt") {
            files.push(path);
        }
    }
    Ok(files)
}

pub fn collect_telemetry_files(file: Option<&PathBuf>, dir: Option<&PathBuf>) -> Result<Vec<PathBuf>> {
    if let Some(file) = file {
        let file = normalize_path(file);
        if file.is_dir() {
            return maybe_scan_directory(&file);
        }
        return Ok(vec![file]);
    }

    if let Some(dir) = dir {
        let dir = normalize_path(dir);
        if dir.is_file() {
            return Ok(vec![dir]);
        }
        return maybe_scan_directory(&dir);
    }

    match default_telemetry_dir() {
        Some(dir) => maybe_scan_directory(&dir),
        None => Ok(vec![PathBuf::from("data/practice_session.csv")]),
    }
}

/// iRacing's telemetry folder. `PCC_TELEMETRY_DIR` wins if set; otherwise it's
/// `Documents\iRacing\telemetry`, checking OneDrive-redirected Documents first (OneDrive
/// sets the `OneDrive*` variables to its root, whatever the folder is called).
pub fn default_telemetry_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("PCC_TELEMETRY_DIR").map(PathBuf::from) {
        if dir.is_dir() {
            return Some(dir);
        }
        eprintln!("PCC_TELEMETRY_DIR is set but {} is not a folder; ignoring it.", dir.display());
    }

    let mut documents: Vec<PathBuf> = ["OneDrive", "OneDriveConsumer", "OneDriveCommercial"]
        .iter()
        .filter_map(|var| std::env::var_os(var))
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
