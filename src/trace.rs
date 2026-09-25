//! Turns raw telemetry frames (live or from an .ibt file) into lap metrics, distance-based
//! lap traces for overlays, and a track outline with detected corners.

use serde::Serialize;

use crate::types::LapMetrics;

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
    pub tyre_avg: f32,
    pub tyre_min: f32,
    pub tyre_max: f32,
    /// Lateral / longitudinal acceleration in m/s² (iRacing's `LatAccel`/`LongAccel`, which
    /// include gravity, so banking and slopes show up too).
    pub lat_accel: f32,
    pub long_accel: f32,
    /// `YawRate` in rad/s. NaN when the channel is missing; traces then derive it from heading.
    pub yaw_rate: f32,
    /// `BrakeABSactive` as 1.0/0.0. NaN for cars or files without it.
    pub abs_active: f32,
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
            tyre_avg: f32::NAN,
            tyre_min: f32::NAN,
            tyre_max: f32::NAN,
            lat_accel: f32::NAN,
            long_accel: f32::NAN,
            yaw_rate: f32::NAN,
            abs_active: f32::NAN,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct TrackInfo {
    /// iRacing's internal id, e.g. "roadatlanta full". Used as the key for saved track maps.
    pub track_name: String,
    pub display_name: String,
    pub config_name: String,
    pub length_m: f64,
    /// Sector start points as lap fractions, starting with 0.0.
    pub sector_pcts: Vec<f64>,
    pub car: String,
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
}

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct Turn {
    pub label: String,
    pub pct: f64,
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
pub fn grid_size(length_m: f64) -> usize {
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
        abs_seen |= fa.abs_active.is_finite();
        out.yaw.push(a.yaw_unwrapped + (b.yaw_unwrapped - a.yaw_unwrapped) * t);
        out.lat.push(fa.lat + (fb.lat - fa.lat) * t);
        out.lon.push(fa.lon + (fb.lon - fa.lon) * t);
    }

    let trace = &mut out.trace;
    if !abs_seen {
        trace.abs.clear();
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

pub fn build_run(frames: &[Frame], track: &TrackInfo) -> RunData {
    let n = grid_size(track.length_m);

    // Split into laps at line crossings (lap distance wrapping from ~1 to ~0). iRacing's
    // Lap counter is used for numbering when it counts, but it isn't relied on: some
    // sessions leave it at 0. Recording gaps and resets also start a new segment.
    let mut groups: Vec<(usize, usize, i32)> = Vec::new();
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
            groups.push((start, i, number));
            if i < frames.len() {
                number = if frames[i].lap > number { frames[i].lap } else { number + 1 };
            }
            start = i;
        }
    }

    let mut laps = Vec::new();
    let mut resampled: Vec<Resampled> = Vec::new();

    for &(s, e, lap_number) in &groups {
        let lap_frames = &frames[s..e];
        if lap_frames.len() < 2 {
            continue;
        }
        // Skip segments where the car was parked (garage, pit box, paused).
        let covered: f64 = lap_frames.windows(2).map(|w| (w[1].pct - w[0].pct).max(0.0)).sum();
        if covered < 0.05 {
            continue;
        }

        let prev = (s > 0).then(|| &frames[s - 1]);
        let next = (e < frames.len()).then(|| &frames[e]);
        let start_time = prev.and_then(|p| line_crossing(p, &lap_frames[0]));
        let end_time = next.and_then(|nx| line_crossing(&lap_frames[lap_frames.len() - 1], nx));

        let tyre_avgs: Vec<f64> = lap_frames.iter().map(|f| f.tyre_avg as f64).filter(|v| !v.is_nan()).collect();
        let tyre_min = lap_frames.iter().map(|f| f.tyre_min).filter(|v| !v.is_nan()).fold(f32::INFINITY, f32::min);
        let tyre_max = lap_frames.iter().map(|f| f.tyre_max).filter(|v| !v.is_nan()).fold(f32::NEG_INFINITY, f32::max);
        let speeds: Vec<f64> = lap_frames.iter().map(|f| f.speed_ms as f64).filter(|v| !v.is_nan()).collect();

        // iRacing only refreshes tyre temperatures in the pit stall, so on track the values are
        // frozen. Only report them if they actually changed during the lap.
        let tyres_live = tyre_max - tyre_min > 0.05;
        let tyre_temp_avg_c = (tyres_live && !tyre_avgs.is_empty()).then(|| tyre_avgs.iter().sum::<f64>() / tyre_avgs.len() as f64);
        let tyre_temp_delta_c = tyres_live.then(|| (tyre_max - tyre_min) as f64);
        let avg_speed_kph = (!speeds.is_empty()).then(|| speeds.iter().sum::<f64>() / speeds.len() as f64 * 3.6);

        let (Some(t0), Some(t1)) = (start_time, end_time) else {
            laps.push(LapMetrics {
                lap_number,
                lap_time_s: lap_frames[lap_frames.len() - 1].time - lap_frames[0].time,
                is_complete: false,
                sectors: Vec::new(),
                avg_speed_kph,
                tyre_temp_avg_c,
                tyre_temp_delta_c,
            });
            continue;
        };

        let complete = is_continuous(lap_frames);
        // Prefer iRacing's own lap time so numbers match the sim; our line-crossing
        // interpolation can be a frame (~17 ms) off.
        let lap_time_s = match official_lap_time(frames, e) {
            Some(official) if (official - (t1 - t0)).abs() < 0.25 => official,
            _ => t1 - t0,
        };

        // Increasing-pct points with virtual start/end points exactly on the line.
        let mut points: Vec<GridPoint> = Vec::with_capacity(lap_frames.len() + 2);
        points.push(GridPoint { pct: 0.0, time: 0.0, frame: lap_frames[0], yaw_unwrapped: 0.0 });
        for f in lap_frames {
            if f.pct > points.last().map(|p| p.pct).unwrap_or(0.0) && f.pct < 1.0 {
                let time = (f.time - t0).clamp(0.0, lap_time_s);
                points.push(GridPoint { pct: f.pct, time, frame: *f, yaw_unwrapped: 0.0 });
            }
        }
        points.push(GridPoint { pct: 1.0, time: lap_time_s, frame: lap_frames[lap_frames.len() - 1], yaw_unwrapped: 0.0 });

        // Unwrap heading so interpolation never swings the long way round.
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

        let sectors = if complete && track.sector_pcts.len() > 1 {
            let mut bounds: Vec<f64> = track.sector_pcts.iter().copied().filter(|p| *p > 0.0 && *p < 1.0).collect();
            bounds.push(1.0);
            let mut sectors = Vec::with_capacity(bounds.len());
            let mut prev_time = 0.0;
            for b in bounds {
                let t = if b >= 1.0 { lap_time_s } else { time_at(&points, b).unwrap_or(prev_time) };
                sectors.push(t - prev_time);
                prev_time = t;
            }
            sectors
        } else {
            Vec::new()
        };

        laps.push(LapMetrics {
            lap_number,
            lap_time_s,
            is_complete: complete,
            sectors,
            avg_speed_kph: if complete && track.length_m > 0.0 {
                Some(track.length_m / lap_time_s * 3.6)
            } else {
                avg_speed_kph
            },
            tyre_temp_avg_c,
            tyre_temp_delta_c,
        });

        if complete {
            resampled.push(resample(&points, lap_number, n));
        }
    }

    // Without a working lap counter, number laps 1, 2, 3… in the order driven.
    if frames.iter().all(|f| f.lap <= 0) {
        for (i, lap) in laps.iter_mut().enumerate() {
            let old = lap.lap_number;
            lap.lap_number = i as i32 + 1;
            if let Some(r) = resampled.iter_mut().find(|r| r.trace.lap_number == old) {
                r.trace.lap_number = lap.lap_number;
            }
        }
    }

    // Build the outline from the fastest clean lap.
    let best = resampled
        .iter()
        .min_by(|a, b| a.trace.time_s[n].partial_cmp(&b.trace.time_s[n]).unwrap_or(std::cmp::Ordering::Equal));
    let (map_points, map_from_gps) = match best {
        Some(best) => {
            let has_gps = best.lat.iter().all(|v| v.is_finite() && *v != 0.0);
            if has_gps {
                (Some(outline_from_gps(&best.lat, &best.lon)), true)
            } else if best.yaw.iter().all(|v| v.is_finite()) {
                let length = if track.length_m > 0.0 {
                    track.length_m
                } else {
                    best.trace.speed_kph.iter().map(|v| *v as f64 / 3.6).sum::<f64>() / n as f64
                        * best.trace.time_s[n] as f64
                };
                (Some(outline_from_heading(&best.yaw, length)), false)
            } else {
                (None, false)
            }
        }
        None => (None, false),
    };

    let mut traces: Vec<LapTrace> = resampled.into_iter().map(|r| r.trace).collect();
    crate::handling::add_balance(&mut traces);

    RunData {
        laps,
        traces,
        map_points,
        map_from_gps,
    }
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
        let data = crate::ibt::read_ibt(Path::new(FIXTURE)).expect("read fixture");
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
        // Tyre temps are frozen on track in iRacing, so they're dropped.
        assert!(timed.iter().all(|lap| lap.tyre_temp_avg_c.is_none()));
    }

    #[test]
    fn fixture_live_map_matches_gps() {
        let data = crate::ibt::read_ibt(Path::new(FIXTURE)).expect("read fixture");
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
            let data = crate::ibt::read_ibt(&path).expect("read fixture");
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
        let data = crate::ibt::read_ibt(Path::new("tests/fixtures/watkinsglen-2021-fullcourse.ibt")).expect("read fixture");
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
        let data = crate::ibt::read_ibt(Path::new(&path)).unwrap();
        println!("{} ({}), {:.0} m, sectors {:?}", data.track.display_name, data.track.car, data.track.length_m, data.track.sector_pcts);
        for lap in build_run(&data.frames, &data.track).laps {
            println!("lap {:>3} {:>8.3}s timed={} sectors={:?}", lap.lap_number, lap.lap_time_s, lap.is_complete, lap.sectors);
        }
        let rms = heading_vs_gps_rms(&data.frames, &data.track);
        println!("heading vs GPS outline: {rms:.1} m RMS");
        assert!(rms < 30.0);
    }
}