use anyhow::Result;
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::Sender,
    Arc, Mutex,
};
use std::time::{Duration, Instant};

use crate::field::capture::{CarSample, FieldRecorder};
use crate::field::{Field, Roster};
use crate::telemetry::trace::{grid_size, lap_metrics, Capture, Frame, LapMetrics, TrackInfo};

/// What the radio hears about from the recorder, each with a snapshot of the capture so far.
pub enum RadioCue {
    /// A lap finished and iRacing's official time for it is in.
    LapDone(Capture),
    /// The car just entered the pit lane.
    PitEntry(Capture),
}

/// Seconds off pit road before entering it again counts as a new pit visit, so flicker at the
/// pit-lane line doesn't trigger a second debrief.
const NEW_PIT_VISIT_S: f64 = 10.0;

/// Collects frames and publishes finished laps to `sink` while a recording is running.
/// Fed by the real iRacing connection or by an .ibt replay.
struct Recorder {
    track: TrackInfo,
    frames: Vec<Frame>,
    sink: Arc<Mutex<Vec<LapMetrics>>>,
    republish_at: Option<f64>,
    /// Gets a snapshot of the capture after each lap and on entering the pit lane.
    radio: Option<Sender<RadioCue>>,
    /// Session time the car last left pit road (`None` before it has).
    left_pits_at: Option<f64>,
    /// Who else is in the session, and their laps when recording live.
    roster: Roster,
    field: FieldRecorder,
}

impl Recorder {
    fn new(track: TrackInfo, sink: Arc<Mutex<Vec<LapMetrics>>>, radio: Option<Sender<RadioCue>>, roster: Roster) -> Self {
        let field = FieldRecorder::new(grid_size(track.length_m));
        Self { track, frames: Vec::new(), sink, republish_at: None, radio, left_pits_at: None, roster, field }
    }

    fn snapshot(&self) -> Capture {
        let field = Field { roster: self.roster.clone(), laps: self.field.laps().to_vec() };
        Capture { track: self.track.clone(), frames: self.frames.clone(), field }
    }

    /// One tick of the other cars (see `FieldRecorder::push`).
    #[cfg_attr(not(windows), allow(dead_code))]
    fn push_field(&mut self, time: f64, wetness: Option<u8>, cars: Vec<CarSample>) {
        self.field.push(time, wetness, cars);
    }

    /// A fresher roster from the session info; an empty one (a failed read) is ignored.
    #[cfg_attr(not(windows), allow(dead_code))]
    fn set_roster(&mut self, roster: Roster) {
        if !roster.drivers.is_empty() {
            self.roster = roster;
        }
    }

    fn push(&mut self, frame: Frame) {
        if !frame.pct.is_finite() || frame.pct < 0.0 {
            return;
        }
        let last = self.frames.last().copied();
        // The Lap counter can't be relied on (some sessions leave it at 0), so a line
        // crossing is also detected from lap distance wrapping around.
        let crossed_line = last.map(|last| last.lap != frame.lap || (last.pct > 0.9 && frame.pct < 0.1)).unwrap_or(false);
        let left_pits = last.is_some_and(|last| last.on_pit_road && !frame.on_pit_road);
        let entered_pits = last.is_some_and(|last| !last.on_pit_road && frame.on_pit_road)
            && self.left_pits_at.is_none_or(|t| frame.time - t >= NEW_PIT_VISIT_S);
        if left_pits {
            self.left_pits_at = Some(frame.time);
        }
        self.frames.push(frame);

        // Publish laps as they finish so the UI can show them during the run, and again a
        // moment later once iRacing's official lap time for that lap has come through.
        if crossed_line {
            self.republish_at = Some(frame.time + 2.5);
        }
        let republish = self.republish_at.map(|t| frame.time >= t).unwrap_or(false);
        if republish {
            self.republish_at = None;
            if let Some(tx) = &self.radio {
                let _ = tx.send(RadioCue::LapDone(self.snapshot()));
            }
        }
        if entered_pits {
            if let Some(tx) = &self.radio {
                let _ = tx.send(RadioCue::PitEntry(self.snapshot()));
            }
        }
        if crossed_line || republish {
            let laps = lap_metrics(&self.frames, &self.track);
            if let Ok(mut sink) = self.sink.lock() {
                *sink = laps;
            }
        }
    }

    fn finish(self) -> Capture {
        let field = Field { roster: self.roster, laps: self.field.into_laps() };
        Capture { track: self.track, frames: self.frames, field }
    }
}

/// How long the driver has to be out of the car (or iRacing silent) before an
/// auto-started recording ends by itself.
pub const AUTO_END_AFTER: Duration = Duration::from_secs(10);

#[cfg(windows)]
fn num(sample: &iracing::telemetry::Sample, name: &'static str) -> Option<f64> {
    use iracing::telemetry::Value;
    match sample.get(name).ok()? {
        Value::FLOAT(v) => Some(v as f64),
        Value::DOUBLE(v) => Some(v),
        Value::INT(v) => Some(v as f64),
        Value::BITS(v) => Some(v as f64),
        Value::BOOL(v) => Some(if v { 1.0 } else { 0.0 }),
        _ => None,
    }
}

/// Tells whether the player is currently driving in a live iRacing session, without
/// recording anything. Polled to start recordings automatically.
#[cfg(windows)]
#[derive(Default)]
pub struct SessionProbe {
    // Kept open between polls: the iracing crate never unmaps a connection's view.
    conn: Option<iracing::telemetry::Connection>,
    last_tick: Option<f64>,
}

#[cfg(windows)]
impl SessionProbe {
    pub fn driving(&mut self) -> bool {
        if self.conn.is_none() {
            // Fails while iRacing isn't running.
            self.conn = iracing::telemetry::Connection::new().ok();
        }
        let Some(sample) = self.conn.as_ref().and_then(|conn| conn.telemetry().ok()) else { return false };
        // The shared memory outlives iRacing with its last values still in it, so only a
        // session clock that is still moving counts as live.
        let tick = num(&sample, "SessionTick");
        let advancing = tick.is_some() && self.last_tick.is_some() && tick != self.last_tick;
        self.last_tick = tick;
        advancing && num(&sample, "IsOnTrack") == Some(1.0)
    }
}

#[cfg(not(windows))]
#[derive(Default)]
pub struct SessionProbe;

#[cfg(not(windows))]
impl SessionProbe {
    pub fn driving(&mut self) -> bool {
        false
    }
}

/// Records until `stop_signal` is set. Finished laps are also published to `live_sink`
/// so callers can show lap times while the recording is still running. With `auto_end`,
/// it also stops once the driver has been out of the car (or iRacing has gone quiet)
/// for [`AUTO_END_AFTER`].
#[cfg(windows)]
pub fn run_live_telemetry_until_stopped(
    stop_signal: Arc<AtomicBool>,
    live_sink: Arc<Mutex<Vec<LapMetrics>>>,
    auto_end: bool,
    radio: Option<Sender<RadioCue>>,
) -> Result<Capture> {
    use iracing::telemetry::Connection;

    let mut conn = Connection::new().map_err(|e| anyhow::anyhow!("Unable to open telemetry. Is iRacing running? {e}"))?;

    // Track details are nice to have (map + sectors) but not required to time laps.
    let (track, roster) = match conn.session_info() {
        Ok(session) => {
            let car = session
                .drivers
                .other_drivers
                .iter()
                .find(|driver| driver.index == session.drivers.car_index)
                .map(|driver| driver.car_screen_name.clone())
                .unwrap_or_default();
            // The session running now, from the `SessionNum` channel (the info lists them all).
            let session_num = conn.telemetry().ok().and_then(|sample| num(&sample, "SessionNum"));
            let sessions = &session.session.sessions;
            let session_type = session_num
                .and_then(|n| sessions.iter().find(|s| s.session_number as f64 == n))
                .or(sessions.first())
                .map(|s| s.session_type.clone())
                .unwrap_or_default();
            let roster = crate::field::roster::from_session_details(&session, session_num.map(|n| n as i32));
            let track = crate::telemetry::track::with_saved(TrackInfo {
                track_name: session.weekend.track_name.clone(),
                track_id: Some(session.weekend.track_id).filter(|id| *id > 0),
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
                session_type,
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
            .0;
            (track, roster)
        }
        Err(err) => {
            eprintln!("Could not read iRacing session info ({err}); recording without track details.");
            (TrackInfo::default(), Roster::default())
        }
    };

    let blocking = conn.blocking().map_err(|e| anyhow::anyhow!("Could not create telemetry handle: {e}"))?;
    let mut recorder = Recorder::new(track, live_sink, radio, roster);

    let channels: Vec<&'static str> = crate::telemetry::trace::channel_names().collect();
    let mut last_on_track = Instant::now();
    let mut roster_read = Instant::now();
    while !stop_signal.load(Ordering::Relaxed) {
        if auto_end && last_on_track.elapsed() >= AUTO_END_AFTER {
            break;
        }
        // Timeouts while iRacing is paused or loading are expected: just poll again.
        let Ok(telem) = blocking.sample(Duration::from_millis(500)) else { continue };

        // Drivers join and results change during a session.
        if roster_read.elapsed() >= ROSTER_REFRESH {
            roster_read = Instant::now();
            if let Ok(session) = conn.session_info() {
                let session_num = num(&telem, "SessionNum").map(|n| n as i32);
                recorder.set_roster(crate::field::roster::from_session_details(&session, session_num));
            }
        }
        // The rest of the player's class, whether or not the player is on track right now.
        if let Some(time) = num(&telem, "SessionTime") {
            let wetness = num(&telem, "TrackWetness").filter(|w| *w >= 1.0).map(|w| w as u8);
            let cars = field_samples(&telem, &recorder.roster);
            recorder.push_field(time, wetness, cars);
        }

        // Not in the car on track (garage, spectating, replay): nothing useful to record.
        if num(&telem, "IsOnTrack") == Some(0.0) {
            continue;
        }
        last_on_track = Instant::now();
        let frame = Frame::from_channels(|i| num(&telem, channels[i]));
        recorder.push(frame);
    }

    Ok(recorder.finish())
}

/// How often the session info (drivers, results) is read again during a recording.
#[cfg(windows)]
const ROSTER_REFRESH: Duration = Duration::from_secs(30);

/// This tick's per-car channels for the other cars in the player's class.
#[cfg(windows)]
fn field_samples(sample: &iracing::telemetry::Sample, roster: &Roster) -> Vec<CarSample> {
    use iracing::telemetry::Value;
    let floats = |name: &'static str| match sample.get(name) {
        Ok(Value::FloatVec(v)) => v,
        _ => Vec::new(),
    };
    let ints = |name: &'static str| match sample.get(name) {
        Ok(Value::IntVec(v)) => v,
        _ => Vec::new(),
    };
    let bools = |name: &'static str| match sample.get(name) {
        Ok(Value::BoolVec(v)) => v,
        _ => Vec::new(),
    };
    let pct = floats("CarIdxLapDistPct");
    let gear = ints("CarIdxGear");
    let pit = bools("CarIdxOnPitRoad");
    let surface = ints("CarIdxTrackSurface");
    let last = floats("CarIdxLastLapTime");
    (0..pct.len())
        .map(|i| i as i32)
        .filter(|&car| car != roster.player_car_idx && roster.in_player_class(car))
        .map(|car| {
            let i = car as usize;
            CarSample {
                car_idx: car,
                pct: pct[i],
                gear: gear.get(i).copied().unwrap_or(0) as i8,
                on_pit_road: pit.get(i).copied().unwrap_or(false),
                surface: surface.get(i).copied().unwrap_or(-1) as i8,
                last_lap_time: last.get(i).copied().unwrap_or(-1.0),
            }
        })
        .collect()
}

#[cfg(not(windows))]
pub fn run_live_telemetry_until_stopped(
    _stop_signal: Arc<AtomicBool>,
    _live_sink: Arc<Mutex<Vec<LapMetrics>>>,
    _auto_end: bool,
    _radio: Option<Sender<RadioCue>>,
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
    radio: Option<Sender<RadioCue>>,
) -> Result<Capture> {
    let data = crate::telemetry::ibt::read_ibt(path)?;
    let track = crate::telemetry::track::with_saved(data.track).0;
    let mut recorder = Recorder::new(track, live_sink, radio, data.field.roster);
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
    fn pit_entry_is_cued_once_per_visit() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut recorder = Recorder::new(TrackInfo::default(), Arc::new(Mutex::new(Vec::new())), Some(tx), Roster::default());
        let frame = |time: f64, pit: bool| {
            let mut f = Frame::from_channels(|_| None);
            (f.time, f.lap, f.pct, f.on_pit_road) = (time, 1, 0.5, pit);
            f
        };
        // Out of the pits, round, and in: one cue. Flickering at the pit-lane line: no more.
        for (time, pit) in [(0.0, true), (1.0, false), (60.0, false), (61.0, true), (62.0, false), (63.0, true)] {
            recorder.push(frame(time, pit));
        }
        let cues: Vec<RadioCue> = rx.try_iter().collect();
        assert_eq!(cues.len(), 1);
        assert!(matches!(&cues[0], RadioCue::PitEntry(capture) if capture.frames.len() == 4));
    }

    #[test]
    fn replay_feeds_the_live_pipeline() {
        let sink = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let capture = replay_until_stopped(
            Path::new("tests/fixtures/roadatlanta-full.ibt"),
            1e6,
            stop,
            Arc::clone(&sink),
            None,
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