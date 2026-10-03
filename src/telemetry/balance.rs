//! Understeer/oversteer balance, added to each lap trace as it is built.

use crate::stats::percentile;
use crate::telemetry::trace::LapTrace;

/// Combined acceleration counted as "at the limit" is the session's 98th percentile, so a
/// single kerb strike doesn't set the reference.
pub const PEAK_PERCENTILE: f64 = 0.98;
/// Below this speed, steering and yaw are dominated by low-speed geometry (pit lane, hairpin
/// crawl after a spin) and aren't used for calibration.
const MIN_SPEED_MS: f64 = 15.0;
/// Balance is only reported where the steering the car's rotation needs is at least this
/// many degrees at the wheel; on straights the ratio is all noise.
const MIN_EXPECTED_STEER_DEG: f64 = 3.0;

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
