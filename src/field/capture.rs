
//! Rebuilding other cars' laps from live per-car channels (`CarIdxLapDistPct`, `CarIdxGear`,
//! `CarIdxOnPitRoad`, `CarIdxTrackSurface`, `CarIdxLastLapTime`), sampled with every telemetry
//! tick. Each car's lap is kept from line to line and then resampled onto the lap-distance
//! grid the player's traces use, so the two can be compared point by point.

use std::collections::HashMap;

use super::OpponentLap;

/// iRacing's `irsdk_TrkLoc` value for "on track".
const SURFACE_ON_TRACK: i8 = 3;
/// A pause in a car's data longer than this (s) breaks the lap: it can't be timed reliably.
const MAX_GAP_S: f64 = 1.0;
/// A jump in lap distance bigger than this (lap fraction) between two ticks means the data
/// for that car went stale (far away, or a reset), so the lap isn't used.
const MAX_STEP_PCT: f64 = 0.05;
/// How long after the line iRacing's own lap time may arrive and still replace ours (s).
const OFFICIAL_TIME_WINDOW_S: f64 = 4.0;
/// A car on track whose lap distance hasn't moved for this long (s) has stale data (or has
/// stopped): either way the lap isn't representative.
const MAX_STILL_S: f64 = 0.5;

/// One car's channels at one tick.
#[derive(Debug, Clone, Copy)]
pub struct CarSample {
    pub car_idx: i32,
    /// Lap fraction, or negative when the car isn't in the world.
    pub pct: f32,
    pub gear: i8,
    pub on_pit_road: bool,
    /// `irsdk_TrkLoc`: -1 not in world, 0 off track, 1 in pit stall, 2 approaching pits, 3 on track.
    pub surface: i8,
    /// iRacing's time for the car's previous lap (s; ≤ 0 when none).
    pub last_lap_time: f32,
}

#[derive(Debug, Clone, Copy)]
struct Point {
    time: f64,
    pct: f64,
    gear: i8,
}

/// A car's lap being driven: points since the line.
#[derive(Debug, Default)]
struct CarState {
    prev: Option<Point>,
    /// Started at a line crossing (the lap the car was on when recording began isn't used).
    points: Vec<Point>,
    in_lap: bool,
    clean: bool,
    laps_done: i32,
    wettest: Option<u8>,
    /// When the lap distance last went up.
    moved_at: f64,
    last_official: f32,
    /// The car's latest finished lap (index into `laps`) and when it finished, so iRacing's
    /// own time for it can replace the interpolated one when it comes through.
    awaiting_official: Option<(usize, f64)>,
}

/// Collects other cars' laps while a live recording runs.
pub struct FieldRecorder {
    /// Grid intervals per lap (the player's traces have `n + 1` points).
    n: usize,
    cars: HashMap<i32, CarState>,
    laps: Vec<OpponentLap>,
}

impl FieldRecorder {
    pub fn new(n: usize) -> Self {
        Self { n: n.max(10), cars: HashMap::new(), laps: Vec::new() }
    }

    /// The laps finished so far.
    pub fn laps(&self) -> &[OpponentLap] {
        &self.laps
    }

    pub fn into_laps(self) -> Vec<OpponentLap> {
        self.laps
    }

    /// Feeds one tick: `time` is the session time, `wetness` the track's `TrackWetness`, and
    /// `cars` the cars to follow (the caller leaves out the player and other classes).
    pub fn push(&mut self, time: f64, wetness: Option<u8>, cars: impl IntoIterator<Item = CarSample>) {
        for car in cars {
            let state = self.cars.entry(car.car_idx).or_default();
            Self::push_car(&mut self.laps, self.n, state, time, wetness, car);
        }
    }

    fn push_car(laps: &mut Vec<OpponentLap>, n: usize, state: &mut CarState, time: f64, wetness: Option<u8>, car: CarSample) {
        // iRacing's time for the lap just finished, once it's in.
        if car.last_lap_time > 0.0 && car.last_lap_time != state.last_official {
            state.last_official = car.last_lap_time;
            if let Some((i, finished_at)) = state.awaiting_official.take() {
                let official = car.last_lap_time as f64;
                if time - finished_at <= OFFICIAL_TIME_WINDOW_S && (official - laps[i].lap_time_s).abs() < 0.25 {
                    laps[i].lap_time_s = official;
                }
            }
        }

        if car.pct < 0.0 || !car.pct.is_finite() {
            // Out of the world (garage, disconnected): whatever lap was going is lost.
            state.prev = None;
            state.in_lap = false;
            return;
        }
        let point = Point { time, pct: car.pct as f64, gear: car.gear };
        let Some(prev) = state.prev.replace(point) else { return };
        if point.time <= prev.time {
            return;
        }

        let crossed = prev.pct > 0.9 && point.pct < 0.1;
        if crossed {
            // Where between the two ticks the line was crossed.
            let span = (1.0 - prev.pct) + point.pct;
            let f = if span > 0.0 { (1.0 - prev.pct) / span } else { 0.0 };
            let t_line = prev.time + f * (point.time - prev.time);
            if state.in_lap {
                let end = Point { time: t_line, pct: 1.0, gear: prev.gear };
                state.points.push(end);
                if let Some(lap) = finish_lap(state, n, car.car_idx) {
                    state.awaiting_official = lap.valid.then_some((laps.len(), time));
                    laps.push(lap);
                }
            }
            state.points.clear();
            state.points.push(Point { time: t_line, pct: 0.0, gear: point.gear });
            state.points.push(point);
            state.in_lap = true;
            state.clean = true;
            state.moved_at = time;
            state.wettest = wetness;
            return;
        }
        if !state.in_lap {
            return;
        }

        let step = point.pct - prev.pct;
        if step > 0.0 {
            state.moved_at = time;
        }
        let stale = time - state.moved_at > MAX_STILL_S;
        let broken = stale || point.time - prev.time > MAX_GAP_S || step < -0.001 || step > MAX_STEP_PCT;
        let off = car.on_pit_road || car.surface != SURFACE_ON_TRACK;
        if broken || off {
            state.clean = false;
        }
        state.wettest = match (state.wettest, wetness) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
        if step > 0.0 {
            state.points.push(point);
        }
    }
}

/// The lap in `state.points` (from the line at pct 0 to the line at pct 1) resampled onto
/// `n + 1` grid points.
fn finish_lap(state: &mut CarState, n: usize, car_idx: i32) -> Option<OpponentLap> {
    state.laps_done += 1;
    let points = &state.points;
    let (first, last) = (points.first()?, points.last()?);
    let lap_time_s = last.time - first.time;
    if lap_time_s <= 0.0 || points.len() < 10 {
        return None;
    }

    let mut time_s = Vec::with_capacity(n + 1);
    let mut gear = Vec::with_capacity(n + 1);
    let mut j = 0;
    for i in 0..=n {
        let pct = i as f64 / n as f64;
        while j + 1 < points.len() - 1 && points[j + 1].pct < pct {
            j += 1;
        }
        let (a, b) = (points[j], points[(j + 1).min(points.len() - 1)]);
        let f = if b.pct > a.pct { ((pct - a.pct) / (b.pct - a.pct)).clamp(0.0, 1.0) } else { 0.0 };
        time_s.push((a.time + f * (b.time - a.time) - first.time) as f32);
        gear.push(if f < 0.5 { a.gear } else { b.gear });
    }

    Some(OpponentLap {
        car_idx,
        lap_number: state.laps_done,
        lap_time_s,
        valid: state.clean,
        track_wetness: state.wettest,
        time_s,
        gear,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(car_idx: i32, pct: f64) -> CarSample {
        CarSample { car_idx, pct: pct.rem_euclid(1.0) as f32, gear: 3, on_pit_road: false, surface: SURFACE_ON_TRACK, last_lap_time: 0.0 }
    }

    /// A car lapping at a steady `lap_s` seconds from `start` pct, ticked at 60 Hz for `secs`.
    fn drive(rec: &mut FieldRecorder, car_idx: i32, lap_s: f64, start: f64, secs: f64, mut tweak: impl FnMut(f64, &mut CarSample)) {
        let ticks = (secs * 60.0) as usize;
        for k in 0..ticks {
            let t = k as f64 / 60.0;
            let mut s = sample(car_idx, start + t / lap_s);
            tweak(t, &mut s);
            rec.push(t, Some(1), [s]);
        }
    }

    #[test]
    fn steady_car_gives_timed_laps_on_the_grid() {
        let mut rec = FieldRecorder::new(100);
        drive(&mut rec, 7, 60.0, 0.5, 200.0, |_, _| {});
        // Laps start at the first crossing (t = 30 s): two whole laps by t = 150 s.
        let laps = rec.laps();
        assert_eq!(laps.len(), 2, "{:?}", laps.iter().map(|l| l.lap_time_s).collect::<Vec<_>>());
        for lap in laps {
            assert_eq!(lap.car_idx, 7);
            assert!(lap.valid);
            assert!((lap.lap_time_s - 60.0).abs() < 0.05, "{}", lap.lap_time_s);
            assert_eq!(lap.time_s.len(), 101);
            assert!((lap.time_s[50] - 30.0).abs() < 0.05, "{}", lap.time_s[50]);
            assert_eq!(lap.track_wetness, Some(1));
        }
    }

    #[test]
    fn pit_lane_and_stale_data_invalidate_the_lap() {
        let mut rec = FieldRecorder::new(100);
        // In the pit lane for a moment during the first full lap.
        drive(&mut rec, 3, 60.0, 0.5, 100.0, |t, s| s.on_pit_road = (50.0..51.0).contains(&t));
        assert!(!rec.laps()[0].valid);

        let mut rec = FieldRecorder::new(100);
        // Data frozen for two seconds (car far away), then catching up.
        drive(&mut rec, 3, 60.0, 0.5, 100.0, |t, s| {
            if (50.0..52.0).contains(&t) {
                s.pct = ((0.5 + 50.0 / 60.0) as f32).rem_euclid(1.0);
            }
        });
        assert!(!rec.laps()[0].valid);
    }

    #[test]
    fn official_lap_time_replaces_ours_when_close() {
        let mut rec = FieldRecorder::new(100);
        drive(&mut rec, 5, 60.0, 0.5, 95.0, |t, s| {
            if t > 91.0 {
                s.last_lap_time = 60.012;
            }
        });
        assert_eq!(rec.laps().len(), 1);
        assert!((rec.laps()[0].lap_time_s - 60.012).abs() < 1e-3);
    }

    #[test]
    fn cars_out_of_the_world_restart_cleanly() {
        let mut rec = FieldRecorder::new(100);
        drive(&mut rec, 9, 60.0, 0.5, 100.0, |t, s| {
            if (40.0..45.0).contains(&t) {
                s.pct = -1.0;
            }
        });
        // The lap interrupted by the disconnect is never finished.
        assert!(rec.laps().is_empty());
    }
}
