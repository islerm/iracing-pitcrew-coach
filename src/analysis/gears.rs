//! Upshifts: the RPM each lap changed up at, compared with the car's shift light, plus time
//! spent on the rev limiter.

use serde::Serialize;

use crate::stats::{median, percentile};
use crate::telemetry::trace::LapTrace;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Early,
    OnTime,
    Late,
    /// Changed up without full throttle, e.g. short-shifting for traction. Not judged.
    PartThrottle,
    /// No reference RPM to judge against.
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
pub struct Upshift {
    pub lap: i32,
    pub from: i8,
    pub to: i8,
    /// Lap fraction where the new gear engaged.
    pub pct: f64,
    /// Highest RPM in the few metres before the change.
    pub rpm: f64,
    pub verdict: Verdict,
}

#[derive(Debug, Clone, Serialize)]
pub struct GearPair {
    pub from: i8,
    pub to: i8,
    /// Full-throttle upshifts only.
    pub count: usize,
    pub median_rpm: f64,
    pub min_rpm: f64,
    pub max_rpm: f64,
    pub early: usize,
    pub late: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct LapLimiter {
    pub lap: i32,
    /// Metres at or above 99% of the redline.
    pub metres: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ShiftReport {
    pub shift_rpm: Option<f64>,
    pub redline_rpm: Option<f64>,
    /// What upshifts are judged against, and where it came from.
    pub reference_rpm: Option<f64>,
    pub reference_source: Option<String>,
    /// How far either side of the reference still counts as on time.
    pub tolerance_rpm: f64,
    pub upshifts: Vec<Upshift>,
    pub pairs: Vec<GearPair>,
    pub limiter: Vec<LapLimiter>,
    pub notes: Vec<String>,
}

/// Upshifts found in one lap's trace (before judging).
fn find_upshifts(trace: &LapTrace) -> Vec<(Upshift, bool)> {
    if trace.rpm.is_empty() {
        return Vec::new();
    }
    let n = trace.gear.len() - 1;
    // Look back ~5 grid points (~15 m) for the peak revs and the throttle before the change.
    let back = 5;
    (1..=n)
        .filter(|&j| trace.gear[j] > trace.gear[j - 1] && trace.gear[j - 1] >= 1)
        .map(|j| {
            let from = j.saturating_sub(back);
            let rpm = (from..j).map(|k| trace.rpm[k] as f64).fold(0.0, f64::max);
            let full = (from..j).map(|k| trace.throttle[k]).fold(0.0, f32::max) >= 90.0;
            let shift = Upshift {
                lap: trace.lap_number,
                from: trace.gear[j - 1],
                to: trace.gear[j],
                pct: j as f64 / n as f64,
                rpm,
                verdict: Verdict::Unknown,
            };
            (shift, full)
        })
        .collect()
}

pub fn shift_report(traces: &[&LapTrace], shift_rpm: Option<f64>, redline_rpm: Option<f64>, length_m: f64) -> ShiftReport {
    let mut found: Vec<(Upshift, bool)> = traces.iter().copied().flat_map(find_upshifts).collect();

    // Judge against the shift light. Without one, fall back to the highest revs you reach at
    // full throttle (98th percentile): not an optimum, but it shows which changes come early.
    let (reference_rpm, reference_source) = match shift_rpm {
        Some(rpm) => (Some(rpm), Some("the car's shift light".to_string())),
        None => {
            let mut peaks: Vec<f64> = found.iter().filter(|(_, full)| *full).map(|(s, _)| s.rpm).collect();
            if peaks.len() >= 3 {
                let p98 = percentile(&mut peaks, 0.98);
                (p98, Some("your highest shift RPM (the car's shift light isn't in this file)".to_string()))
            } else {
                (None, None)
            }
        }
    };
    let tolerance_rpm = reference_rpm.map(|r| (r * 0.03).max(200.0)).unwrap_or(0.0);

    for (shift, full) in found.iter_mut() {
        shift.verdict = match (reference_rpm, *full) {
            (_, false) => Verdict::PartThrottle,
            (None, true) => Verdict::Unknown,
            (Some(r), true) if shift.rpm < r - tolerance_rpm => Verdict::Early,
            (Some(r), true) if shift.rpm > r + tolerance_rpm => Verdict::Late,
            (Some(_), true) => Verdict::OnTime,
        };
    }
    let upshifts: Vec<Upshift> = found.into_iter().map(|(s, _)| s).collect();

    let mut keys: Vec<(i8, i8)> = upshifts.iter().map(|s| (s.from, s.to)).collect();
    keys.sort();
    keys.dedup();
    let pairs: Vec<GearPair> = keys
        .into_iter()
        .filter_map(|(from, to)| {
            let judged: Vec<&Upshift> = upshifts
                .iter()
                .filter(|s| s.from == from && s.to == to && s.verdict != Verdict::PartThrottle)
                .collect();
            if judged.is_empty() {
                return None;
            }
            let mut rpms: Vec<f64> = judged.iter().map(|s| s.rpm).collect();
            Some(GearPair {
                from,
                to,
                count: judged.len(),
                min_rpm: rpms.iter().copied().fold(f64::INFINITY, f64::min),
                max_rpm: rpms.iter().copied().fold(0.0, f64::max),
                median_rpm: median(&mut rpms).unwrap_or_default(),
                early: judged.iter().filter(|s| s.verdict == Verdict::Early).count(),
                late: judged.iter().filter(|s| s.verdict == Verdict::Late).count(),
            })
        })
        .collect();

    let limiter: Vec<LapLimiter> = match redline_rpm {
        Some(red) => traces
            .iter()
            .filter(|t| !t.rpm.is_empty())
            .map(|t| {
                let ds = length_m / (t.rpm.len() - 1).max(1) as f64;
                LapLimiter { lap: t.lap_number, metres: t.rpm.iter().filter(|&&r| r as f64 >= red * 0.99).count() as f64 * ds }
            })
            .collect(),
        None => Vec::new(),
    };

    let mut notes = Vec::new();
    if let Some(r) = reference_rpm {
        for p in &pairs {
            let diff = p.median_rpm - r;
            if p.early * 2 > p.count {
                notes.push(format!(
                    "Gear {}→{} upshift: you usually change up at {:.0} rpm, {:.0} rpm before {} ({} of {} early). Holding the gear longer should be quicker.",
                    p.from, p.to, p.median_rpm, -diff, reference_source.as_deref().unwrap_or("the reference"), p.early, p.count
                ));
            } else if p.late * 2 > p.count {
                notes.push(format!(
                    "Gear {}→{} upshift: you usually change up at {:.0} rpm, {:.0} rpm past {} ({} of {} late). Change up sooner.",
                    p.from, p.to, p.median_rpm, diff, reference_source.as_deref().unwrap_or("the reference"), p.late, p.count
                ));
            }
        }
        if !pairs.is_empty() && notes.is_empty() {
            notes.push("Your full-throttle upshifts are on time.".to_string());
        }
    }
    let mut limiter_m: Vec<f64> = limiter.iter().map(|l| l.metres).collect();
    if !limiter_m.is_empty() {
        let m = median(&mut limiter_m).unwrap_or_default();
        if m >= 20.0 {
            notes.push(format!("You're on the rev limiter for about {m:.0} m a lap. That's time the engine isn't accelerating the car: change up before it cuts in."));
        }
    }

    ShiftReport {
        shift_rpm,
        redline_rpm,
        reference_rpm,
        reference_source,
        tolerance_rpm,
        upshifts,
        pairs,
        limiter,
        notes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telemetry::trace::build_run;
    use std::path::Path;

    #[test]
    fn shifts_on_fixture() {
        let data = crate::telemetry::ibt::read_ibt(Path::new("tests/fixtures/roadatlanta-full.ibt")).expect("read fixture");
        let run = build_run(&data.frames, &data.track);
        assert!(run.traces.iter().all(|t| t.rpm.len() == t.time_s.len()), "fixture has RPM");

        // No shift light in the fixture: the reference falls back to your own highest shifts.
        let traces: Vec<&LapTrace> = run.traces.iter().collect();
        let report = shift_report(&traces, None, None, data.track.length_m);
        assert!(report.upshifts.len() >= 4 * run.traces.len(), "{} upshifts", report.upshifts.len());
        assert!(report.upshifts.iter().all(|s| s.to == s.from + 1 && s.rpm > 3000.0 && s.rpm < 12000.0));
        assert!(report.reference_rpm.is_some());
        assert!(!report.pairs.is_empty());

        // With a known shift light, everything is judged against it.
        let high = shift_report(&traces, Some(20000.0), Some(21000.0), data.track.length_m);
        assert!(high.upshifts.iter().all(|s| matches!(s.verdict, Verdict::Early | Verdict::PartThrottle)));
        assert!(high.notes.iter().any(|n| n.contains("early")));
        assert!(high.limiter.iter().all(|l| l.metres == 0.0));
    }
}
