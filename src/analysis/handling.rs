//! Car-handling analysis on resampled lap traces: understeer/oversteer balance, grip use,
//! ABS and coasting per corner, and plain-language notes for the coach.

use crate::analysis::corner::{phase_stats, PhaseStats};
use crate::telemetry::balance::PEAK_PERCENTILE;
use crate::stats::percentile;
use crate::telemetry::trace::{turn_segments, LapTrace, Turn};

/// Session peak combined acceleration (g) across all traces, if any have accelerometer data.
pub fn peak_combined_g(traces: &[LapTrace]) -> Option<f64> {
    let mut all: Vec<f64> = traces
        .iter()
        .flat_map(|t| t.lat_g.iter().zip(&t.long_g).map(|(a, b)| (*a as f64).hypot(*b as f64)))
        .collect();
    percentile(&mut all, PEAK_PERCENTILE).filter(|p| *p > 0.3)
}

fn corner_time(s: &PhaseStats) -> f64 {
    s.entry_time_s + s.exit_time_s
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
        on_fastest: PhaseStats,
        on_best: PhaseStats,
    }
    let mut losses: Vec<Loss> = turn_segments(turns)
        .into_iter()
        .zip(turns)
        .filter_map(|(segment, turn)| {
            let on_fastest = phase_stats(fastest, segment, length_m, peak_g);
            let (best_trace, on_best) = traces
                .iter()
                .map(|t| (t, phase_stats(t, segment, length_m, peak_g)))
                .min_by(|a, b| corner_time(&a.1).partial_cmp(&corner_time(&b.1)).unwrap_or(std::cmp::Ordering::Equal))?;
            let loss = corner_time(&on_fastest) - corner_time(&on_best);
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
                // Metres before the apex: larger means earlier.
                if bf - bb > 8.0 {
                    why.push(format!("braked {:.0} m earlier", bf - bb));
                }
            }
            let speed_diff = f.min_speed_kph - b.min_speed_kph;
            if speed_diff <= -2.0 {
                why.push(format!("minimum speed {:.0} kph lower", -speed_diff));
            }
            // Whichever half of the corner differs more.
            let balance = [("entry", f.entry_balance_deg, b.entry_balance_deg), ("exit", f.exit_balance_deg, b.exit_balance_deg)]
                .into_iter()
                .filter_map(|(phase, f, b)| Some((phase, f?, b?)))
                .max_by(|x, y| (x.1 - x.2).abs().partial_cmp(&(y.1 - y.2).abs()).unwrap_or(std::cmp::Ordering::Equal));
            if let Some((phase, bf, bb)) = balance {
                // Balance grows with cornering load, so small differences in a fast, heavily
                // loaded corner are noise: require a gap relative to the better lap's value.
                let threshold = (0.25 * bb.abs()).max(3.0);
                if bf - bb >= threshold {
                    why.push(format!("more understeer on {phase} ({:+.1}° extra steering lock)", bf - bb));
                    advice.get_or_insert(if phase == "entry" {
                        "carry a little brake closer to the apex to keep the front loaded, and let the car rotate before adding more lock"
                    } else {
                        "wait for the car to finish rotating before you pick up the throttle, then unwind the lock as you add power"
                    });
                } else if bf - bb <= -threshold {
                    why.push(format!("more oversteer on {phase} ({:.1}° less lock than the car's rotation needed)", bb - bf));
                    advice.get_or_insert(if phase == "entry" {
                        "release the brake more smoothly so the rear stays settled as you turn in"
                    } else {
                        "pick up the throttle later and more gently to settle the rear"
                    });
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
    use crate::telemetry::trace::{build_run, detect_turns};
    use std::path::Path;

    #[test]
    fn fixture_traces_have_handling_channels() {
        let data = crate::telemetry::ibt::read_ibt(Path::new("tests/fixtures/roadatlanta-full.ibt")).expect("read fixture");
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
            let stats = phase_stats(&run.traces[0], segment, data.track.length_m, Some(peak));
            let grip = stats.grip_pct.expect("grip");
            assert!((20.0..=110.0).contains(&grip), "turn {} grip {grip}%", turn.label);
            assert!(stats.entry_balance_deg.is_some() && stats.exit_balance_deg.is_some(), "turn {} balance", turn.label);
        }
        let notes = corner_notes(&run.traces, &turns, data.track.length_m);
        assert!(notes.len() <= 3);
        assert!(notes.iter().all(|n| n.starts_with("Turn ")), "{notes:?}");
    }
}

