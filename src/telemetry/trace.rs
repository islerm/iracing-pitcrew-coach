//! Turns raw telemetry frames (live or from an .ibt file) into lap metrics, distance-based
//! lap traces for overlays, and a track outline with detected corners.

use serde::Serialize;

use crate::telemetry::balance::add_balance;
use crate::telemetry::weather::{lap_weather, WeatherSample, WeatherSnapshot};

/// One telemetry sample. Missing channels are NaN.
#[derive(Debug, Clone, Copy)]
pub struct Frame {
    pub time: f64,
    pub lap: i32,
    pub pct: f64,
    pub speed_ms: f32,
    pub throttle: f32,
    pub brake: f32,
    pub gear: i32,
    pub steer_rad: f32,
    pub yaw: f32,
    pub lat: f64,
    pub lon: f64,
    pub on_pit_road: bool,
    /// iRacing's official time for the previous lap (updates shortly after the line).
    pub last_lap_time: f32,
    /// Lateral / longitudinal acceleration in m/s² (iRacing's `LatAccel`/`LongAccel`, which
    /// include gravity, so banking and slopes show up too).
    pub lat_accel: f32,
    pub long_accel: f32,
    /// `YawRate` in rad/s. NaN when the channel is missing; traces then derive it from heading.
    pub yaw_rate: f32,
    /// `BrakeABSactive` as 1.0/0.0. NaN for cars or files without it.
    pub abs_active: f32,
    /// `PlayerTrackSurface` (iRacing's TrkLoc: 0 = off track, 3 = on track). `None` when missing.
    pub track_surface: Option<i8>,
    /// `PlayerCarMyIncidentCount`, cumulative over the session. NaN when missing.
    pub incidents: f32,
    /// Engine RPM. NaN when missing.
    pub rpm: f32,
    /// `Alt`: altitude in metres. Only in .ibt files (live telemetry has no position data).
    pub alt: f32,
    pub weather: WeatherSample,
}

/// Everything read from one recording: an .ibt file or a live session.
pub struct Capture {
    pub track: TrackInfo,
    pub frames: Vec<Frame>,
}

/// Telemetry channels read into a `Frame`, in the order `Frame::from_channels` expects.
/// `YawNorth` is preferred over `Yaw` (which has a per-track offset).
const FRAME_CHANNELS: [&str; 22] = [
    "SessionTime", "Lap", "LapDistPct", "Speed", "Throttle", "Brake", "Gear", "SteeringWheelAngle", "YawNorth",
    "Yaw", "Lat", "Lon", "OnPitRoad", "LapLastLapTime", "LatAccel", "LongAccel", "YawRate", "BrakeABSactive",
    "PlayerTrackSurface", "PlayerCarMyIncidentCount", "RPM", "Alt",
];

/// Every channel a `Frame` needs, frame channels first then the weather ones: the index
/// space of `Frame::from_channels`.
pub fn channel_names() -> impl Iterator<Item = &'static str> {
    FRAME_CHANNELS.into_iter().chain(crate::telemetry::weather::CHANNELS)
}

impl Frame {
    /// Builds a frame from `get(i)`, the value of the `i`th entry of `channel_names()`
    /// (None when the source lacks it).
    pub fn from_channels(get: impl Fn(usize) -> Option<f64>) -> Self {
        let values: [Option<f64>; FRAME_CHANNELS.len()] = std::array::from_fn(&get);
        let [time, lap, pct, speed, throttle, brake, gear, steer, yaw_north, yaw, lat, lon, pit, last_lap, lat_accel, long_accel, yaw_rate, abs, surface, incidents, rpm, alt] =
            values;
        let nan = |v: Option<f64>| v.unwrap_or(f64::NAN);
        Self {
            time: nan(time),
            lap: lap.unwrap_or(0.0) as i32,
            pct: nan(pct),
            speed_ms: nan(speed) as f32,
            throttle: nan(throttle) as f32,
            brake: nan(brake) as f32,
            gear: gear.unwrap_or(0.0) as i32,
            steer_rad: nan(steer) as f32,
            yaw: nan(yaw_north.or(yaw)) as f32,
            lat: nan(lat),
            lon: nan(lon),
            on_pit_road: pit.unwrap_or(0.0) != 0.0,
            last_lap_time: nan(last_lap) as f32,
            lat_accel: nan(lat_accel) as f32,
            long_accel: nan(long_accel) as f32,
            yaw_rate: nan(yaw_rate) as f32,
            abs_active: nan(abs) as f32,
            track_surface: surface.map(|v| v as i8),
            incidents: nan(incidents) as f32,
            rpm: nan(rpm) as f32,
            alt: nan(alt) as f32,
            weather: WeatherSample::from_channels(|i| get(FRAME_CHANNELS.len() + i)),
        }
    }
}

impl Default for Frame {
    fn default() -> Self {
        Self {
            time: 0.0,
            lap: 0,
            pct: 0.0,
            speed_ms: f32::NAN,
            throttle: f32::NAN,
            brake: f32::NAN,
            gear: 0,
            steer_rad: f32::NAN,
            yaw: f32::NAN,
            lat: f64::NAN,
            lon: f64::NAN,
            on_pit_road: false,
            last_lap_time: f32::NAN,
            lat_accel: f32::NAN,
            long_accel: f32::NAN,
            yaw_rate: f32::NAN,
            abs_active: f32::NAN,
            track_surface: None,
            incidents: f32::NAN,
            rpm: f32::NAN,
            alt: f32::NAN,
            weather: WeatherSample::default(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct TrackInfo {
    /// iRacing's internal id, e.g. "roadatlanta full". Used as the key for saved track maps.
    pub track_name: String,
    /// iRacing's numeric `TrackID`, which locates the official track map.
    pub track_id: Option<u32>,
    pub display_name: String,
    pub config_name: String,
    pub length_m: f64,
    /// Sector start points as lap fractions, starting with 0.0.
    pub sector_pcts: Vec<f64>,
    pub car: String,
    /// RPM where the car's shift light says to change up (`DriverCarSLShiftRPM`).
    pub shift_rpm: Option<f64>,
    /// `DriverCarRedLine`.
    pub redline_rpm: Option<f64>,
    /// Conditions from the session info (see `weather`).
    pub weather: WeatherSnapshot,
}

/// Channels resampled onto a fixed lap-distance grid (`n + 1` points from 0.0 to 1.0),
/// so any two laps can be compared point by point.
#[derive(Debug, Clone, Serialize)]
pub struct LapTrace {
    pub lap_number: i32,
    pub time_s: Vec<f32>,
    pub speed_kph: Vec<f32>,
    pub throttle: Vec<f32>,
    pub brake: Vec<f32>,
    pub gear: Vec<i8>,
    pub steer_deg: Vec<f32>,
    /// Lateral and longitudinal acceleration in g. Empty when the source has no accelerometer data.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub lat_g: Vec<f32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub long_g: Vec<f32>,
    /// Yaw rate in deg/s, from `YawRate` or, when that's missing, differentiated heading.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub yaw_rate_dps: Vec<f32>,
    /// 1 where ABS was active. Empty when the car or file has no ABS channel.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub abs: Vec<u8>,
    /// Handling balance in steering-wheel degrees: how much more lock the driver used than the
    /// car's rotation needed. Positive = understeer, negative = oversteer. See `handling`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub balance_deg: Vec<f32>,
    /// Engine RPM. Empty when the source has no RPM channel.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub rpm: Vec<f32>,
    /// Altitude in metres. Empty for live recordings, which have no position data.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub alt_m: Vec<f32>,
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct Turn {
    pub label: String,
    pub pct: f64,
}

/// Each turn owns the stretch from halfway after the previous turn to halfway to the next,
/// matching the UI's corner table.
pub fn turn_segments(turns: &[Turn]) -> Vec<(f64, f64, f64)> {
    (0..turns.len())
        .map(|i| {
            let start = if i == 0 { 0.0 } else { (turns[i - 1].pct + turns[i].pct) / 2.0 };
            let end = if i + 1 == turns.len() { 1.0 } else { (turns[i].pct + turns[i + 1].pct) / 2.0 };
            (start, turns[i].pct, end)
        })
        .collect()
}

#[derive(Debug, Clone, Serialize)]
pub struct LapMetrics {
    pub lap_number: i32,
    pub lap_time_s: f64,
    pub is_complete: bool,
    /// Sector times in order. Empty when the source has no sector data.
    pub sectors: Vec<f64>,
    pub avg_speed_kph: Option<f64>,
    /// Lap fractions where the car left the track. Empty when it stayed on, or when the
    /// source has no track-surface channel.
    pub off_track_pcts: Vec<f64>,
    /// Incident points picked up during the lap. `None` when the source has no incident count.
    pub incidents: Option<u32>,
    /// Average air and track temperature over the lap, and the wettest the track got
    /// (`TrackWetness`: 1 dry … 7 extremely wet). `None` when the source has no weather channels.
    pub air_temp_c: Option<f64>,
    pub track_temp_c: Option<f64>,
    pub track_wetness: Option<u8>,
    /// Top speed in kph, and the share of the lap (%) at full throttle and on the brakes.
    /// `None` when the source has no speed / pedal channels.
    pub top_speed_kph: Option<f64>,
    pub full_throttle_pct: Option<f64>,
    pub braking_pct: Option<f64>,
}

impl LapMetrics {
    /// Whether two laps were driven on a similar track surface, so their times say something
    /// about the driving: track wetness at most one step apart (dry and mostly dry compare,
    /// dry and very lightly wet don't). Laps with no wetness data compare with anything.
    pub fn same_conditions(&self, other: &LapMetrics) -> bool {
        match (self.track_wetness, other.track_wetness) {
            (Some(a), Some(b)) => a.abs_diff(b) <= 1,
            _ => true,
        }
    }
}

pub struct RunData {
    pub laps: Vec<LapMetrics>,
    pub traces: Vec<LapTrace>,
    /// Track outline points (metres, y = north/up) on the same grid as the traces.
    pub map_points: Option<Vec<[f32; 2]>>,
    /// True when `map_points` came from GPS rather than integrated heading.
    pub map_from_gps: bool,
}

/// Grid used for traces and the map: roughly one point every 3 m.
fn grid_size(length_m: f64) -> usize {
    if length_m > 0.0 {
        ((length_m / 3.0).round() as usize).clamp(400, 4000)
    } else {
        1000
    }
}

/// Standard gravity, for converting m/s² to g.
const G: f32 = 9.80665;

fn round2(value: f32) -> f32 {
    (value * 100.0).round() / 100.0
}

/// Linear interpolation of the time the car crossed the line between `before` (end of the
/// previous lap) and `after` (start of the next).
fn line_crossing(before: &Frame, after: &Frame) -> Option<f64> {
    if before.pct < 0.9 || after.pct > 0.1 || after.time - before.time > 1.0 {
        return None;
    }
    let to_line = 1.0 - before.pct;
    let span = to_line + after.pct;
    let fraction = if span > 0.0 { to_line / span } else { 0.5 };
    Some(before.time + (after.time - before.time) * fraction)
}

/// iRacing's `LapLastLapTime` for the lap that ended at frame `end`: the first new value
/// that appears within a few seconds after the line.
fn official_lap_time(frames: &[Frame], end: usize) -> Option<f64> {
    let held = frames.get(end.checked_sub(1)?)?.last_lap_time;
    let start_time = frames.get(end)?.time;
    frames[end..]
        .iter()
        .take_while(|f| f.time - start_time < 3.0)
        .map(|f| f.last_lap_time)
        .find(|v| v.is_finite() && *v > 0.0 && *v != held)
        .map(|v| v as f64)
}

/// A lap is only timed if it was driven continuously from line to line: no pit road,
/// no resets/tows (large jumps in lap distance) and no gaps in the recording.
pub(crate) fn is_continuous(frames: &[Frame]) -> bool {
    frames.iter().all(|f| !f.on_pit_road)
        && frames.windows(2).all(|w| {
            let dp = w[1].pct - w[0].pct;
            dp < 0.02 && dp > -0.01 && w[1].time - w[0].time < 1.0
        })
}

/// iRacing's `PlayerTrackSurface` value for "off track".
const SURFACE_OFF_TRACK: i8 = 0;

/// Top speed in kph, and the share of the lap (%) spent at full throttle and on the brakes.
/// Samples are evenly spaced in time, so a share of samples is a share of the lap time.
fn lap_driving(frames: &[Frame]) -> (Option<f64>, Option<f64>, Option<f64>) {
    let top = frames.iter().map(|f| f.speed_ms).filter(|v| v.is_finite()).fold(None, |m: Option<f32>, v| Some(m.map_or(v, |m| m.max(v))));
    let share = |value: fn(&Frame) -> f32, on: fn(f32) -> bool| {
        let known: Vec<f32> = frames.iter().map(value).filter(|v| v.is_finite()).collect();
        (!known.is_empty()).then(|| known.iter().filter(|v| on(**v)).count() as f64 / known.len() as f64 * 100.0)
    };
    (
        top.map(|v| v as f64 * 3.6),
        share(|f| f.throttle, |t| t >= 0.98),
        share(|f| f.brake, |b| b >= 0.05),
    )
}

/// Where the lap went off track (one lap fraction per excursion; brief returns to the
/// surface within a second count as the same excursion) and how many incident points it
/// picked up. `prev` is the last frame before the lap, the baseline for the incident count.
fn lap_off_tracks(prev: Option<&Frame>, frames: &[Frame]) -> (Vec<f64>, Option<u32>) {
    let mut pcts = Vec::new();
    let mut last_off: Option<f64> = None;
    for f in frames {
        if f.track_surface == Some(SURFACE_OFF_TRACK) && !f.on_pit_road {
            if last_off.is_none_or(|t| f.time - t > 1.0) && f.pct.is_finite() {
                pcts.push(f.pct);
            }
            last_off = Some(f.time);
        }
    }

    let counts = prev.into_iter().chain(frames).map(|f| f.incidents).filter(|v| v.is_finite());
    let (first, last) = counts.fold((None, None), |(first, _), v| (first.or(Some(v)), Some(v)));
    let incidents = first.zip(last).map(|(a, b)| (b - a).max(0.0).round() as u32);
    (pcts, incidents)
}

struct GridPoint {
    pct: f64,
    time: f64,
    frame: Frame,
    yaw_unwrapped: f64,
}

fn interpolate(a: f32, b: f32, t: f64) -> f32 {
    if a.is_nan() {
        return b;
    }
    if b.is_nan() {
        return a;
    }
    a + (b - a) * t as f32
}

struct Resampled {
    trace: LapTrace,
    yaw: Vec<f64>,
    lat: Vec<f64>,
    lon: Vec<f64>,
}

/// Resample a complete lap's points (strictly increasing pct, 0.0 → 1.0) onto the grid.
fn resample(points: &[GridPoint], lap_number: i32, n: usize) -> Resampled {
    let mut out = Resampled {
        trace: LapTrace {
            lap_number,
            time_s: Vec::with_capacity(n + 1),
            speed_kph: Vec::with_capacity(n + 1),
            throttle: Vec::with_capacity(n + 1),
            brake: Vec::with_capacity(n + 1),
            gear: Vec::with_capacity(n + 1),
            steer_deg: Vec::with_capacity(n + 1),
            lat_g: Vec::with_capacity(n + 1),
            long_g: Vec::with_capacity(n + 1),
            yaw_rate_dps: Vec::with_capacity(n + 1),
            abs: Vec::with_capacity(n + 1),
            balance_deg: Vec::new(),
            rpm: Vec::with_capacity(n + 1),
            alt_m: Vec::with_capacity(n + 1),
        },
        yaw: Vec::with_capacity(n + 1),
        lat: Vec::with_capacity(n + 1),
        lon: Vec::with_capacity(n + 1),
    };

    let mut abs_seen = false;
    let mut k = 0;
    for j in 0..=n {
        let p = j as f64 / n as f64;
        while k + 2 < points.len() && points[k + 1].pct < p {
            k += 1;
        }
        let a = &points[k];
        let b = &points[(k + 1).min(points.len() - 1)];
        let t = if b.pct > a.pct { ((p - a.pct) / (b.pct - a.pct)).clamp(0.0, 1.0) } else { 0.0 };
        let (fa, fb) = (&a.frame, &b.frame);

        let trace = &mut out.trace;
        trace.time_s.push((a.time + (b.time - a.time) * t) as f32);
        trace.speed_kph.push(round2(interpolate(fa.speed_ms, fb.speed_ms, t) * 3.6));
        trace.throttle.push(round2(interpolate(fa.throttle, fb.throttle, t) * 100.0));
        trace.brake.push(round2(interpolate(fa.brake, fb.brake, t) * 100.0));
        trace.gear.push(if t < 0.5 { fa.gear } else { fb.gear } as i8);
        trace.steer_deg.push(round2(interpolate(fa.steer_rad, fb.steer_rad, t).to_degrees()));
        trace.lat_g.push(round2(interpolate(fa.lat_accel, fb.lat_accel, t) / G));
        trace.long_g.push(round2(interpolate(fa.long_accel, fb.long_accel, t) / G));
        trace.yaw_rate_dps.push(round2(interpolate(fa.yaw_rate, fb.yaw_rate, t).to_degrees()));
        trace.abs.push(if t < 0.5 { fa.abs_active } else { fb.abs_active } as u8);
        trace.rpm.push(interpolate(fa.rpm, fb.rpm, t).round());
        trace.alt_m.push(round2(interpolate(fa.alt, fb.alt, t)));
        abs_seen |= fa.abs_active.is_finite();
        out.yaw.push(a.yaw_unwrapped + (b.yaw_unwrapped - a.yaw_unwrapped) * t);
        out.lat.push(fa.lat + (fb.lat - fa.lat) * t);
        out.lon.push(fa.lon + (fb.lon - fa.lon) * t);
    }

    let trace = &mut out.trace;
    if !abs_seen {
        trace.abs.clear();
    }
    if trace.rpm.iter().any(|v| !v.is_finite()) {
        trace.rpm.clear();
    }
    if trace.alt_m.iter().any(|v| !v.is_finite()) {
        trace.alt_m.clear();
    }
    if trace.lat_g.iter().chain(&trace.long_g).any(|v| !v.is_finite()) {
        trace.lat_g.clear();
        trace.long_g.clear();
    }
    if trace.yaw_rate_dps.iter().any(|v| !v.is_finite()) {
        trace.yaw_rate_dps = yaw_rate_from_heading(&out.yaw, &trace.time_s);
    }
    out
}

/// Yaw rate (deg/s) by differentiating unwrapped heading over time, for files without
/// `YawRate`. Central differences over ~5 grid points keep quantization noise down.
fn yaw_rate_from_heading(yaw: &[f64], time_s: &[f32]) -> Vec<f32> {
    if yaw.iter().any(|v| !v.is_finite()) {
        return Vec::new();
    }
    let n = yaw.len() - 1;
    let half = 2;
    (0..=n)
        .map(|j| {
            let (a, b) = (j.saturating_sub(half), (j + half).min(n));
            let dt = (time_s[b] - time_s[a]) as f64;
            if dt > 0.0 { round2(((yaw[b] - yaw[a]) / dt).to_degrees() as f32) } else { 0.0 }
        })
        .collect()
}

/// Time since the lap start at a given lap fraction, from the lap's increasing points.
fn time_at(points: &[GridPoint], pct: f64) -> Option<f64> {
    let i = points.iter().position(|p| p.pct >= pct)?;
    if i == 0 {
        return Some(points[0].time);
    }
    let (a, b) = (&points[i - 1], &points[i]);
    let t = if b.pct > a.pct { (pct - a.pct) / (b.pct - a.pct) } else { 0.0 };
    Some(a.time + (b.time - a.time) * t)
}

/// A stretch of frames between two line crossings (or gaps/resets): `frames[start..end]`.
struct Segment {
    start: usize,
    end: usize,
    number: i32,
}

/// Split into laps at line crossings (lap distance wrapping from ~1 to ~0). iRacing's Lap
/// counter is used for numbering when it counts, but it isn't relied on: some sessions leave
/// it at 0. Recording gaps and resets also start a new segment.
fn segment_laps(frames: &[Frame]) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut start = 0;
    let mut number = frames.first().map(|f| f.lap.max(1)).unwrap_or(1);
    for i in 1..=frames.len() {
        let boundary = i == frames.len() || {
            let (a, b) = (&frames[i - 1], &frames[i]);
            let crossed = a.pct > 0.9 && b.pct < 0.1;
            let jumped = b.time - a.time > 1.0 || (!crossed && (b.pct - a.pct).abs() > 0.02);
            crossed || jumped || b.lap != a.lap
        };
        if boundary {
            segments.push(Segment { start, end: i, number });
            if i < frames.len() {
                number = if frames[i].lap > number { frames[i].lap } else { number + 1 };
            }
            start = i;
        }
    }
    segments
}

/// A lap's start time and duration, when the frames either side of it let the line
/// crossings be found.
struct LapTiming {
    start_time: f64,
    lap_time_s: f64,
}

/// Times a segment from the line crossings at its ends, preferring iRacing's own lap time so
/// numbers match the sim (our interpolation can be a frame, ~17 ms, off).
fn lap_timing(frames: &[Frame], seg: &Segment) -> Option<LapTiming> {
    let lap_frames = &frames[seg.start..seg.end];
    let prev = frames.get(seg.start.checked_sub(1)?)?;
    let next = frames.get(seg.end)?;
    let t0 = line_crossing(prev, &lap_frames[0])?;
    let t1 = line_crossing(&lap_frames[lap_frames.len() - 1], next)?;
    let lap_time_s = match official_lap_time(frames, seg.end) {
        Some(official) if (official - (t1 - t0)).abs() < 0.25 => official,
        _ => t1 - t0,
    };
    Some(LapTiming { start_time: t0, lap_time_s })
}

/// Increasing-pct points with virtual start/end points exactly on the line. Heading is not
/// unwrapped yet (see `unwrap_heading`).
fn lap_points(lap_frames: &[Frame], timing: &LapTiming) -> Vec<GridPoint> {
    let mut points: Vec<GridPoint> = Vec::with_capacity(lap_frames.len() + 2);
    points.push(GridPoint { pct: 0.0, time: 0.0, frame: lap_frames[0], yaw_unwrapped: 0.0 });
    for f in lap_frames {
        if f.pct > points.last().map(|p| p.pct).unwrap_or(0.0) && f.pct < 1.0 {
            let time = (f.time - timing.start_time).clamp(0.0, timing.lap_time_s);
            points.push(GridPoint { pct: f.pct, time, frame: *f, yaw_unwrapped: 0.0 });
        }
    }
    points.push(GridPoint {
        pct: 1.0,
        time: timing.lap_time_s,
        frame: lap_frames[lap_frames.len() - 1],
        yaw_unwrapped: 0.0,
    });
    points
}

/// Unwrap heading so interpolation never swings the long way round.
fn unwrap_heading(points: &mut [GridPoint]) {
    let mut offset = 0.0;
    let mut last_raw = points[0].frame.yaw as f64;
    for p in points.iter_mut() {
        let raw = p.frame.yaw as f64;
        if !raw.is_nan() && !last_raw.is_nan() {
            let d = raw - last_raw;
            if d > std::f64::consts::PI {
                offset -= std::f64::consts::TAU;
            } else if d < -std::f64::consts::PI {
                offset += std::f64::consts::TAU;
            }
        }
        if !raw.is_nan() {
            last_raw = raw;
        }
        p.yaw_unwrapped = raw + offset;
    }
}

/// Sector durations from the lap's points. Empty without sector boundaries.
fn sector_times(points: &[GridPoint], lap_time_s: f64, sector_pcts: &[f64]) -> Vec<f64> {
    if sector_pcts.len() < 2 {
        return Vec::new();
    }
    let mut bounds: Vec<f64> = sector_pcts.iter().copied().filter(|p| *p > 0.0 && *p < 1.0).collect();
    bounds.push(1.0);
    let mut sectors = Vec::with_capacity(bounds.len());
    let mut prev_time = 0.0;
    for b in bounds {
        let t = if b >= 1.0 { lap_time_s } else { time_at(points, b).unwrap_or(prev_time) };
        sectors.push(t - prev_time);
        prev_time = t;
    }
    sectors
}

/// One lap's metrics, plus its distance-ordered points when it was timed and driven
/// continuously (the only laps that get a trace).
struct AnalysedLap {
    metrics: LapMetrics,
    points: Option<Vec<GridPoint>>,
}

fn analyse_lap(frames: &[Frame], seg: &Segment, track: &TrackInfo) -> Option<AnalysedLap> {
    let lap_frames = &frames[seg.start..seg.end];
    if lap_frames.len() < 2 {
        return None;
    }
    // Skip segments where the car was parked (garage, pit box, paused).
    let covered: f64 = lap_frames.windows(2).map(|w| (w[1].pct - w[0].pct).max(0.0)).sum();
    if covered < 0.05 {
        return None;
    }

    let prev = seg.start.checked_sub(1).map(|i| &frames[i]);
    let timing = lap_timing(frames, seg);
    let complete = timing.is_some() && is_continuous(lap_frames);
    let points = timing.as_ref().filter(|_| complete).map(|t| lap_points(lap_frames, t));
    let lap_time_s = match &timing {
        Some(t) => t.lap_time_s,
        None => lap_frames[lap_frames.len() - 1].time - lap_frames[0].time,
    };
    let sectors = points.as_deref().map(|p| sector_times(p, lap_time_s, &track.sector_pcts)).unwrap_or_default();

    let avg_speed_kph = if complete && track.length_m > 0.0 {
        Some(track.length_m / lap_time_s * 3.6)
    } else {
        let speeds: Vec<f64> = lap_frames.iter().map(|f| f.speed_ms as f64).filter(|v| !v.is_nan()).collect();
        (!speeds.is_empty()).then(|| speeds.iter().sum::<f64>() / speeds.len() as f64 * 3.6)
    };
    let (off_track_pcts, incidents) = lap_off_tracks(prev, lap_frames);
    let (air_temp_c, track_temp_c, track_wetness) = lap_weather(lap_frames);
    let (top_speed_kph, full_throttle_pct, braking_pct) = lap_driving(lap_frames);

    let metrics = LapMetrics {
        lap_number: seg.number,
        lap_time_s,
        is_complete: complete,
        sectors,
        avg_speed_kph,
        off_track_pcts,
        incidents,
        air_temp_c,
        track_temp_c,
        track_wetness,
        top_speed_kph,
        full_throttle_pct,
        braking_pct,
    };
    Some(AnalysedLap { metrics, points })
}

/// Every lap in the recording, in the order driven.
fn analyse_laps(frames: &[Frame], track: &TrackInfo) -> Vec<AnalysedLap> {
    let mut laps: Vec<AnalysedLap> = segment_laps(frames).iter().filter_map(|seg| analyse_lap(frames, seg, track)).collect();
    // Without a working lap counter, number laps 1, 2, 3… in the order driven.
    if frames.iter().all(|f| f.lap <= 0) {
        for (i, lap) in laps.iter_mut().enumerate() {
            lap.metrics.lap_number = i as i32 + 1;
        }
    }
    laps
}

/// Per-lap metrics only: the cheap path for live recording, which republishes laps as they
/// finish and has no use for traces or the map.
pub fn lap_metrics(frames: &[Frame], track: &TrackInfo) -> Vec<LapMetrics> {
    analyse_laps(frames, track).into_iter().map(|lap| lap.metrics).collect()
}

/// The track outline from the fastest clean lap: GPS when it has it, else dead-reckoned
/// from heading. The flag is true for GPS.
fn choose_outline(resampled: &[Resampled], track: &TrackInfo, n: usize) -> (Option<Vec<[f32; 2]>>, bool) {
    let best = resampled
        .iter()
        .min_by(|a, b| a.trace.time_s[n].partial_cmp(&b.trace.time_s[n]).unwrap_or(std::cmp::Ordering::Equal));
    let Some(best) = best else { return (None, false) };
    if best.lat.iter().all(|v| v.is_finite() && *v != 0.0) {
        (Some(outline_from_gps(&best.lat, &best.lon)), true)
    } else if best.yaw.iter().all(|v| v.is_finite()) {
        let length = if track.length_m > 0.0 {
            track.length_m
        } else {
            best.trace.speed_kph.iter().map(|v| *v as f64 / 3.6).sum::<f64>() / n as f64 * best.trace.time_s[n] as f64
        };
        (Some(outline_from_heading(&best.yaw, length)), false)
    } else {
        (None, false)
    }
}

/// Laps, distance-based traces and the track outline for a recording.
pub fn build_run(frames: &[Frame], track: &TrackInfo) -> RunData {
    let n = grid_size(track.length_m);
    let mut laps = Vec::new();
    let mut resampled = Vec::new();
    for lap in analyse_laps(frames, track) {
        if let Some(mut points) = lap.points {
            unwrap_heading(&mut points);
            resampled.push(resample(&points, lap.metrics.lap_number, n));
        }
        laps.push(lap.metrics);
    }

    let (map_points, map_from_gps) = choose_outline(&resampled, track, n);
    let mut traces: Vec<LapTrace> = resampled.into_iter().map(|r| r.trace).collect();
    add_balance(&mut traces);

    RunData { laps, traces, map_points, map_from_gps }
}

fn outline_from_gps(lat: &[f64], lon: &[f64]) -> Vec<[f32; 2]> {
    const EARTH_RADIUS_M: f64 = 6_371_000.0;
    let (lat0, lon0) = (lat[0], lon[0]);
    let cos_lat = lat0.to_radians().cos();
    lat.iter()
        .zip(lon)
        .map(|(la, lo)| {
            let x = (lo - lon0).to_radians() * EARTH_RADIUS_M * cos_lat;
            let y = (la - lat0).to_radians() * EARTH_RADIUS_M;
            [(x * 10.0).round() as f32 / 10.0, (y * 10.0).round() as f32 / 10.0]
        })
        .collect()
}

/// Dead-reckon the outline from heading along evenly spaced lap distance, then spread
/// the closing error around the lap so the loop joins up.
///
/// `yaw` is iRacing's `YawNorth`: a compass bearing, clockwise from north. Checked against
/// GPS in real .ibt files, the direction of travel is (sin(yaw), cos(yaw)) in an x-east /
/// y-north frame, so the outline comes out north-up like the GPS one. (Plain `Yaw` has a
/// per-track offset and is only used as a fallback.)
fn outline_from_heading(yaw: &[f64], length_m: f64) -> Vec<[f32; 2]> {
    let n = yaw.len() - 1;
    let ds = length_m / n as f64;
    let mut points = Vec::with_capacity(n + 1);
    let (mut x, mut y) = (0.0_f64, 0.0_f64);
    points.push((x, y));
    for j in 0..n {
        let heading = (yaw[j] + yaw[j + 1]) / 2.0;
        x += heading.sin() * ds;
        y += heading.cos() * ds;
        points.push((x, y));
    }
    let (ex, ey) = points[n];
    points
        .iter()
        .enumerate()
        .map(|(j, (px, py))| {
            let f = j as f64 / n as f64;
            let cx = px - ex * f;
            let cy = py - ey * f;
            [(cx * 10.0).round() as f32 / 10.0, (cy * 10.0).round() as f32 / 10.0]
        })
        .collect()
}

/// Find corners from the outline's curvature and number them from the start line.
pub fn detect_turns(points: &[[f32; 2]], length_m: f64) -> Vec<Turn> {
    let n = points.len().saturating_sub(1);
    if n < 50 || length_m <= 0.0 {
        return Vec::new();
    }
    let ds = length_m / n as f64;
    let wrap = |a: f64| {
        let mut a = a;
        while a > std::f64::consts::PI {
            a -= std::f64::consts::TAU;
        }
        while a < -std::f64::consts::PI {
            a += std::f64::consts::TAU;
        }
        a
    };

    // Heading change between consecutive segments (circular: the last point equals the first).
    let heading: Vec<f64> = (0..n)
        .map(|j| {
            let (a, b) = (points[j], points[j + 1]);
            ((b[1] - a[1]) as f64).atan2((b[0] - a[0]) as f64)
        })
        .collect();
    let dh: Vec<f64> = (0..n).map(|j| wrap(heading[(j + 1) % n] - heading[j])).collect();

    // Curvature smoothed over ~40 m.
    let half = ((20.0 / ds).round() as usize).max(1);
    let curvature: Vec<f64> = (0..n)
        .map(|j| {
            let sum: f64 = (0..=2 * half).map(|k| dh[(j + n + k - half) % n]).sum();
            sum / ((2 * half + 1) as f64 * ds)
        })
        .collect();

    const MIN_CURVATURE: f64 = 1.0 / 300.0; // tighter than a 300 m radius
    const MIN_TURN_RAD: f64 = 0.35; // ~20 degrees of total direction change
    let merge_gap = ((30.0 / ds).round() as usize).max(1);

    // Start scanning from a straight section so no corner is split across the wrap.
    let Some(origin) = (0..n).find(|&j| curvature[j].abs() < MIN_CURVATURE) else {
        return Vec::new();
    };

    struct Region {
        start: usize,
        end: usize,
        sign: f64,
    }
    let mut regions: Vec<Region> = Vec::new();
    let mut k = 0;
    while k < n {
        let j = (origin + k) % n;
        let c = curvature[j];
        if c.abs() >= MIN_CURVATURE {
            let sign = c.signum();
            let start = k;
            while k < n && curvature[(origin + k) % n].abs() >= MIN_CURVATURE && curvature[(origin + k) % n].signum() == sign {
                k += 1;
            }
            match regions.last_mut() {
                Some(last) if last.sign == sign && start - last.end <= merge_gap => last.end = k,
                _ => regions.push(Region { start, end: k, sign }),
            }
        } else {
            k += 1;
        }
    }

    let mut turns: Vec<Turn> = regions
        .iter()
        .filter_map(|r| {
            let total: f64 = (r.start..r.end).map(|k| dh[(origin + k) % n]).sum();
            if total.abs() < MIN_TURN_RAD {
                return None;
            }
            let apex = (r.start..r.end)
                .max_by(|a, b| {
                    curvature[(origin + a) % n]
                        .abs()
                        .partial_cmp(&curvature[(origin + b) % n].abs())
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|k| (origin + k) % n)?;
            Some(Turn { label: String::new(), pct: apex as f64 / n as f64 })
        })
        .collect();

    turns.sort_by(|a, b| a.pct.partial_cmp(&b.pct).unwrap_or(std::cmp::Ordering::Equal));
    for (i, turn) in turns.iter_mut().enumerate() {
        turn.label = (i + 1).to_string();
    }
    turns
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// Anonymized Road Atlanta session made with `--make-fixture` (4 timed laps).
    const FIXTURE: &str = "tests/fixtures/roadatlanta-full.ibt";

    #[test]
    fn laps_compare_only_in_similar_conditions() {
        let lap = |track_wetness: Option<u8>| LapMetrics {
            lap_number: 1,
            lap_time_s: 90.0,
            is_complete: true,
            sectors: Vec::new(),
            avg_speed_kph: None,
            off_track_pcts: Vec::new(),
            incidents: None,
            air_temp_c: None,
            track_temp_c: None,
            track_wetness,
            top_speed_kph: None,
            full_throttle_pct: None,
            braking_pct: None,
        };
        assert!(lap(Some(1)).same_conditions(&lap(Some(2))));
        assert!(!lap(Some(1)).same_conditions(&lap(Some(3))));
        assert!(!lap(Some(5)).same_conditions(&lap(Some(1))));
        assert!(lap(None).same_conditions(&lap(Some(6))));
    }

    /// RMS distance (m) between the GPS outline and the heading-only outline that live
    /// mode has to use, for the same frames.
    fn heading_vs_gps_rms(frames: &[Frame], track: &TrackInfo) -> f64 {
        let gps = build_run(frames, track);
        assert!(gps.map_from_gps, "expected GPS channels");
        let no_gps: Vec<Frame> = frames.iter().map(|f| Frame { lat: f64::NAN, lon: f64::NAN, ..*f }).collect();
        let heading = build_run(&no_gps, track);
        assert!(!heading.map_from_gps);
        let (a, b) = (gps.map_points.unwrap(), heading.map_points.unwrap());
        let sum: f64 = a.iter().zip(&b).map(|(p, q)| ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)) as f64).sum();
        (sum / a.len() as f64).sqrt()
    }

    #[test]
    fn fixture_laps_match_iracing() {
        let data = crate::telemetry::ibt::read_ibt(Path::new(FIXTURE)).expect("read fixture");
        assert_eq!(data.track.display_name, "Road Atlanta");
        assert_eq!(data.track.car, "Dallara P217 LMP2");
        assert_eq!(data.track.sector_pcts.len(), 4);

        let run = build_run(&data.frames, &data.track);
        let timed: Vec<&LapMetrics> = run.laps.iter().filter(|lap| lap.is_complete).collect();
        // iRacing's own LapLastLapTime values for these laps.
        let official = [79.792, 78.091, 77.097, 76.901];
        assert_eq!(timed.len(), official.len());
        for (lap, want) in timed.iter().zip(official) {
            assert!((lap.lap_time_s - want).abs() < 0.001, "lap {} was {}", lap.lap_number, lap.lap_time_s);
            assert_eq!(lap.sectors.len(), 4);
            let sum: f64 = lap.sectors.iter().sum();
            assert!((sum - lap.lap_time_s).abs() < 0.002, "sectors don't add up on lap {}", lap.lap_number);
        }
        // The partial laps either side of the timed run are kept but not timed.
        assert!(run.laps.iter().any(|lap| !lap.is_complete));
        assert_eq!(run.traces.len(), timed.len());
        for lap in &timed {
            let (top, full, brake) = (lap.top_speed_kph.unwrap(), lap.full_throttle_pct.unwrap(), lap.braking_pct.unwrap());
            assert!((200.0..350.0).contains(&top), "lap {} top speed {top}", lap.lap_number);
            assert!((30.0..90.0).contains(&full), "lap {} full throttle {full}%", lap.lap_number);
            assert!((3.0..40.0).contains(&brake), "lap {} braking {brake}%", lap.lap_number);
        }
    }

    #[test]
    fn fixture_live_map_matches_gps() {
        let data = crate::telemetry::ibt::read_ibt(Path::new(FIXTURE)).expect("read fixture");
        let rms = heading_vs_gps_rms(&data.frames, &data.track);
        assert!(rms < 15.0, "heading outline is {rms:.1} m RMS from GPS");

        let points = build_run(&data.frames, &data.track).map_points.unwrap();
        let turns = detect_turns(&points, data.track.length_m);
        assert!((8..=14).contains(&turns.len()), "detected {} turns", turns.len());
        assert!(turns.windows(2).all(|w| w[0].pct < w[1].pct));
    }

    /// Every committed fixture, so new ones get the same checks automatically.
    fn all_fixtures() -> Vec<std::path::PathBuf> {
        let mut files: Vec<_> = std::fs::read_dir("tests/fixtures")
            .expect("tests/fixtures")
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|path| path.extension().map(|ext| ext == "ibt").unwrap_or(false))
            .collect();
        files.sort();
        assert!(!files.is_empty(), "no fixtures in tests/fixtures");
        files
    }

    #[test]
    fn fixtures_have_no_identifying_session_info() {
        for path in all_fixtures() {
            let bytes = std::fs::read(&path).expect("read fixture");
            let text = String::from_utf8_lossy(&bytes);
            for banned in ["UserID", "TeamName", "ClubName", "CarSetup", "SessionID", "SubSessionID", "WeekendOptions"] {
                assert!(!text.contains(banned), "{} contains {banned}", path.display());
            }
            assert!(text.contains("UserName: Test Driver"), "{} was not made with --make-fixture", path.display());
            // Disk sub-header (session date/time) is zeroed apart from the record count.
            assert!(bytes[112..140].iter().all(|b| *b == 0), "{} keeps the session date", path.display());
        }
    }

    #[test]
    fn fixtures_have_timed_laps_sectors_and_accurate_live_maps() {
        for path in all_fixtures() {
            let name = path.display();
            let data = crate::telemetry::ibt::read_ibt(&path).expect("read fixture");
            assert!(!data.track.display_name.is_empty(), "{name}: no track name");
            let run = build_run(&data.frames, &data.track);
            let timed: Vec<&LapMetrics> = run.laps.iter().filter(|lap| lap.is_complete).collect();
            assert!(timed.len() >= 3, "{name}: only {} timed laps", timed.len());
            for lap in &timed {
                assert_eq!(lap.sectors.len(), data.track.sector_pcts.len(), "{name}: lap {} sectors", lap.lap_number);
                let sum: f64 = lap.sectors.iter().sum();
                assert!((sum - lap.lap_time_s).abs() < 0.002, "{name}: sectors don't add up on lap {}", lap.lap_number);
            }

            let rms = heading_vs_gps_rms(&data.frames, &data.track);
            assert!(rms < 15.0, "{name}: heading outline is {rms:.1} m RMS from GPS");
            let points = run.map_points.expect("map");
            let turns = detect_turns(&points, data.track.length_m);
            assert!((6..=20).contains(&turns.len()), "{name}: detected {} turns", turns.len());
        }
    }

    #[test]
    fn watkins_glen_laps_match_iracing() {
        let data = crate::telemetry::ibt::read_ibt(Path::new("tests/fixtures/watkinsglen-2021-fullcourse.ibt")).expect("read fixture");
        let run = build_run(&data.frames, &data.track);
        let timed: Vec<f64> = run.laps.iter().filter(|lap| lap.is_complete).map(|lap| lap.lap_time_s).collect();
        // iRacing's own LapLastLapTime values for these laps.
        let official = [108.027, 106.047, 106.283, 105.486];
        assert_eq!(timed.len(), official.len());
        for (got, want) in timed.iter().zip(official) {
            assert!((got - want).abs() < 0.001, "lap time {got} vs official {want}");
        }
    }

    /// Checks the heading-based outline against GPS on any real file:
    /// `PCC_IBT=path\to\file.ibt cargo test outline -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn outline_matches_gps() {
        let path = std::env::var("PCC_IBT").expect("set PCC_IBT");
        let data = crate::telemetry::ibt::read_ibt(Path::new(&path)).unwrap();
        println!("{} ({}), {:.0} m, sectors {:?}", data.track.display_name, data.track.car, data.track.length_m, data.track.sector_pcts);
        for lap in build_run(&data.frames, &data.track).laps {
            println!("lap {:>3} {:>8.3}s timed={} sectors={:?}", lap.lap_number, lap.lap_time_s, lap.is_complete, lap.sectors);
        }
        let rms = heading_vs_gps_rms(&data.frames, &data.track);
        println!("heading vs GPS outline: {rms:.1} m RMS");
        assert!(rms < 30.0);
    }

    #[test]
    fn off_tracks_and_incidents_per_lap() {
        // 10 Hz lap: off at 20% for 0.3 s, a 0.5 s blip back on (same excursion), then off
        // again at 60% (a new one). Incidents go 2 → 4, counted from the frame before the lap.
        let frame = |i: usize| {
            let pct = i as f64 / 100.0;
            let off = (20..23).contains(&i) || (28..30).contains(&i) || (60..62).contains(&i);
            Frame {
                time: i as f64 * 0.1,
                pct,
                track_surface: Some(if off { SURFACE_OFF_TRACK } else { 3 }),
                incidents: if i < 22 { 2.0 } else { 4.0 },
                ..Frame::default()
            }
        };
        let frames: Vec<Frame> = (0..100).map(frame).collect();
        let prev = Frame { incidents: 2.0, ..Frame::default() };
        let (pcts, incidents) = lap_off_tracks(Some(&prev), &frames);
        assert_eq!(pcts, vec![0.2, 0.6]);
        assert_eq!(incidents, Some(2));

        // Files without the channels report nothing, not zero.
        let bare: Vec<Frame> = (0..10).map(|i| Frame { time: i as f64, ..Frame::default() }).collect();
        assert_eq!(lap_off_tracks(None, &bare), (Vec::new(), None));
    }
}
