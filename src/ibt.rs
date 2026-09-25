//! Minimal reader for iRacing .ibt telemetry files.
//!
//! Only the header, variable table and sample buffers are decoded. The session-info YAML
//! is scanned for the handful of keys we need rather than deserialized against a full
//! schema, so files from newer sim builds don't break the import.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::trace::{Frame, TrackInfo};

const HEADER_SIZE: usize = 112;
const VAR_HEADER_SIZE: usize = 144;

#[derive(Debug, Clone)]
struct Var {
    ty: i32,
    offset: usize,
}

pub struct IbtData {
    pub track: TrackInfo,
    pub frames: Vec<Frame>,
}

fn i32_at(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn read_value(sample: &[u8], var: &Option<Var>) -> Option<f64> {
    let var = var.as_ref()?;
    let o = var.offset;
    let bytes = sample.get(o..)?;
    Some(match var.ty {
        0 | 1 => *bytes.first()? as f64,
        2 => i32::from_le_bytes(bytes.get(..4)?.try_into().ok()?) as f64,
        3 => u32::from_le_bytes(bytes.get(..4)?.try_into().ok()?) as f64,
        4 => f32::from_le_bytes(bytes.get(..4)?.try_into().ok()?) as f64,
        5 => f64::from_le_bytes(bytes.get(..8)?.try_into().ok()?),
        _ => return None,
    })
}

/// Value of the first `key: value` line found at any indentation after `from`.
fn yaml_value<'a>(yaml: &'a str, key: &str, from: usize) -> Option<(&'a str, usize)> {
    let needle = format!("{key}:");
    let mut pos = from;
    for line in yaml[from..].split_inclusive('\n') {
        let trimmed = line.trim_start().trim_start_matches("- ");
        if let Some(rest) = trimmed.strip_prefix(&needle) {
            return Some((rest.trim(), pos + line.len()));
        }
        pos += line.len();
    }
    None
}

pub fn parse_track_info(yaml: &str) -> TrackInfo {
    let get = |key: &str| yaml_value(yaml, key, 0).map(|(v, _)| v.to_string()).unwrap_or_default();

    let length_m = get("TrackLength")
        .split_whitespace()
        .next()
        .and_then(|v| v.parse::<f64>().ok())
        .map(|km| km * 1000.0)
        .unwrap_or(0.0);

    let mut sector_pcts = Vec::new();
    if let Some(split_start) = yaml.find("SplitTimeInfo:") {
        let mut pos = split_start;
        while let Some((value, next)) = yaml_value(yaml, "SectorStartPct", pos) {
            match value.parse::<f64>() {
                Ok(v) => sector_pcts.push(v),
                Err(_) => break,
            }
            pos = next;
        }
    }

    // The player's car: find their CarIdx entry in the DriverInfo driver list (CarIdx also
    // appears earlier in session results, so search from the Drivers: list only).
    let mut car = String::new();
    if let (Some(drivers), Some((idx, _))) = (yaml.find("DriverInfo:"), yaml_value(yaml, "DriverCarIdx", 0)) {
        let mut pos = drivers;
        while let Some((value, next)) = yaml_value(yaml, "CarIdx", pos) {
            if value == idx {
                if let Some((name, _)) = yaml_value(yaml, "CarScreenName", next) {
                    car = name.to_string();
                }
                break;
            }
            pos = next;
        }
    }

    TrackInfo {
        track_name: get("TrackName"),
        display_name: get("TrackDisplayName"),
        config_name: get("TrackConfigName"),
        length_m,
        sector_pcts,
        car,
    }
}

/// An opened .ibt: header fields, variable table and session YAML, with the reader
/// positioned at the first sample.
struct RawIbt {
    reader: BufReader<File>,
    header: [u8; HEADER_SIZE],
    /// (name, var, raw 144-byte header)
    vars: Vec<(String, Var, Vec<u8>)>,
    yaml: String,
    buf_len: usize,
}

fn open_raw(path: &Path) -> Result<RawIbt> {
    let file = File::open(path).map_err(|err| {
        // Windows "sharing violation": iRacing is still writing this session.
        if err.raw_os_error() == Some(32) {
            anyhow::anyhow!(
                "{} is still being recorded by iRacing. Leave the session (or stop logging with Alt+L) and try again.",
                path.display()
            )
        } else {
            anyhow::anyhow!("failed to open {}: {err}", path.display())
        }
    })?;
    let mut reader = BufReader::with_capacity(1 << 20, file);

    let mut header = [0u8; HEADER_SIZE];
    reader.read_exact(&mut header).context("file is too small to be an .ibt")?;
    let session_len = i32_at(&header, 16) as usize;
    let session_offset = i32_at(&header, 20) as u64;
    let num_vars = i32_at(&header, 24) as usize;
    let var_header_offset = i32_at(&header, 28) as u64;
    let buf_len = i32_at(&header, 36) as usize;
    let buf_offset = i32_at(&header, 52) as u64;
    if num_vars == 0 || buf_len == 0 || num_vars > 10_000 {
        bail!("{} does not look like an iRacing telemetry file", path.display());
    }

    let mut var_bytes = vec![0u8; num_vars * VAR_HEADER_SIZE];
    reader.seek(SeekFrom::Start(var_header_offset))?;
    reader.read_exact(&mut var_bytes)?;
    let vars = var_bytes
        .chunks_exact(VAR_HEADER_SIZE)
        .map(|chunk| {
            let name_bytes = &chunk[16..48];
            let end = name_bytes.iter().position(|b| *b == 0).unwrap_or(name_bytes.len());
            let name = String::from_utf8_lossy(&name_bytes[..end]).to_string();
            (name, Var { ty: i32_at(chunk, 0), offset: i32_at(chunk, 4) as usize }, chunk.to_vec())
        })
        .collect();

    let mut session_bytes = vec![0u8; session_len];
    reader.seek(SeekFrom::Start(session_offset))?;
    reader.read_exact(&mut session_bytes)?;
    // Session info is Latin-1; map bytes straight to chars.
    let yaml: String = session_bytes.iter().take_while(|b| **b != 0).map(|b| *b as char).collect();

    reader.seek(SeekFrom::Start(buf_offset))?;
    Ok(RawIbt { reader, header, vars, yaml, buf_len })
}

pub fn read_ibt(path: &Path) -> Result<IbtData> {
    let RawIbt { mut reader, vars, yaml, buf_len, .. } = open_raw(path)?;
    let find = |name: &str| vars.iter().find(|(n, _, _)| n == name).map(|(_, v, _)| v.clone());
    let track = parse_track_info(&yaml);

    let v_time = find("SessionTime");
    let v_lap = find("Lap");
    let v_pct = find("LapDistPct");
    let v_speed = find("Speed");
    let v_throttle = find("Throttle");
    let v_brake = find("Brake");
    let v_gear = find("Gear");
    let v_steer = find("SteeringWheelAngle");
    let v_yaw = find("YawNorth").or_else(|| find("Yaw"));
    let v_lat = find("Lat");
    let v_lon = find("Lon");
    let v_pit = find("OnPitRoad");
    let v_last_lap = find("LapLastLapTime");
    let v_lat_accel = find("LatAccel");
    let v_long_accel = find("LongAccel");
    let v_yaw_rate = find("YawRate");
    let v_abs = find("BrakeABSactive");
    let v_tyres: Vec<Option<Var>> = ["LFtempCL", "RFtempCL", "LRtempCL", "RRtempCL"].iter().map(|n| find(n)).collect();
    if v_lap.is_none() || v_pct.is_none() || v_time.is_none() {
        bail!("{} is missing Lap/LapDistPct/SessionTime channels", path.display());
    }

    let mut sample = vec![0u8; buf_len];
    let mut frames = Vec::new();
    let nan = |v: Option<f64>| v.unwrap_or(f64::NAN);
    while reader.read_exact(&mut sample).is_ok() {
        let temps: Vec<f32> = v_tyres
            .iter()
            .filter_map(|v| read_value(&sample, v))
            .map(|t| t as f32)
            .filter(|t| t.is_finite() && *t > 0.0)
            .collect();
        frames.push(Frame {
            time: nan(read_value(&sample, &v_time)),
            lap: read_value(&sample, &v_lap).unwrap_or(0.0) as i32,
            pct: nan(read_value(&sample, &v_pct)),
            speed_ms: nan(read_value(&sample, &v_speed)) as f32,
            throttle: nan(read_value(&sample, &v_throttle)) as f32,
            brake: nan(read_value(&sample, &v_brake)) as f32,
            gear: read_value(&sample, &v_gear).unwrap_or(0.0) as i32,
            steer_rad: nan(read_value(&sample, &v_steer)) as f32,
            yaw: nan(read_value(&sample, &v_yaw)) as f32,
            lat: nan(read_value(&sample, &v_lat)),
            lon: nan(read_value(&sample, &v_lon)),
            on_pit_road: read_value(&sample, &v_pit).unwrap_or(0.0) != 0.0,
            last_lap_time: nan(read_value(&sample, &v_last_lap)) as f32,
            tyre_avg: if temps.is_empty() { f32::NAN } else { temps.iter().sum::<f32>() / temps.len() as f32 },
            tyre_min: temps.iter().copied().fold(f32::NAN, f32::min),
            tyre_max: temps.iter().copied().fold(f32::NAN, f32::max),
            lat_accel: nan(read_value(&sample, &v_lat_accel)) as f32,
            long_accel: nan(read_value(&sample, &v_long_accel)) as f32,
            yaw_rate: nan(read_value(&sample, &v_yaw_rate)) as f32,
            abs_active: nan(read_value(&sample, &v_abs)) as f32,
        });
    }

    if frames.is_empty() {
        bail!("{} contains no telemetry samples", path.display());
    }
    Ok(IbtData { track, frames })
}

/// Channels kept in test fixtures: what the app reads, plus a few likely to be useful soon.
const FIXTURE_CHANNELS: &[&str] = &[
    "SessionTime", "Lap", "LapDistPct", "LapDist", "LapLastLapTime", "Speed", "RPM", "Gear",
    "Throttle", "Brake", "SteeringWheelAngle", "Yaw", "YawNorth", "Lat", "Lon", "LatAccel",
    "LongAccel", "OnPitRoad", "IsOnTrack", "LFtempCL", "RFtempCL", "LRtempCL", "RRtempCL",
    "YawRate", "BrakeABSactive", "BrakeRaw", "VelocityX", "VelocityY", "FuelLevel", "FuelUsePerHour",
    "dcBrakeBias", "dcABS", "dcTractionControl", "TrackTempCrew", "AirTemp",
];

fn type_size(ty: i32) -> usize {
    match ty {
        0 | 1 => 1,
        5 => 8,
        _ => 4,
    }
}

/// Session info for a fixture: only track, sectors and car. Driver names, iRacing IDs,
/// clubs, setup, weather and dates are left out entirely rather than masked.
fn fixture_yaml(original: &str, track: &TrackInfo) -> String {
    let raw = |key: &str| yaml_value(original, key, 0).map(|(v, _)| v.to_string()).unwrap_or_default();
    let mut yaml = String::from("---\nWeekendInfo:\n");
    yaml += &format!(" TrackName: {}\n", track.track_name);
    yaml += &format!(" TrackDisplayName: {}\n", track.display_name);
    yaml += &format!(" TrackConfigName: {}\n", track.config_name);
    yaml += &format!(" TrackLength: {}\n", raw("TrackLength"));
    yaml += "SplitTimeInfo:\n Sectors:\n";
    for (i, pct) in track.sector_pcts.iter().enumerate() {
        yaml += &format!(" - SectorNum: {i}\n   SectorStartPct: {pct:.6}\n");
    }
    yaml += "DriverInfo:\n DriverCarIdx: 0\n Drivers:\n - CarIdx: 0\n   UserName: Test Driver\n";
    yaml += &format!("   CarScreenName: {}\n", track.car);
    yaml += "...\n";
    yaml
}

/// Write an anonymized, trimmed copy of `input` for use as committed test data: a run of
/// `laps` consecutive timed laps (plus a few seconds either side so the line crossings
/// are in the data), only `FIXTURE_CHANNELS`, and sanitized session info.
pub fn write_fixture(input: &Path, out: Option<&Path>, laps: usize) -> Result<String> {
    let laps = laps.max(1);
    let frames = read_ibt(input)?.frames;
    let RawIbt { mut reader, header, vars, yaml, buf_len } = open_raw(input)?;
    let track = parse_track_info(&yaml);

    // Line crossings, then the first run of `laps` consecutive clean laps.
    let crossings: Vec<usize> = (1..frames.len())
        .filter(|&i| frames[i - 1].pct > 0.9 && frames[i].pct < 0.1 && frames[i].time - frames[i - 1].time < 1.0)
        .collect();
    let clean = |a: usize, b: usize| crate::trace::is_continuous(&frames[a..b]);
    let wanted = laps.min(crossings.len().saturating_sub(1));
    if wanted == 0 {
        bail!("{} has no complete lap to export", input.display());
    }
    let first = (0..crossings.len() - wanted)
        .find(|&k| (k..k + wanted).all(|j| clean(crossings[j], crossings[j + 1])))
        .with_context(|| format!("no run of {wanted} clean laps in {}", input.display()))?;
    let margin = 5.0;
    let t_start = frames[crossings[first]].time - margin;
    let t_end = frames[crossings[first + wanted]].time + margin;
    let range: Vec<usize> = (0..frames.len()).filter(|&i| frames[i].time >= t_start && frames[i].time <= t_end).collect();

    // New variable table: kept channels packed back to back.
    let kept: Vec<&(String, Var, Vec<u8>)> = FIXTURE_CHANNELS
        .iter()
        .filter_map(|name| vars.iter().find(|(n, _, raw)| n == name && i32_at(raw, 8) == 1))
        .collect();
    let mut new_offsets = Vec::with_capacity(kept.len());
    let mut new_buf_len = 0;
    for (_, var, _) in &kept {
        new_offsets.push(new_buf_len);
        new_buf_len += type_size(var.ty);
    }

    let mut session = fixture_yaml(&yaml, &track).into_bytes();
    session.push(0);
    let var_header_offset = HEADER_SIZE + 32;
    let session_offset = var_header_offset + kept.len() * VAR_HEADER_SIZE;
    let buf_offset = session_offset + session.len();

    let mut out_bytes: Vec<u8> = Vec::with_capacity(buf_offset + range.len() * new_buf_len);
    let mut new_header = [0u8; HEADER_SIZE];
    let put = |h: &mut [u8], at: usize, v: i32| h[at..at + 4].copy_from_slice(&v.to_le_bytes());
    put(&mut new_header, 0, i32_at(&header, 0)); // version
    put(&mut new_header, 4, 1); // status
    put(&mut new_header, 8, i32_at(&header, 8)); // tick rate
    put(&mut new_header, 16, session.len() as i32);
    put(&mut new_header, 20, session_offset as i32);
    put(&mut new_header, 24, kept.len() as i32);
    put(&mut new_header, 28, var_header_offset as i32);
    put(&mut new_header, 32, 1); // one buffer
    put(&mut new_header, 36, new_buf_len as i32);
    put(&mut new_header, 52, buf_offset as i32);
    out_bytes.extend_from_slice(&new_header);

    // Disk sub-header: no session date/time, just the record count.
    let mut disk = [0u8; 32];
    put(&mut disk, 28, range.len() as i32);
    out_bytes.extend_from_slice(&disk);

    for ((_, _, raw), offset) in kept.iter().zip(&new_offsets) {
        let mut var_header = raw.clone();
        put(&mut var_header, 4, *offset as i32);
        out_bytes.extend_from_slice(&var_header);
    }
    out_bytes.extend_from_slice(&session);

    let mut sample = vec![0u8; buf_len];
    let mut index = 0;
    let mut next = range.iter().peekable();
    while let Some(&&want) = next.peek() {
        if reader.read_exact(&mut sample).is_err() {
            break;
        }
        if index == want {
            for ((_, var, _), _) in kept.iter().zip(&new_offsets) {
                out_bytes.extend_from_slice(&sample[var.offset..var.offset + type_size(var.ty)]);
            }
            next.next();
        }
        index += 1;
    }

    let out_path = match out {
        Some(path) => path.to_path_buf(),
        None => {
            let slug: String = track
                .track_name
                .to_ascii_lowercase()
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
                .collect();
            let slug = if slug.trim_matches('-').is_empty() { "track".to_string() } else { slug.trim_matches('-').to_string() };
            Path::new("tests").join("fixtures").join(format!("{slug}.ibt"))
        }
    };
    if let Some(dir) = out_path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&out_path, &out_bytes).with_context(|| format!("failed to write {}", out_path.display()))?;

    // Read it back through the normal pipeline as a check.
    let check = read_ibt(&out_path)?;
    let run = crate::trace::build_run(&check.frames, &check.track);
    let times: Vec<String> = run
        .laps
        .iter()
        .filter(|lap| lap.is_complete)
        .map(|lap| format!("{:.3}", lap.lap_time_s))
        .collect();
    Ok(format!(
        "Wrote {} ({:.1} MB): {} · {}, {} timed laps [{}], {} channels.\n\
         Session info reduced to track, sectors and car; driver identity, IDs, setup and dates removed.",
        out_path.display(),
        out_bytes.len() as f64 / 1e6,
        track.display_name,
        track.car,
        times.len(),
        times.join(", "),
        kept.len(),
    ))
}