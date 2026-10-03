use anyhow::Result;
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};

use crate::telemetry::trace::{lap_metrics, Capture, Frame, LapMetrics, TrackInfo};

/// Collects frames and publishes finished laps to `sink` while a recording is running.
/// Fed by the real iRacing connection or by an .ibt replay.
struct Recorder {
    track: TrackInfo,
    frames: Vec<Frame>,
    sink: Arc<Mutex<Vec<LapMetrics>>>,
    republish_at: Option<f64>,
}

impl Recorder {
    fn new(track: TrackInfo, sink: Arc<Mutex<Vec<LapMetrics>>>) -> Self {
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
            let laps = lap_metrics(&self.frames, &self.track);
            if let Ok(mut sink) = self.sink.lock() {
                *sink = laps;
            }
        }
    }

    fn finish(self) -> Capture {
        Capture { track: self.track, frames: self.frames }
    }
}

/// Records until `stop_signal` is set. Finished laps are also published to `live_sink`
/// so callers can show lap times while the recording is still running.
#[cfg(windows)]
pub fn run_live_telemetry_until_stopped(
    stop_signal: Arc<AtomicBool>,
    live_sink: Arc<Mutex<Vec<LapMetrics>>>,
) -> Result<Capture> {
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
            crate::telemetry::track::with_saved(TrackInfo {
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
                shift_rpm: Some(session.drivers.shift_light_shift_rpm as f64).filter(|v| *v > 0.0),
                redline_rpm: Some(session.drivers.red_line_rpm as f64).filter(|v| *v > 0.0),
                // The rest of the weather comes from the telemetry channels on every sample.
                weather: crate::telemetry::weather::WeatherSnapshot {
                    weather_type: session.weekend.track_weather.clone(),
                    skies: session.weekend.track_skies.clone(),
                    wind_ms: crate::telemetry::weather::wind_ms(&session.weekend.track_wind_speed),
                    ..Default::default()
                },
            })
            .0
        }
        Err(err) => {
            eprintln!("Could not read iRacing session info ({err}); recording without track details.");
            TrackInfo::default()
        }
    };

    let blocking = conn.blocking().map_err(|e| anyhow::anyhow!("Could not create telemetry handle: {e}"))?;
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

    let channels: Vec<&'static str> = crate::telemetry::trace::channel_names().collect();
    while !stop_signal.load(Ordering::Relaxed) {
        // Timeouts while iRacing is paused or loading are expected: just poll again.
        let Ok(telem) = blocking.sample(Duration::from_millis(500)) else { continue };

        // Not in the car on track (garage, spectating, replay): nothing useful to record.
        if num(&telem, "IsOnTrack") == Some(0.0) {
            continue;
        }
        let frame = Frame::from_channels(|i| num(&telem, channels[i]));
        recorder.push(frame);
    }

    Ok(recorder.finish())
}

#[cfg(not(windows))]
pub fn run_live_telemetry_until_stopped(
    _stop_signal: Arc<AtomicBool>,
    _live_sink: Arc<Mutex<Vec<LapMetrics>>>,
) -> Result<Capture> {
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
) -> Result<Capture> {
    let data = crate::telemetry::ibt::read_ibt(path)?;
    let track = crate::telemetry::track::with_saved(data.track).0;
    let mut recorder = Recorder::new(track, live_sink);
    let speed = if speed > 0.0 { speed } else { 1.0 };

    let started = Instant::now();
    let first_time = data.frames.iter().map(|f| f.time).find(|t| t.is_finite()).unwrap_or(0.0);
    for frame in data.frames {
        if stop_signal.load(Ordering::Relaxed) {
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
    use crate::telemetry::trace::build_run;

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