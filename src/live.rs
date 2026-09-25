use anyhow::Result;
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};

use crate::trace::{build_run, Frame, TrackInfo};
use crate::types::LapMetrics;

/// Everything recorded during a live session, ready for `trace::build_run`.
pub struct LiveCapture {
    pub track: TrackInfo,
    pub frames: Vec<Frame>,
}

/// Collects frames and publishes finished laps to `sink` while a recording is running.
/// Fed by the real iRacing connection or by an .ibt replay.
struct Recorder {
    track: TrackInfo,
    frames: Vec<Frame>,
    sink: Option<Arc<Mutex<Vec<LapMetrics>>>>,
    republish_at: Option<f64>,
}

impl Recorder {
    fn new(track: TrackInfo, sink: Option<Arc<Mutex<Vec<LapMetrics>>>>) -> Self {
        Self { track, frames: Vec::new(), sink, republish_at: None }
    }

    fn push(&mut self, frame: Frame) {
        if !frame.pct.is_finite() || frame.pct < 0.0 {
            return;
        }
        // The Lap counter can't be relied on (some sessions leave it at 0), so a line
        // crossing is also detected from lap distance wrapping around.
        let crossed_line = self
            .frames
            .last()
            .map(|last| last.lap != frame.lap || (last.pct > 0.9 && frame.pct < 0.1))
            .unwrap_or(false);
        self.frames.push(frame);

        // Publish laps as they finish so the UI can show them during the run, and again a
        // moment later once iRacing's official lap time for that lap has come through.
        if crossed_line {
            self.republish_at = Some(frame.time + 2.5);
        }
        let republish = self.republish_at.map(|t| frame.time >= t).unwrap_or(false);
        if republish {
            self.republish_at = None;
        }
        if crossed_line || republish {
            if let Some(sink) = &self.sink {
                let laps = build_run(&self.frames, &self.track).laps;
                if let Ok(mut sink) = sink.lock() {
                    *sink = laps;
                }
            }
        }
    }

    fn finish(self) -> LiveCapture {
        LiveCapture { track: self.track, frames: self.frames }
    }
}

fn stopped(stop_signal: &Option<Arc<AtomicBool>>) -> bool {
    stop_signal.as_ref().map(|flag| flag.load(Ordering::Relaxed)).unwrap_or(false)
}

#[cfg(windows)]
fn capture_live_telemetry(
    duration: Option<Duration>,
    stop_signal: Option<Arc<AtomicBool>>,
    live_sink: Option<Arc<Mutex<Vec<LapMetrics>>>>,
    verbose: bool,
) -> Result<LiveCapture> {
    use iracing::telemetry::{Connection, Sample, Value};

    let mut conn = Connection::new().map_err(|e| anyhow::anyhow!("Unable to open telemetry. Is iRacing running? {e}"))?;

    // Track details are nice to have (map + sectors) but not required to time laps.
    let track = match conn.session_info() {
        Ok(session) => {
            let car = session
                .drivers
                .other_drivers
                .iter()
                .find(|driver| driver.index == session.drivers.car_index)
                .map(|driver| driver.car_screen_name.clone())
                .unwrap_or_default();
            crate::track::with_saved_sectors(TrackInfo {
                track_name: session.weekend.track_name.clone(),
                display_name: session.weekend.track_display_name.clone(),
                config_name: session.weekend.track_config_name.clone(),
                length_m: session
                    .weekend
                    .track_length
                    .split_whitespace()
                    .next()
                    .and_then(|km| km.parse::<f64>().ok())
                    .map(|km| km * 1000.0)
                    .unwrap_or(0.0),
                sector_pcts: Vec::new(),
                car,
            })
        }
        Err(err) => {
            eprintln!("Could not read iRacing session info ({err}); recording without track details.");
            TrackInfo::default()
        }
    };

    let blocking = conn.blocking().map_err(|e| anyhow::anyhow!("Could not create telemetry handle: {e}"))?;
    let end = duration.map(|d| Instant::now() + d);
    let mut recorder = Recorder::new(track, live_sink);

    fn num(sample: &Sample, name: &'static str) -> Option<f64> {
        match sample.get(name).ok()? {
            Value::FLOAT(v) => Some(v as f64),
            Value::DOUBLE(v) => Some(v),
            Value::INT(v) => Some(v as f64),
            Value::BITS(v) => Some(v as f64),
            Value::BOOL(v) => Some(if v { 1.0 } else { 0.0 }),
            _ => None,
        }
    }

    while end.map(|deadline| Instant::now() < deadline).unwrap_or(true) {
        if stopped(&stop_signal) {
            break;
        }

        let telem = match blocking.sample(Duration::from_millis(500)) {
            Ok(sample) => sample,
            Err(err) => {
                if verbose {
                    println!("Telemetry error: {err:?}");
                }
                continue;
            }
        };

        let nan = |v: Option<f64>| v.unwrap_or(f64::NAN);
        let temps: Vec<f32> = ["LFtempCL", "RFtempCL", "LRtempCL", "RRtempCL"]
            .iter()
            .filter_map(|name| num(&telem, name))
            .map(|t| t as f32)
            .filter(|t| t.is_finite() && *t > 0.0)
            .collect();

        let frame = Frame {
            time: nan(num(&telem, "SessionTime")),
            lap: num(&telem, "Lap").unwrap_or(0.0) as i32,
            pct: nan(num(&telem, "LapDistPct")),
            speed_ms: nan(num(&telem, "Speed")) as f32,
            throttle: nan(num(&telem, "Throttle")) as f32,
            brake: nan(num(&telem, "Brake")) as f32,
            gear: num(&telem, "Gear").unwrap_or(0.0) as i32,
            steer_rad: nan(num(&telem, "SteeringWheelAngle")) as f32,
            yaw: nan(num(&telem, "YawNorth").or_else(|| num(&telem, "Yaw"))) as f32,
            lat: f64::NAN,
            lon: f64::NAN,
            on_pit_road: num(&telem, "OnPitRoad").unwrap_or(0.0) != 0.0,
            last_lap_time: nan(num(&telem, "LapLastLapTime")) as f32,
            tyre_avg: if temps.is_empty() { f32::NAN } else { temps.iter().sum::<f32>() / temps.len() as f32 },
            tyre_min: temps.iter().copied().fold(f32::NAN, f32::min),
            tyre_max: temps.iter().copied().fold(f32::NAN, f32::max),
            lat_accel: nan(num(&telem, "LatAccel")) as f32,
            long_accel: nan(num(&telem, "LongAccel")) as f32,
            yaw_rate: nan(num(&telem, "YawRate")) as f32,
            abs_active: nan(num(&telem, "BrakeABSactive")) as f32,
        };

        if verbose {
            println!(
                "Lap {:>3} {:>5.1}% {:>6.1} kph gear {}",
                frame.lap,
                frame.pct * 100.0,
                frame.speed_ms * 3.6,
                frame.gear
            );
        }

        // Not in the car on track (garage, spectating, replay): nothing useful to record.
        if num(&telem, "IsOnTrack").map(|v| v == 0.0).unwrap_or(false) {
            continue;
        }
        recorder.push(frame);
    }

    Ok(recorder.finish())
}

#[cfg(windows)]
pub fn run_live_telemetry(duration_seconds: u64) -> Result<Vec<LapMetrics>> {
    let capture = capture_live_telemetry(Some(Duration::from_secs(duration_seconds)), None, None, true)?;
    Ok(build_run(&capture.frames, &capture.track).laps)
}

/// Records until `stop_signal` is set. Finished laps are also published to `live_sink`
/// so callers can show lap times while the recording is still running.
#[cfg(windows)]
pub fn run_live_telemetry_until_stopped(
    stop_signal: Arc<AtomicBool>,
    live_sink: Arc<Mutex<Vec<LapMetrics>>>,
) -> Result<LiveCapture> {
    capture_live_telemetry(None, Some(stop_signal), Some(live_sink), false)
}

#[cfg(not(windows))]
pub fn run_live_telemetry(_duration_seconds: u64) -> Result<Vec<LapMetrics>> {
    anyhow::bail!("Live telemetry requires a Windows host with iRacing running. Use --replay <file.ibt> to simulate it.")
}

#[cfg(not(windows))]
pub fn run_live_telemetry_until_stopped(
    _stop_signal: Arc<AtomicBool>,
    _live_sink: Arc<Mutex<Vec<LapMetrics>>>,
) -> Result<LiveCapture> {
    anyhow::bail!("Live telemetry requires a Windows host with iRacing running. Start the UI with --replay <file.ibt> to simulate it.")
}

/// Plays an .ibt file back as if it were live telemetry, `speed` times faster than real
/// time, until the file ends or `stop_signal` is set. Lets the live UI be developed
/// without iRacing (e.g. on macOS).
pub fn replay_until_stopped(
    path: &Path,
    speed: f64,
    stop_signal: Arc<AtomicBool>,
    live_sink: Arc<Mutex<Vec<LapMetrics>>>,
) -> Result<LiveCapture> {
    let data = crate::ibt::read_ibt(path)?;
    let track = crate::track::with_saved_sectors(data.track);
    let mut recorder = Recorder::new(track, Some(live_sink));
    let stop = Some(stop_signal);
    let speed = if speed > 0.0 { speed } else { 1.0 };

    let started = Instant::now();
    let first_time = data.frames.iter().map(|f| f.time).find(|t| t.is_finite()).unwrap_or(0.0);
    for frame in data.frames {
        if stopped(&stop) {
            break;
        }
        if frame.time.is_finite() {
            let due = Duration::from_secs_f64(((frame.time - first_time) / speed).max(0.0));
            let elapsed = started.elapsed();
            // Sleep in small batches rather than per frame.
            if due > elapsed + Duration::from_millis(15) {
                std::thread::sleep(due - elapsed);
            }
        }
        recorder.push(frame);
    }
    Ok(recorder.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_feeds_the_live_pipeline() {
        let sink = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let capture = replay_until_stopped(
            Path::new("tests/fixtures/roadatlanta-full.ibt"),
            1e6,
            stop,
            Arc::clone(&sink),
        )
        .expect("replay fixture");

        // Laps were published while "recording", with official times once available.
        let published = sink.lock().unwrap().clone();
        let timed: Vec<f64> = published.iter().filter(|l| l.is_complete).map(|l| l.lap_time_s).collect();
        assert!(timed.len() >= 3, "published {timed:?}");

        let run = build_run(&capture.frames, &capture.track);
        assert_eq!(run.laps.iter().filter(|l| l.is_complete).count(), 4);
    }
}