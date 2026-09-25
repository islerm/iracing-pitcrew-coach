//! Car-handling analysis on resampled lap traces: understeer/oversteer balance, grip use,
//! ABS and coasting per corner, and plain-language notes for the coach.

use crate::trace::{LapTrace, Turn};

/// Combined acceleration counted as "at the limit" is the session's 98th percentile, so a
/// single kerb strike doesn't set the reference.
const PEAK_PERCENTILE: f64 = 0.98;
/// Below this speed, steering and yaw are dominated by low-speed geometry (pit lane, hairpin
/// crawl after a spin) and aren't used for calibration.
const MIN_SPEED_MS: f64 = 15.0;
/// Balance is only reported where the steering the car's rotation needs is at least this
/// many degrees at the wheel; on straights the ratio is all noise.
const MIN_EXPECTED_STEER_DEG: f64 = 3.0;

fn percentile(values: &mut [f64], p: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Some(values[((values.len() - 1) as f64 * p).round() as usize])
}

/// Adds `balance_deg` to every trace that has yaw rate.
///
/// Without the car's steering ratio and wheelbase, the steering needed for a given path
/// curvature (yaw rate / speed) is learned from the session itself: a least-squares fit
/// over moderate-cornering samples, where the tyres are still in their linear range.
/// Balance is then the steering used beyond that, signed so positive means more lock than
/// the car's rotation called for (understeer) and negative means less, or counter-steer
/// (oversteer). Near the limit some positive balance is normal for most cars, so compare
/// the same corner across laps rather than reading it as an absolute.
pub fn add_balance(traces: &mut [LapTrace]) {
    let usable = |t: &LapTrace| !t.yaw_rate_dps.is_empty() && t.yaw_rate_dps.len() == t.steer_deg.len();

    // Lateral acceleration implied by the path, a = r·v, which unlike LatAccel isn't
    // affected by banking.
    let mut samples: Vec<(f64, f64, f64)> = Vec::new(); // (curvature rad/m, steer deg, a g)
    for trace in traces.iter().filter(|t| usable(t)) {
        for j in 0..trace.steer_deg.len() {
            let v = trace.speed_kph[j] as f64 / 3.6;
            let r = (trace.yaw_rate_dps[j] as f64).to_radians();
            let steer = trace.steer_deg[j] as f64;
            if v > MIN_SPEED_MS && steer.is_finite() && r.is_finite() {
                samples.push((r / v, steer, (r * v).abs() / 9.80665));
            }
        }
    }
    let mut accel: Vec<f64> = samples.iter().map(|s| s.2).collect();
    let Some(peak) = percentile(&mut accel, PEAK_PERCENTILE) else { return };
    let (mut sxy, mut sxx, mut count) = (0.0, 0.0, 0);
    for &(curvature, steer, a) in &samples {
        if a > 0.1 && a < 0.5 * peak {
            sxy += curvature * steer;
            sxx += curvature * curvature;
            count += 1;
        }
    }
    if count < 200 || sxx <= 0.0 {
        return;
    }
    let steer_per_curvature = sxy / sxx;

    for trace in traces.iter_mut().filter(|t| usable(t)) {
        trace.balance_deg = (0..trace.steer_deg.len())
            .map(|j| {
                let v = trace.speed_kph[j] as f64 / 3.6;
                if v < MIN_SPEED_MS {
                    return 0.0;
                }
                let expected = steer_per_curvature * (trace.yaw_rate_dps[j] as f64).to_radians() / v;
                if expected.abs() < MIN_EXPECTED_STEER_DEG {
                    return 0.0;
                }
                let excess = (trace.steer_deg[j] as f64 - expected) * expected.signum();
                ((excess * 10.0).round() / 10.0) as f32
            })
            .collect();
    }
}

/// How one lap drove one corner.
#[derive(Debug, Clone)]
pub struct CornerStats {
    pub time_s: f64,
    pub min_speed_kph: f64,
    /// Distance from the segment start to the first brake application before the apex.
    pub brake_point_m: Option<f64>,
    /// Mean combined g while cornering or braking, as % of the session's peak.
    pub grip_pct: Option<f64>,
    /// % of braking samples with ABS active.
    pub abs_pct: Option<f64>,
    /// Mean balance where the car is most loaded laterally.
    pub balance_deg: Option<f64>,
    /// Distance with neither pedal pressed.
    pub coast_m: f64,
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

/// Session peak combined acceleration (g) across all traces, if any have accelerometer data.
pub fn peak_combined_g(traces: &[LapTrace]) -> Option<f64> {
    let mut all: Vec<f64> = traces
        .iter()
        .flat_map(|t| t.lat_g.iter().zip(&t.long_g).map(|(a, b)| (*a as f64).hypot(*b as f64)))
        .collect();
    percentile(&mut all, PEAK_PERCENTILE).filter(|p| *p > 0.3)
}

pub fn corner_stats(trace: &LapTrace, segment: (f64, f64, f64), length_m: f64, peak_g: Option<f64>) -> CornerStats {
    let n = trace.time_s.len() - 1;
    let idx = |pct: f64| ((pct * n as f64).round() as usize).min(n);
    let (a, apex, b) = (idx(segment.0), idx(segment.1), idx(segment.2));
    let ds = length_m / n as f64;
    let range = a..=b;

    let min_speed_kph = range.clone().map(|j| trace.speed_kph[j] as f64).fold(f64::INFINITY, f64::min);
    let brake_point_m = (a..=apex).find(|&j| trace.brake[j] >= 10.0).map(|j| (j - a) as f64 * ds);
    let coast_m = range.clone().filter(|&j| trace.throttle[j] < 5.0 && trace.brake[j] < 5.0).count() as f64 * ds;

    let has_g = !trace.lat_g.is_empty();
    let combined = |j: usize| (trace.lat_g[j] as f64).hypot(trace.long_g[j] as f64);
    let grip_pct = peak_g.filter(|_| has_g).and_then(|peak| {
        let working: Vec<f64> = range.clone().filter(|&j| trace.brake[j] >= 5.0 || (trace.lat_g[j] as f64).abs() > 0.3 * peak).map(combined).collect();
        (!working.is_empty()).then(|| working.iter().sum::<f64>() / working.len() as f64 / peak * 100.0)
    });

    let abs_pct = (!trace.abs.is_empty()).then(|| {
        let braking: Vec<usize> = range.clone().filter(|&j| trace.brake[j] >= 10.0).collect();
        if braking.is_empty() {
            0.0
        } else {
            braking.iter().filter(|&&j| trace.abs[j] != 0).count() as f64 / braking.len() as f64 * 100.0
        }
    });

    // Mid-corner: the most laterally loaded third of the segment.
    let balance_deg = (!trace.balance_deg.is_empty()).then(|| {
        let load = |j: usize| if has_g { (trace.lat_g[j] as f64).abs() } else { (trace.yaw_rate_dps[j] as f64 * trace.speed_kph[j] as f64).abs() };
        let mut loaded: Vec<usize> = range.clone().collect();
        loaded.sort_by(|&x, &y| load(y).partial_cmp(&load(x)).unwrap_or(std::cmp::Ordering::Equal));
        loaded.truncate((loaded.len() / 3).max(1));
        loaded.iter().map(|&j| trace.balance_deg[j] as f64).sum::<f64>() / loaded.len() as f64
    });

    CornerStats {
        time_s: (trace.time_s[b] - trace.time_s[a]) as f64,
        min_speed_kph,
        brake_point_m,
        grip_pct,
        abs_pct,
        balance_deg,
        coast_m,
    }
}

/// Where the fastest lap lost the most time against the best the driver managed in each
/// corner, and what was different, as short notes for the coach and the "next time" list.
pub fn corner_notes(traces: &[LapTrace], turns: &[Turn], length_m: f64) -> Vec<String> {
    if traces.len() < 2 || turns.is_empty() || length_m <= 0.0 {
        return Vec::new();
    }
    let lap_time = |t: &LapTrace| *t.time_s.last().unwrap_or(&f32::INFINITY);
    let Some(fastest) = traces.iter().min_by(|a, b| lap_time(a).partial_cmp(&lap_time(b)).unwrap_or(std::cmp::Ordering::Equal)) else {
        return Vec::new();
    };
    let peak_g = peak_combined_g(traces);

    struct Loss {
        label: String,
        loss: f64,
        best_lap: i32,
        on_fastest: CornerStats,
        on_best: CornerStats,
    }
    let mut losses: Vec<Loss> = turn_segments(turns)
        .into_iter()
        .zip(turns)
        .filter_map(|(segment, turn)| {
            let on_fastest = corner_stats(fastest, segment, length_m, peak_g);
            let (best_trace, on_best) = traces
                .iter()
                .map(|t| (t, corner_stats(t, segment, length_m, peak_g)))
                .min_by(|a, b| a.1.time_s.partial_cmp(&b.1.time_s).unwrap_or(std::cmp::Ordering::Equal))?;
            let loss = on_fastest.time_s - on_best.time_s;
            (loss >= 0.03 && best_trace.lap_number != fastest.lap_number).then(|| Loss {
                label: turn.label.clone(),
                loss,
                best_lap: best_trace.lap_number,
                on_fastest,
                on_best,
            })
        })
        .collect();
    losses.sort_by(|a, b| b.loss.partial_cmp(&a.loss).unwrap_or(std::cmp::Ordering::Equal));

    losses
        .into_iter()
        .take(3)
        .map(|l| {
            let (f, b) = (&l.on_fastest, &l.on_best);
            let mut why = Vec::new();
            let mut advice = None;
            if let (Some(bf), Some(bb)) = (f.brake_point_m, b.brake_point_m) {
                if bf - bb < -8.0 {
                    why.push(format!("braked {:.0} m earlier", bb - bf));
                }
            }
            let speed_diff = f.min_speed_kph - b.min_speed_kph;
            if speed_diff <= -2.0 {
                why.push(format!("minimum speed {:.0} kph lower", -speed_diff));
            }
            if let (Some(bf), Some(bb)) = (f.balance_deg, b.balance_deg) {
                // Balance grows with cornering load, so small differences in a fast, heavily
                // loaded corner are noise: require a gap relative to the better lap's value.
                let threshold = (0.25 * bb.abs()).max(3.0);
                if bf - bb >= threshold {
                    why.push(format!("more understeer mid-corner ({:+.1}° extra steering lock)", bf - bb));
                    advice.get_or_insert("carry a little brake closer to the apex to keep the front loaded, and let the car rotate before adding more lock");
                } else if bf - bb <= -threshold {
                    why.push(format!("more oversteer mid-corner ({:.1}° less lock than the car's rotation needed)", bb - bf));
                    advice.get_or_insert("release the brake more smoothly and pick up the throttle later and more gently to settle the rear");
                }
            }
            if let (Some(af), Some(ab)) = (f.abs_pct, b.abs_pct) {
                if af - ab >= 15.0 {
                    why.push(format!("ABS active for {af:.0}% of braking vs {ab:.0}%"));
                    advice.get_or_insert("brake with slightly less peak pressure so the ABS isn't doing the work");
                }
            }
            if let (Some(gf), Some(gb)) = (f.grip_pct, b.grip_pct) {
                if gb - gf >= 4.0 {
                    why.push(format!("used {gf:.0}% of the available grip vs {gb:.0}%"));
                    advice.get_or_insert("commit more: that lap shows the grip is there");
                }
            }
            if f.coast_m - b.coast_m >= 10.0 {
                why.push(format!("{:.0} m more coasting", f.coast_m - b.coast_m));
                advice.get_or_insert("close the gap between coming off the brake and getting back on the throttle");
            }
            let mut note = format!("Turn {}: {:.2}s slower on your fastest lap than your best through there (lap {})", l.label, l.loss, l.best_lap);
            if !why.is_empty() {
                note += &format!(": {}", why.join(", "));
            }
            note.push('.');
            if let Some(advice) = advice {
                note += &format!(" Next time, {advice}.");
            }
            note
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace::{build_run, detect_turns};
    use std::path::Path;

    #[test]
    fn fixture_traces_have_handling_channels() {
        let data = crate::ibt::read_ibt(Path::new("tests/fixtures/roadatlanta-full.ibt")).expect("read fixture");
        let run = build_run(&data.frames, &data.track);
        for trace in &run.traces {
            let n = trace.time_s.len();
            assert_eq!(trace.lat_g.len(), n);
            assert_eq!(trace.long_g.len(), n);
            assert_eq!(trace.yaw_rate_dps.len(), n);
            assert_eq!(trace.balance_deg.len(), n);
            // The fixture predates BrakeABSactive, so no ABS channel is invented.
            assert!(trace.abs.is_empty());
            // An LMP2 pulls well over 2 g somewhere on a lap at Road Atlanta.
            let peak = trace.lat_g.iter().map(|v| v.abs()).fold(0.0, f32::max);
            assert!(peak > 2.0 && peak < 6.0, "lap {} peak lateral {peak} g", trace.lap_number);
            // Balance is a correction around the car's own steering; it should stay small.
            let mut sorted: Vec<f32> = trace.balance_deg.iter().map(|v| v.abs()).collect();
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let p90 = sorted[sorted.len() * 9 / 10];
            assert!(p90 < 15.0, "lap {} balance p90 {p90}°", trace.lap_number);
        }

        let turns = detect_turns(run.map_points.as_ref().unwrap(), data.track.length_m);
        let peak = peak_combined_g(&run.traces).expect("peak g");
        for (segment, turn) in turn_segments(&turns).into_iter().zip(&turns) {
            let stats = corner_stats(&run.traces[0], segment, data.track.length_m, Some(peak));
            let grip = stats.grip_pct.expect("grip");
            assert!((20.0..=110.0).contains(&grip), "turn {} grip {grip}%", turn.label);
            assert!(stats.balance_deg.is_some());
        }
        let notes = corner_notes(&run.traces, &turns, data.track.length_m);
        assert!(notes.len() <= 3);
        assert!(notes.iter().all(|n| n.starts_with("Turn ")), "{notes:?}");
    }
}

