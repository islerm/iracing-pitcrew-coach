
//! Comparing the player's laps with other cars', corner by corner. Other cars only come with
//! lap distance over time (and gear), so both sides are measured the same way from that: the
//! time through each corner, a speed trace derived from time over distance, where the car
//! starts slowing for the corner, its apex speed and its exit speed.

use serde::Serialize;

use super::{OpponentLap, Roster};
use crate::stats::median;
use crate::telemetry::trace::{turn_segments, LapMetrics, LapTrace, Turn};

/// How many of the class's quickest drivers (one lap each) make up the reference.
pub const REFERENCE_DRIVERS: usize = 3;
/// Smallest corner gap worth pointing out.
const MIN_GAP_S: f64 = 0.05;

/// Speed (kph) at each grid point from the time it took to cover ~±6 m around it. Smooths
/// the jitter in other cars' lap distance; used for the player's laps too, so the numbers
/// compare like for like.
pub fn derived_speed(time_s: &[f32], length_m: f64) -> Vec<f32> {
    let n = time_s.len().saturating_sub(1);
    if n == 0 || length_m <= 0.0 {
        return vec![0.0; time_s.len()];
    }
    let ds = length_m / n as f64;
    let k = ((6.0 / ds).round() as usize).max(2);
    (0..=n)
        .map(|i| {
            let (a, b) = (i.saturating_sub(k), (i + k).min(n));
            let dt = (time_s[b] - time_s[a]) as f64;
            if dt > 0.0 {
                ((b - a) as f64 * ds / dt * 3.6) as f32
            } else {
                0.0
            }
        })
        .collect()
}

/// `values` (on any grid) resampled onto `n + 1` points.
fn resample(values: &[f32], n: usize) -> Vec<f32> {
    let m = values.len().saturating_sub(1);
    if m == n || m == 0 {
        return values.to_vec();
    }
    (0..=n)
        .map(|i| {
            let x = i as f64 * m as f64 / n as f64;
            let j = (x.floor() as usize).min(m - 1);
            let f = (x - j as f64) as f32;
            values[j] + f * (values[j + 1] - values[j])
        })
        .collect()
}

/// One lap through one corner, from time, derived speed and gear only.
#[derive(Debug, Clone, Serialize)]
pub struct CornerPass {
    pub time_s: f64,
    pub apex_speed_kph: f64,
    /// Metres before the turn's apex marker where the car starts slowing for it (its last
    /// speed peak): roughly the braking point.
    pub slowing_at_m: f64,
    pub exit_speed_kph: f64,
    pub apex_gear: i8,
}

fn corner_pass(time_s: &[f32], speed: &[f32], gear: &[i8], segment: (f64, f64, f64), length_m: f64) -> CornerPass {
    let n = time_s.len() - 1;
    let idx = |pct: f64| ((pct * n as f64).round() as usize).min(n);
    let (a, apex, b) = (idx(segment.0), idx(segment.1), idx(segment.2));
    let ds = length_m / n as f64;
    let by_speed = |x: &usize, y: &usize| speed[*x].total_cmp(&speed[*y]);
    let min_idx = (a..=b).min_by(by_speed).unwrap_or(apex);
    let peak_idx = (a..=min_idx).max_by(by_speed).unwrap_or(a);
    CornerPass {
        time_s: (time_s[b] - time_s[a]) as f64,
        apex_speed_kph: speed[min_idx] as f64,
        slowing_at_m: (apex as f64 - peak_idx as f64) * ds,
        exit_speed_kph: speed[b] as f64,
        apex_gear: gear.get(min_idx).copied().unwrap_or(0),
    }
}

/// A lap the player is compared against.
#[derive(Debug, Clone, Serialize)]
pub struct Reference {
    pub car_idx: i32,
    pub name: String,
    pub irating: i64,
    pub lap_number: i32,
    pub lap_time_s: f64,
    /// False when it comes from an earlier session at this track (the library in the store).
    pub this_session: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CornerGap {
    pub turn: String,
    /// Time the player loses here against the reference (negative = gains).
    pub gap_s: f64,
    pub you: CornerPass,
    /// The reference laps' median pass.
    pub them: CornerPass,
    /// What the quicker cars do differently, in words; empty when nothing stands out.
    pub why: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FieldComparison {
    pub your_lap: i32,
    pub your_time_s: f64,
    pub references: Vec<Reference>,
    /// Every corner, in track order.
    pub corners: Vec<CornerGap>,
    /// The corners losing the most, worst first, as sentences.
    pub notes: Vec<String>,
}

/// Whether two laps' track wetness is close enough to compare (as `LapMetrics::same_conditions`).
fn similar_wetness(a: Option<u8>, b: Option<u8>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.abs_diff(b) <= 1,
        _ => true,
    }
}

/// The reference laps: the best valid lap of each of the quickest drivers in the player's
/// class, in conditions like `yours`, quickest first.
pub fn pick_references<'a>(laps: &'a [OpponentLap], roster: &Roster, yours: &LapMetrics) -> Vec<(&'a OpponentLap, Reference)> {
    let mut best: Vec<&OpponentLap> = Vec::new();
    for lap in laps.iter().filter(|l| {
        l.valid && l.car_idx != roster.player_car_idx && roster.in_player_class(l.car_idx) && similar_wetness(l.track_wetness, yours.track_wetness)
    }) {
        match best.iter_mut().find(|b| b.car_idx == lap.car_idx) {
            Some(b) if lap.lap_time_s < b.lap_time_s => *b = lap,
            Some(_) => {}
            None => best.push(lap),
        }
    }
    best.sort_by(|a, b| a.lap_time_s.total_cmp(&b.lap_time_s));
    best.truncate(REFERENCE_DRIVERS);
    best.into_iter()
        .map(|lap| {
            let driver = roster.driver(lap.car_idx);
            let reference = Reference {
                car_idx: lap.car_idx,
                name: driver.map(|d| d.name.clone()).unwrap_or_default(),
                irating: driver.map(|d| d.irating).unwrap_or_default(),
                lap_number: lap.lap_number,
                lap_time_s: lap.lap_time_s,
                this_session: true,
            };
            (lap, reference)
        })
        .collect()
}

/// The player's lap against the reference laps, corner by corner. `None` without references,
/// turns or a usable lap.
pub fn compare(yours: &LapTrace, your_metrics: &LapMetrics, references: &[(&OpponentLap, Reference)], turns: &[Turn], length_m: f64) -> Option<FieldComparison> {
    if references.is_empty() || turns.is_empty() || length_m <= 0.0 || yours.time_s.len() < 10 {
        return None;
    }
    let n = yours.time_s.len() - 1;
    let my_speed = derived_speed(&yours.time_s, length_m);
    let theirs: Vec<(Vec<f32>, Vec<f32>, Vec<i8>)> = references
        .iter()
        .map(|(lap, _)| {
            let time = resample(&lap.time_s, n);
            let gear_f: Vec<f32> = lap.gear.iter().map(|&g| g as f32).collect();
            let gear = resample(&gear_f, n).into_iter().map(|g| g.round() as i8).collect();
            let speed = derived_speed(&time, length_m);
            (time, speed, gear)
        })
        .collect();

    let corners: Vec<CornerGap> = turn_segments(turns)
        .into_iter()
        .zip(turns)
        .map(|(segment, turn)| {
            let you = corner_pass(&yours.time_s, &my_speed, &yours.gear, segment, length_m);
            let passes: Vec<CornerPass> = theirs.iter().map(|(t, s, g)| corner_pass(t, s, g, segment, length_m)).collect();
            let them = median_pass(&passes);
            CornerGap { turn: turn.label.clone(), gap_s: you.time_s - them.time_s, why: why(&you, &them), you, them }
        })
        .collect();

    let mut worst: Vec<&CornerGap> = corners.iter().filter(|c| c.gap_s >= MIN_GAP_S).collect();
    worst.sort_by(|a, b| b.gap_s.total_cmp(&a.gap_s));
    let notes = worst
        .iter()
        .take(3)
        .map(|c| {
            let mut note = format!("Turn {}: {:.2}s to the quickest in your class", c.turn, c.gap_s);
            if !c.why.is_empty() {
                note += &format!(", who {}", c.why);
            }
            note + "."
        })
        .collect();

    Some(FieldComparison {
        your_lap: yours.lap_number,
        your_time_s: your_metrics.lap_time_s,
        references: references.iter().map(|(_, r)| r.clone()).collect(),
        corners,
        notes,
    })
}

fn median_pass(passes: &[CornerPass]) -> CornerPass {
    let med = |f: fn(&CornerPass) -> f64| median(&mut passes.iter().map(f).collect::<Vec<_>>()).unwrap_or(0.0);
    let mut gears: Vec<i8> = passes.iter().map(|p| p.apex_gear).collect();
    gears.sort_unstable();
    CornerPass {
        time_s: med(|p| p.time_s),
        apex_speed_kph: med(|p| p.apex_speed_kph),
        slowing_at_m: med(|p| p.slowing_at_m),
        exit_speed_kph: med(|p| p.exit_speed_kph),
        apex_gear: gears.get(gears.len() / 2).copied().unwrap_or(0),
    }
}

/// What the reference does differently from the player through a corner, from their side:
/// "start slowing 12 m later and carry 6 kph more at the apex". The two biggest differences.
fn why(you: &CornerPass, them: &CornerPass) -> String {
    let mut parts: Vec<(f64, String)> = Vec::new();
    let later = you.slowing_at_m - them.slowing_at_m;
    if later >= 8.0 {
        parts.push((later / 8.0, format!("start slowing {later:.0} m later")));
    }
    let apex = them.apex_speed_kph - you.apex_speed_kph;
    if apex >= 2.0 {
        parts.push((apex / 2.0, format!("carry {apex:.0} kph more at the apex")));
    }
    let exit = them.exit_speed_kph - you.exit_speed_kph;
    if exit >= 3.0 {
        parts.push((exit / 3.0, format!("come out {exit:.0} kph faster")));
    }
    if them.apex_gear > 0 && you.apex_gear > 0 && them.apex_gear != you.apex_gear {
        parts.push((1.0, format!("use gear {} where you use {}", them.apex_gear, you.apex_gear)));
    }
    parts.sort_by(|a, b| b.0.total_cmp(&a.0));
    parts.into_iter().take(2).map(|(_, p)| p).collect::<Vec<_>>().join(" and ")
}

/// The player's fastest complete lap that stayed on track and has a trace: the lap compared
/// with the field.
pub fn your_best<'a>(laps: &'a [LapMetrics], traces: &'a [LapTrace]) -> Option<(&'a LapTrace, &'a LapMetrics)> {
    laps.iter()
        .filter(|l| l.is_complete && l.off_track_pcts.is_empty())
        .filter_map(|l| Some((traces.iter().find(|t| t.lap_number == l.lap_number)?, l)))
        .min_by(|a, b| a.1.lap_time_s.total_cmp(&b.1.lap_time_s))
}

/// References from the store's library (laps by drivers seen in earlier sessions at this
/// track, in this class): the best valid lap per driver in conditions like `yours`, quickest
/// first. Used when nobody else in the current session has a usable lap.
pub fn library_references<'a>(library: &'a [(OpponentLap, super::FieldDriver)], yours: &LapMetrics) -> Vec<(&'a OpponentLap, Reference)> {
    let mut best: Vec<&(OpponentLap, super::FieldDriver)> = Vec::new();
    for entry in library.iter().filter(|(l, _)| l.valid && similar_wetness(l.track_wetness, yours.track_wetness)) {
        let key = |e: &(OpponentLap, super::FieldDriver)| if e.1.user_id > 0 { e.1.user_id.to_string() } else { e.1.name.clone() };
        match best.iter_mut().find(|b| key(b) == key(entry)) {
            Some(b) if entry.0.lap_time_s < b.0.lap_time_s => *b = entry,
            Some(_) => {}
            None => best.push(entry),
        }
    }
    best.sort_by(|a, b| a.0.lap_time_s.total_cmp(&b.0.lap_time_s));
    best.truncate(REFERENCE_DRIVERS);
    best.into_iter()
        .map(|(lap, driver)| {
            let reference = Reference {
                car_idx: lap.car_idx,
                name: driver.name.clone(),
                irating: driver.irating,
                lap_number: lap.lap_number,
                lap_time_s: lap.lap_time_s,
                this_session: false,
            };
            (lap, reference)
        })
        .collect()
}

/// One sentence for the pit debrief, light on numbers: the corner where the class's quickest
/// gain the most, and how.
pub fn radio_line(cmp: &FieldComparison) -> Option<String> {
    let worst = cmp.corners.iter().filter(|c| c.gap_s >= MIN_GAP_S).max_by(|a, b| a.gap_s.total_cmp(&b.gap_s))?;
    let how = radio_how(&worst.you, &worst.them);
    Some(match how {
        Some(how) => format!("Against the field, the quickest in your class gain most on you in Turn {}: they {how}.", worst.turn),
        None => format!("Against the field, the quickest in your class gain most on you in Turn {}.", worst.turn),
    })
}

/// The biggest difference, without numbers.
fn radio_how(you: &CornerPass, them: &CornerPass) -> Option<&'static str> {
    let options = [
        ((you.slowing_at_m - them.slowing_at_m) / 8.0, "brake later"),
        ((them.apex_speed_kph - you.apex_speed_kph) / 2.0, "carry more speed through the apex"),
        ((them.exit_speed_kph - you.exit_speed_kph) / 3.0, "get a better exit"),
    ];
    options.into_iter().filter(|o| o.0 >= 1.0).max_by(|a, b| a.0.total_cmp(&b.0)).map(|o| o.1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::field::FieldDriver;

    const LENGTH: f64 = 1000.0;
    const N: usize = 1000;

    /// Time at each metre for a lap at `v(x)` m/s.
    fn lap_times(v: impl Fn(f64) -> f64) -> Vec<f32> {
        let mut t = 0.0;
        let mut out = vec![0.0f32];
        for i in 1..=N {
            t += (LENGTH / N as f64) / v(i as f64 - 0.5);
            out.push(t as f32);
        }
        out
    }

    /// A straight, then a corner at 500 m slowed to `apex` m/s from `brake_at` m.
    fn profile(apex: f64, brake_at: f64) -> impl Fn(f64) -> f64 {
        move |x: f64| {
            if x < brake_at {
                50.0
            } else if x < 500.0 {
                50.0 - (50.0 - apex) * (x - brake_at) / (500.0 - brake_at)
            } else {
                (apex + (x - 500.0) * 0.1).min(50.0)
            }
        }
    }

    fn trace(time_s: Vec<f32>) -> LapTrace {
        LapTrace {
            lap_number: 4,
            speed_kph: vec![0.0; time_s.len()],
            throttle: vec![0.0; time_s.len()],
            brake: vec![0.0; time_s.len()],
            gear: vec![3; time_s.len()],
            steer_deg: vec![0.0; time_s.len()],
            lat_g: Vec::new(),
            long_g: Vec::new(),
            yaw_rate_dps: Vec::new(),
            abs: Vec::new(),
            balance_deg: Vec::new(),
            rpm: Vec::new(),
            alt_m: Vec::new(),
            time_s,
        }
    }

    fn metrics(lap_time_s: f64) -> LapMetrics {
        LapMetrics {
            lap_number: 4,
            lap_time_s,
            is_complete: true,
            sectors: Vec::new(),
            avg_speed_kph: None,
            off_track_pcts: Vec::new(),
            incidents: None,
            air_temp_c: None,
            track_temp_c: None,
            track_wetness: Some(1),
            top_speed_kph: None,
            full_throttle_pct: None,
            braking_pct: None,
        }
    }

    fn roster() -> Roster {
        let driver = |car_idx: i32, class_id: i64, is_player: bool| FieldDriver { car_idx, name: format!("Driver {car_idx}"), class_id, irating: 2000, is_player, ..Default::default() };
        Roster { player_car_idx: 0, drivers: vec![driver(0, 1, true), driver(1, 1, false), driver(2, 1, false), driver(3, 2, false)], ..Default::default() }
    }

    fn opponent(car_idx: i32, time_s: Vec<f32>, wetness: u8) -> OpponentLap {
        OpponentLap { car_idx, lap_number: 3, lap_time_s: *time_s.last().unwrap() as f64, valid: true, track_wetness: Some(wetness), gear: vec![3; time_s.len()], time_s }
    }

    #[test]
    fn derived_speed_matches_a_steady_car() {
        let t = lap_times(|_| 40.0);
        let v = derived_speed(&t, LENGTH);
        assert!(v.iter().all(|&s| (s - 144.0).abs() < 0.5), "{:?}", &v[..5]);
    }

    #[test]
    fn references_are_class_rivals_in_similar_conditions_quickest_first() {
        let fast = opponent(1, lap_times(profile(25.0, 450.0)), 1);
        let slow = opponent(2, lap_times(profile(20.0, 400.0)), 2);
        let wet = opponent(2, lap_times(profile(30.0, 470.0)), 5);
        let other_class = opponent(3, lap_times(profile(30.0, 470.0)), 1);
        let laps = vec![slow, fast, wet, other_class];
        let refs = pick_references(&laps, &roster(), &metrics(30.0));
        let cars: Vec<i32> = refs.iter().map(|(l, _)| l.car_idx).collect();
        assert_eq!(cars, vec![1, 2]);
        assert_eq!(refs[0].1.name, "Driver 1");
    }

    #[test]
    fn finds_where_the_quicker_car_gains_and_why() {
        // The quicker car brakes 40 m later and carries 5 m/s (18 kph) more at the apex.
        let mine = trace(lap_times(profile(20.0, 400.0)));
        let laps = vec![opponent(1, lap_times(profile(25.0, 440.0)), 1)];
        let refs = pick_references(&laps, &roster(), &metrics(30.0));
        let turns = vec![Turn { label: "1".into(), pct: 0.5 }];
        let cmp = compare(&mine, &metrics(30.0), &refs, &turns, LENGTH).expect("comparison");

        let corner = &cmp.corners[0];
        assert!(corner.gap_s > 0.3, "{}", corner.gap_s);
        assert!((corner.them.apex_speed_kph - corner.you.apex_speed_kph - 18.0).abs() < 3.0, "{corner:?}");
        assert!(corner.why.contains("kph more at the apex") && corner.why.contains("start slowing"), "{}", corner.why);
        assert!(cmp.notes[0].starts_with("Turn 1: "), "{:?}", cmp.notes);
        let line = radio_line(&cmp).expect("radio line");
        assert!(line.contains("in Turn 1: they "), "{line}");
        // Light on numbers for the radio: only the turn's.
        assert_eq!(line.chars().filter(|c| c.is_ascii_digit()).count(), 1, "{line}");
    }

    #[test]
    fn no_references_no_comparison() {
        let mine = trace(lap_times(|_| 40.0));
        let turns = vec![Turn { label: "1".into(), pct: 0.5 }];
        assert!(compare(&mine, &metrics(25.0), &[], &turns, LENGTH).is_none());
    }
}
