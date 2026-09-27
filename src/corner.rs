//! One corner on one lap, split into entry (segment start to apex) and exit (apex to segment
//! end), compared against either a reference lap or the driver's other laps.

use serde::Serialize;

use crate::handling::{peak_combined_g, turn_segments};
use crate::trace::{LapTrace, Turn};

/// How one lap drove the two halves of one corner.
#[derive(Debug, Clone, Serialize)]
pub struct PhaseStats {
    pub entry_time_s: f64,
    pub exit_time_s: f64,
    /// Speed where braking starts, or the top speed before the corner when it is taken without braking.
    pub entry_speed_kph: f64,
    /// Metres before the apex where the brake first reaches 10% on the way into the corner.
    /// Larger = earlier.
    pub brake_point_m: Option<f64>,
    pub min_speed_kph: f64,
    /// Metres from the apex to the throttle reaching 20% after the slowest point (negative = before the apex).
    pub throttle_on_m: Option<f64>,
    /// Same, for 95% throttle.
    pub full_throttle_m: Option<f64>,
    pub exit_speed_kph: f64,
    /// Distance with neither pedal pressed.
    pub coast_m: f64,
    /// % of braking samples with ABS active.
    pub abs_pct: Option<f64>,
    /// Mean balance where the car is most loaded laterally (+ understeer, − oversteer).
    pub entry_balance_deg: Option<f64>,
    pub exit_balance_deg: Option<f64>,
    /// Mean combined g while braking or cornering, as % of the session peak.
    pub grip_pct: Option<f64>,
    /// Gear at the slowest point.
    pub apex_gear: i8,
}

pub fn phase_stats(trace: &LapTrace, segment: (f64, f64, f64), length_m: f64, peak_g: Option<f64>) -> PhaseStats {
    let n = trace.time_s.len() - 1;
    let idx = |pct: f64| ((pct * n as f64).round() as usize).min(n);
    let (a, apex, b) = (idx(segment.0), idx(segment.1), idx(segment.2));
    let ds = length_m / n as f64;
    let from_apex = |j: usize| (j as f64 - apex as f64) * ds;

    let cmp = |x: &usize, y: &usize| trace.speed_kph[*x].partial_cmp(&trace.speed_kph[*y]).unwrap_or(std::cmp::Ordering::Equal);
    let min_idx = (a..=b).min_by(cmp).unwrap_or(apex);
    // The braking that slows the car for this corner starts after the last speed peak before
    // the slowest point; a dab earlier in the segment (the previous corner) doesn't count.
    let peak_idx = (a..=min_idx).max_by(cmp).unwrap_or(a);
    let brake_idx = (peak_idx..=min_idx).find(|&j| trace.brake[j] >= 10.0);
    let throttle_at = |level: f32| (min_idx..=b).find(|&j| trace.throttle[j] >= level).map(from_apex);

    let has_g = !trace.lat_g.is_empty();
    let balance_over = |from: usize, to: usize| {
        (!trace.balance_deg.is_empty() && to > from).then(|| {
            let load = |j: usize| if has_g { (trace.lat_g[j] as f64).abs() } else { (trace.yaw_rate_dps[j] as f64 * trace.speed_kph[j] as f64).abs() };
            let mut loaded: Vec<usize> = (from..=to).collect();
            loaded.sort_by(|&x, &y| load(y).partial_cmp(&load(x)).unwrap_or(std::cmp::Ordering::Equal));
            loaded.truncate((loaded.len() / 3).max(1));
            loaded.iter().map(|&j| trace.balance_deg[j] as f64).sum::<f64>() / loaded.len() as f64
        })
    };

    let grip_pct = peak_g.filter(|_| has_g).and_then(|peak| {
        let working: Vec<f64> = (a..=b)
            .filter(|&j| trace.brake[j] >= 5.0 || (trace.lat_g[j] as f64).abs() > 0.3 * peak)
            .map(|j| (trace.lat_g[j] as f64).hypot(trace.long_g[j] as f64))
            .collect();
        (!working.is_empty()).then(|| working.iter().sum::<f64>() / working.len() as f64 / peak * 100.0)
    });

    let abs_pct = (!trace.abs.is_empty()).then(|| {
        let braking: Vec<usize> = (a..=b).filter(|&j| trace.brake[j] >= 10.0).collect();
        if braking.is_empty() {
            0.0
        } else {
            braking.iter().filter(|&&j| trace.abs[j] != 0).count() as f64 / braking.len() as f64 * 100.0
        }
    });

    PhaseStats {
        entry_time_s: (trace.time_s[apex] - trace.time_s[a]) as f64,
        exit_time_s: (trace.time_s[b] - trace.time_s[apex]) as f64,
        entry_speed_kph: trace.speed_kph[brake_idx.unwrap_or(peak_idx)] as f64,
        brake_point_m: brake_idx.map(|j| -from_apex(j)),
        min_speed_kph: trace.speed_kph[min_idx] as f64,
        throttle_on_m: throttle_at(20.0),
        full_throttle_m: throttle_at(95.0),
        exit_speed_kph: trace.speed_kph[b] as f64,
        coast_m: (a..=b).filter(|&j| trace.throttle[j] < 5.0 && trace.brake[j] < 5.0).count() as f64 * ds,
        abs_pct,
        entry_balance_deg: balance_over(a, apex),
        exit_balance_deg: balance_over(apex, b),
        grip_pct,
        apex_gear: trace.gear[min_idx],
    }
}

fn median(mut values: Vec<f64>) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let m = values.len() / 2;
    Some(if values.len() % 2 == 0 { (values[m - 1] + values[m]) / 2.0 } else { values[m] })
}

/// A "typical lap" through the corner: the median of each measurement over `laps`. Optional
/// measurements are only kept when at least half the laps have them (e.g. most laps braked).
fn median_stats(laps: &[PhaseStats]) -> PhaseStats {
    let med = |f: fn(&PhaseStats) -> f64| median(laps.iter().map(f).collect()).unwrap_or(0.0);
    let med_opt = |f: fn(&PhaseStats) -> Option<f64>| {
        let values: Vec<f64> = laps.iter().filter_map(f).collect();
        (values.len() * 2 >= laps.len()).then(|| median(values)).flatten()
    };
    PhaseStats {
        entry_time_s: med(|s| s.entry_time_s),
        exit_time_s: med(|s| s.exit_time_s),
        entry_speed_kph: med(|s| s.entry_speed_kph),
        brake_point_m: med_opt(|s| s.brake_point_m),
        min_speed_kph: med(|s| s.min_speed_kph),
        throttle_on_m: med_opt(|s| s.throttle_on_m),
        full_throttle_m: med_opt(|s| s.full_throttle_m),
        exit_speed_kph: med(|s| s.exit_speed_kph),
        coast_m: med(|s| s.coast_m),
        abs_pct: med_opt(|s| s.abs_pct),
        entry_balance_deg: med_opt(|s| s.entry_balance_deg),
        exit_balance_deg: med_opt(|s| s.exit_balance_deg),
        grip_pct: med_opt(|s| s.grip_pct),
        apex_gear: mode(laps.iter().map(|s| s.apex_gear)),
    }
}

/// Most common value (the lowest on a tie).
fn mode(values: impl Iterator<Item = i8>) -> i8 {
    let mut counts = std::collections::BTreeMap::new();
    for v in values {
        *counts.entry(v).or_insert(0usize) += 1;
    }
    counts.iter().max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0))).map(|(v, _)| *v).unwrap_or(0)
}

/// Laps grouped by the gear they used at the slowest point, with their time through the corner.
#[derive(Debug, Clone, Serialize)]
pub struct GearOption {
    pub gear: i8,
    pub laps: Vec<i32>,
    pub avg_time_s: f64,
    pub best_time_s: f64,
}

/// The shape of the road through the corner, from the altitude channel.
#[derive(Debug, Clone, Serialize)]
pub struct Terrain {
    /// Average gradient over the ~150 m before the apex (the braking zone). + uphill.
    pub entry_grade_pct: f64,
    /// Average gradient over the ~150 m after the apex. + uphill.
    pub exit_grade_pct: f64,
    /// How far the apex sits above (+, a crest) or below (−, a compression) the road 40 m
    /// either side of it.
    pub apex_crest_m: f64,
    /// Height change from the segment start to its end.
    pub change_m: f64,
}

pub fn terrain(trace: &LapTrace, segment: (f64, f64, f64), length_m: f64) -> Option<Terrain> {
    let alt = &trace.alt_m;
    if alt.is_empty() || length_m <= 0.0 {
        return None;
    }
    let n = alt.len() - 1;
    let idx = |pct: f64| ((pct * n as f64).round() as usize).min(n);
    let (a, apex, b) = (idx(segment.0), idx(segment.1), idx(segment.2));
    let ds = length_m / n as f64;
    let steps = |m: f64| (m / ds).round().max(1.0) as usize;
    let grade = |from: usize, to: usize| if to > from { (alt[to] - alt[from]) as f64 / ((to - from) as f64 * ds) * 100.0 } else { 0.0 };
    let k = steps(40.0);
    Some(Terrain {
        entry_grade_pct: grade(apex.saturating_sub(steps(150.0)).max(a), apex),
        exit_grade_pct: grade(apex, (apex + steps(150.0)).min(b)),
        apex_crest_m: alt[apex] as f64 - (alt[apex.saturating_sub(k)] as f64 + alt[(apex + k).min(n)] as f64) / 2.0,
        change_m: (alt[b] - alt[a]) as f64,
    })
}

/// What the terrain means for driving the corner, for the driver and the coach.
fn terrain_notes(t: &Terrain) -> Vec<String> {
    let mut notes = Vec::new();
    if t.entry_grade_pct <= -3.0 {
        notes.push(format!("Downhill braking zone ({:.0}% grade): the car needs more distance to slow down and the rear goes light, so brake a touch earlier and keep it straight.", t.entry_grade_pct));
    } else if t.entry_grade_pct >= 3.0 {
        notes.push(format!("Uphill braking zone (+{:.0}% grade): gravity helps you slow down, so you can brake later than it looks.", t.entry_grade_pct));
    }
    if t.apex_crest_m >= 0.5 {
        notes.push(format!("The apex is on a crest ({:.1} m): the car goes light there, so there is less grip mid-corner. Be smooth with the steering and the throttle.", t.apex_crest_m));
    } else if t.apex_crest_m <= -0.5 {
        notes.push(format!("The apex is in a compression ({:.1} m): the extra load gives more grip, so you can carry more speed.", -t.apex_crest_m));
    }
    if t.exit_grade_pct >= 3.0 {
        notes.push(format!("Uphill exit (+{:.0}%): traction is good but the car accelerates slower, so an early, clean exit pays off.", t.exit_grade_pct));
    } else if t.exit_grade_pct <= -3.0 {
        notes.push(format!("Downhill exit ({:.0}%): the rear unloads, so feed in the throttle gently.", t.exit_grade_pct));
    }
    notes
}

#[derive(Debug, Clone, Serialize)]
pub struct PhaseRank {
    /// 1 = quickest of the laps considered.
    pub rank: usize,
    pub of: usize,
    pub best_lap: i32,
    pub best_time_s: f64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct PhaseNotes {
    pub went_well: Vec<String>,
    pub to_work_on: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CornerReport {
    pub turn: String,
    pub lap: i32,
    /// The reference lap, or `None` when comparing against the typical lap.
    pub ref_lap: Option<i32>,
    /// How many other laps make up the typical lap.
    pub field_size: usize,
    /// "lap 5" or "your typical lap".
    pub baseline_label: String,
    pub sel: PhaseStats,
    pub base: PhaseStats,
    pub entry_rank: Option<PhaseRank>,
    pub exit_rank: Option<PhaseRank>,
    pub entry: PhaseNotes,
    pub exit: PhaseNotes,
    /// Laps grouped by apex gear. Only filled when the laps used more than one gear here.
    pub gear_options: Vec<GearOption>,
    pub terrain: Option<Terrain>,
    pub terrain_notes: Vec<String>,
}

fn rank(lap: i32, laps: &[(i32, f64)]) -> Option<PhaseRank> {
    let mine = laps.iter().find(|(n, _)| *n == lap)?.1;
    let (best_lap, best_time_s) = laps.iter().copied().min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))?;
    Some(PhaseRank {
        rank: 1 + laps.iter().filter(|(_, t)| *t < mine).count(),
        of: laps.len(),
        best_lap,
        best_time_s,
    })
}

/// Compares `lap` through turn `turn_index` against `ref_lap`, or against the median of the
/// other laps in `traces` when `ref_lap` is `None`. `traces` should hold only the laps the
/// driver counts; the rankings are over those.
pub fn corner_report(traces: &[LapTrace], turns: &[Turn], length_m: f64, turn_index: usize, lap: i32, ref_lap: Option<i32>) -> Option<CornerReport> {
    let turn = turns.get(turn_index)?;
    let segment = turn_segments(turns)[turn_index];
    let peak_g = peak_combined_g(traces);
    let stats: Vec<(i32, PhaseStats)> = traces.iter().map(|t| (t.lap_number, phase_stats(t, segment, length_m, peak_g))).collect();
    let find = |n: i32| stats.iter().find(|(l, _)| *l == n).map(|(_, s)| s.clone());

    let sel = find(lap)?;
    let others: Vec<PhaseStats> = stats.iter().filter(|(l, _)| *l != lap).map(|(_, s)| s.clone()).collect();
    let (base, baseline_label) = match ref_lap.filter(|r| *r != lap) {
        Some(r) => (find(r)?, format!("lap {r}")),
        None if !others.is_empty() => (median_stats(&others), "your typical lap".to_string()),
        None => return None,
    };

    let entry_times: Vec<(i32, f64)> = stats.iter().map(|(l, s)| (*l, s.entry_time_s)).collect();
    let exit_times: Vec<(i32, f64)> = stats.iter().map(|(l, s)| (*l, s.exit_time_s)).collect();
    let (entry, mut exit) = phase_notes(&sel, &base, &baseline_label);

    // Gear choice: when laps took the corner in different gears, which was quicker?
    let mut gear_options: Vec<GearOption> = Vec::new();
    for (l, s) in &stats {
        let t = s.entry_time_s + s.exit_time_s;
        match gear_options.iter_mut().find(|g| g.gear == s.apex_gear) {
            Some(g) => {
                g.laps.push(*l);
                g.avg_time_s += t;
                g.best_time_s = g.best_time_s.min(t);
            }
            None => gear_options.push(GearOption { gear: s.apex_gear, laps: vec![*l], avg_time_s: t, best_time_s: t }),
        }
    }
    for g in gear_options.iter_mut() {
        g.avg_time_s /= g.laps.len() as f64;
    }
    gear_options.sort_by_key(|g| g.gear);
    if gear_options.len() < 2 {
        gear_options.clear();
    }
    // Best against best: an average would blame the gear for a lap that was messy for other reasons.
    if let Some(mine) = gear_options.iter().find(|g| g.gear == sel.apex_gear) {
        let quickest = gear_options
            .iter()
            .min_by(|a, b| a.best_time_s.partial_cmp(&b.best_time_s).unwrap_or(std::cmp::Ordering::Equal))
            .expect("non-empty");
        let laps = |g: &GearOption| if g.laps.len() == 1 { "1 lap".to_string() } else { format!("{} laps", g.laps.len()) };
        // A bigger gap than this is a mistake on that pass, not the gear.
        let plausible = |gap: f64| (0.05..=0.8).contains(&gap);
        if quickest.gear != mine.gear && plausible(mine.best_time_s - quickest.best_time_s) {
            exit.to_work_on.push(format!(
                "Gear choice: your best pass in gear {} was {:.2}s quicker than your best in gear {}, the gear this lap used ({} vs {})",
                quickest.gear,
                mine.best_time_s - quickest.best_time_s,
                mine.gear,
                laps(quickest),
                laps(mine)
            ));
        } else if quickest.gear == mine.gear {
            let next = gear_options.iter().filter(|g| g.gear != mine.gear).map(|g| g.best_time_s - mine.best_time_s).fold(f64::INFINITY, f64::min);
            if plausible(next) {
                exit.went_well.push(format!("Gear choice: gear {} gives your quickest pass through here ({:.2}s better than your best in another gear)", mine.gear, next));
            }
        }
    }

    let terrain = traces.iter().find(|t| t.lap_number == lap).and_then(|t| terrain(t, segment, length_m));
    let terrain_notes = terrain.as_ref().map(terrain_notes).unwrap_or_default();

    Some(CornerReport {
        gear_options,
        terrain,
        terrain_notes,
        turn: turn.label.clone(),
        lap,
        ref_lap: ref_lap.filter(|r| *r != lap),
        field_size: others.len(),
        baseline_label,
        entry_rank: rank(lap, &entry_times).filter(|r| r.of >= 2),
        exit_rank: rank(lap, &exit_times).filter(|r| r.of >= 2),
        sel,
        base,
        entry,
        exit,
    })
}

/// Plain-language differences between the lap and the baseline, split by phase.
fn phase_notes(sel: &PhaseStats, base: &PhaseStats, vs: &str) -> (PhaseNotes, PhaseNotes) {
    let mut entry = PhaseNotes::default();
    let mut exit = PhaseNotes::default();

    let time_note = |notes: &mut PhaseNotes, phase: &str, dt: f64| {
        if dt <= -0.02 {
            notes.went_well.push(format!("{:.2}s quicker on {phase} than {vs}", -dt));
        } else if dt >= 0.02 {
            notes.to_work_on.push(format!("{dt:.2}s slower on {phase} than {vs}"));
        } else {
            notes.went_well.push(format!("Matched {vs} on {phase}"));
        }
    };

    // ---- Entry ----
    let entry_dt = sel.entry_time_s - base.entry_time_s;
    time_note(&mut entry, "entry", entry_dt);

    let arrive = sel.entry_speed_kph - base.entry_speed_kph;
    if arrive >= 3.0 {
        entry.went_well.push(format!("Arrived {arrive:.0} kph faster (a better exit from the corner before)"));
    } else if arrive <= -3.0 {
        entry.to_work_on.push(format!("Arrived {:.0} kph slower, carried over from the corner before", -arrive));
    }

    let min_diff = sel.min_speed_kph - base.min_speed_kph;
    match (sel.brake_point_m, base.brake_point_m) {
        (Some(s), Some(b)) => {
            let earlier = s - b;
            if earlier <= -5.0 {
                if entry_dt <= 0.0 {
                    entry.went_well.push(format!("Braked {:.0} m later ({s:.0} m before the apex vs {b:.0} m)", -earlier));
                } else if min_diff <= -2.0 {
                    entry.to_work_on.push(format!(
                        "Braked {:.0} m later but the minimum speed dropped {:.0} kph: the late braking cost more than it gained",
                        -earlier, -min_diff
                    ));
                } else {
                    entry.to_work_on.push(format!("Braked {:.0} m later without gaining time on entry", -earlier));
                }
            } else if earlier >= 5.0 {
                if entry_dt > 0.0 {
                    entry.to_work_on.push(format!("Braked {earlier:.0} m earlier ({s:.0} m before the apex vs {b:.0} m)"));
                } else {
                    entry.went_well.push(format!("Braked {earlier:.0} m earlier and was still quicker: a cleaner entry"));
                }
            }
        }
        (Some(s), None) => entry.to_work_on.push(format!("Used the brake ({s:.0} m before the apex) where {vs} only lifted")),
        (None, Some(_)) => entry.went_well.push(format!("Took it without braking where {vs} braked")),
        (None, None) => {}
    }

    if min_diff >= 2.0 {
        entry.went_well.push(format!("Carried {min_diff:.0} kph more minimum speed ({:.0} vs {:.0})", sel.min_speed_kph, base.min_speed_kph));
    } else if min_diff <= -2.0 {
        entry.to_work_on.push(format!("Minimum speed {:.0} kph lower ({:.0} vs {:.0})", -min_diff, sel.min_speed_kph, base.min_speed_kph));
    }

    if let (Some(s), Some(b)) = (sel.abs_pct, base.abs_pct) {
        if s - b >= 15.0 {
            entry.to_work_on.push(format!("ABS active for {s:.0}% of braking vs {b:.0}%: ease off the peak pressure"));
        } else if b - s >= 15.0 {
            entry.went_well.push(format!("Less ABS ({s:.0}% of braking vs {b:.0}%)"));
        }
    }

    // Balance grows with cornering load, so small differences in fast corners are noise.
    let balance_note = |notes: &mut PhaseNotes, phase: &str, s: Option<f64>, b: Option<f64>| {
        if let (Some(s), Some(b)) = (s, b) {
            let threshold = (0.25 * b.abs()).max(3.0);
            if s - b >= threshold {
                notes.to_work_on.push(format!("More understeer on {phase} ({:+.0}° extra steering lock)", s - b));
            } else if s - b <= -threshold {
                notes.to_work_on.push(format!("More oversteer on {phase} ({:.0}° less lock than the rotation needed)", b - s));
            }
        }
    };
    balance_note(&mut entry, "entry", sel.entry_balance_deg, base.entry_balance_deg);

    let coast = sel.coast_m - base.coast_m;
    if coast >= 10.0 {
        entry.to_work_on.push(format!("{coast:.0} m more coasting between the brake and the throttle"));
    } else if coast <= -10.0 {
        entry.went_well.push(format!("{:.0} m less coasting", -coast));
    }

    // ---- Exit ----
    time_note(&mut exit, "exit", sel.exit_time_s - base.exit_time_s);

    let pedal_note = |notes: &mut PhaseNotes, what: &str, s: Option<f64>, b: Option<f64>, threshold: f64| {
        if let (Some(s), Some(b)) = (s, b) {
            if s - b <= -threshold {
                notes.went_well.push(format!("{what} {:.0} m sooner", b - s));
            } else if s - b >= threshold {
                notes.to_work_on.push(format!("{what} {:.0} m later", s - b));
            }
        }
    };
    pedal_note(&mut exit, "Back on the throttle", sel.throttle_on_m, base.throttle_on_m, 5.0);
    pedal_note(&mut exit, "Full throttle", sel.full_throttle_m, base.full_throttle_m, 8.0);

    let exit_diff = sel.exit_speed_kph - base.exit_speed_kph;
    if exit_diff >= 2.0 {
        exit.went_well.push(format!("{exit_diff:.0} kph faster onto the next straight"));
    } else if exit_diff <= -2.0 {
        exit.to_work_on.push(format!("{:.0} kph slower onto the next straight", -exit_diff));
    }

    balance_note(&mut exit, "exit", sel.exit_balance_deg, base.exit_balance_deg);

    if sel.apex_gear != base.apex_gear && sel.apex_gear > 0 && base.apex_gear > 0 {
        let total = sel.entry_time_s + sel.exit_time_s - base.entry_time_s - base.exit_time_s;
        let note = format!("Took the apex in gear {} where {vs} used gear {}", sel.apex_gear, base.apex_gear);
        if total <= -0.02 {
            exit.went_well.push(format!("{note}, and was quicker through the corner"));
        } else if total >= 0.02 {
            exit.to_work_on.push(format!("{note}, and was slower through the corner"));
        }
    }

    (entry, exit)
}

/// Prompt for the coach model: the findings first (small local models follow those far better
/// than raw numbers), then the measurements, with which direction is better spelled out.
pub fn build_corner_prompt(report: &CornerReport) -> String {
    let (s, b) = (&report.sel, &report.base);
    let vs = &report.baseline_label;
    let opt = |v: Option<f64>, unit: &str| v.map(|v| format!("{v:.0}{unit}")).unwrap_or_else(|| "n/a".to_string());
    let row = |name: &str, sv: String, bv: String, hint: &str| format!("- {name}: {sv} on lap {} vs {bv} on {vs}{hint}", report.lap);
    let mut lines = vec![
        row("Entry time (turn-in zone to apex)", format!("{:.3}s", s.entry_time_s), format!("{:.3}s", b.entry_time_s), " (lower is better)"),
        row("Exit time (apex to the next straight)", format!("{:.3}s", s.exit_time_s), format!("{:.3}s", b.exit_time_s), " (lower is better)"),
        row("Speed when braking starts", format!("{:.0} kph", s.entry_speed_kph), format!("{:.0} kph", b.entry_speed_kph), ""),
        row("Brake point, metres before the apex", opt(s.brake_point_m, " m"), opt(b.brake_point_m, " m"), " (smaller = later braking)"),
        row("Minimum speed", format!("{:.0} kph", s.min_speed_kph), format!("{:.0} kph", b.min_speed_kph), " (higher is better)"),
        row("Throttle pickup, metres after the apex", opt(s.throttle_on_m, " m"), opt(b.throttle_on_m, " m"), " (smaller = earlier, better)"),
        row("Full throttle, metres after the apex", opt(s.full_throttle_m, " m"), opt(b.full_throttle_m, " m"), " (smaller = earlier, better)"),
        row("Exit speed", format!("{:.0} kph", s.exit_speed_kph), format!("{:.0} kph", b.exit_speed_kph), " (higher is better)"),
        row("Coasting, no pedals", format!("{:.0} m", s.coast_m), format!("{:.0} m", b.coast_m), " (less is better)"),
        row("Gear at the slowest point", s.apex_gear.to_string(), b.apex_gear.to_string(), ""),
    ];
    for g in &report.gear_options {
        lines.push(format!("- Laps taking the apex in gear {}: {} lap(s), best {:.3}s through the corner", g.gear, g.laps.len(), g.best_time_s));
    }
    if s.abs_pct.is_some() {
        lines.push(row("ABS share of braking", opt(s.abs_pct, "%"), opt(b.abs_pct, "%"), " (less is better)"));
    }
    if s.entry_balance_deg.is_some() {
        let hint = " (positive = understeer, negative = oversteer; closer to zero is more neutral)";
        lines.push(row("Entry balance", opt(s.entry_balance_deg, "°"), opt(b.entry_balance_deg, "°"), hint));
        lines.push(row("Exit balance", opt(s.exit_balance_deg, "°"), opt(b.exit_balance_deg, "°"), hint));
    }
    for (name, r) in [("entry", &report.entry_rank), ("exit", &report.exit_rank)] {
        if let Some(r) = r {
            lines.push(format!("- Lap {}'s {name} ranks {} of {} laps (quickest: lap {}).", report.lap, r.rank, r.of, r.best_lap));
        }
    }
    let list = |items: &[String]| if items.is_empty() { "nothing notable".to_string() } else { items.join("; ") };
    let terrain = if report.terrain_notes.is_empty() {
        String::new()
    } else {
        format!("The track here (use this to explain why, e.g. where the car is light or loaded):\n{}\n\n", report.terrain_notes.iter().map(|n| format!("- {n}")).collect::<Vec<_>>().join("\n"))
    };

    format!(
        "You are a race engineer coaching an iRacing driver through Turn {turn}. You are comparing lap {lap} with {vs}. \
Use only the facts below and don't invent numbers or targets.\n\n\
Findings:\n- Entry, went well: {ew}\n- Entry, to work on: {eb}\n- Exit, went well: {xw}\n- Exit, to work on: {xb}\n\n\
{terrain}\
Measurements:\n{data}\n\n\
Reply in under 120 words as two short paragraphs starting \"Entry:\" and \"Exit:\". In each, name one thing to keep doing and one concrete change \
for the next lap, based on the findings. Call the comparison \"{vs}\", never \"last lap\". Speak like a calm pit lane engineer, no bullet points.",
        turn = report.turn,
        lap = report.lap,
        ew = list(&report.entry.went_well),
        eb = list(&report.entry.to_work_on),
        xw = list(&report.exit.went_well),
        xb = list(&report.exit.to_work_on),
        data = lines.join("\n"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace::{build_run, detect_turns};
    use std::path::Path;

    #[test]
    fn corner_report_on_fixture() {
        let data = crate::ibt::read_ibt(Path::new("tests/fixtures/roadatlanta-full.ibt")).expect("read fixture");
        let run = build_run(&data.frames, &data.track);
        assert!(run.traces.len() >= 2, "fixture needs two traced laps");
        let turns = detect_turns(run.map_points.as_ref().unwrap(), data.track.length_m);
        let (a, b) = (run.traces[0].lap_number, run.traces[1].lap_number);

        for i in 0..turns.len() {
            let vs_lap = corner_report(&run.traces, &turns, data.track.length_m, i, a, Some(b)).expect("report vs lap");
            assert_eq!(vs_lap.ref_lap, Some(b));
            let s = &vs_lap.sel;
            assert!(s.entry_time_s > 0.0 && s.exit_time_s > 0.0, "turn {} times {s:?}", vs_lap.turn);
            assert!(s.min_speed_kph <= s.entry_speed_kph + 0.1 && s.min_speed_kph <= s.exit_speed_kph + 0.1);
            // Every phase gets at least its time summary.
            assert!(!(vs_lap.entry.went_well.is_empty() && vs_lap.entry.to_work_on.is_empty()));
            assert!(!(vs_lap.exit.went_well.is_empty() && vs_lap.exit.to_work_on.is_empty()));

            let vs_field = corner_report(&run.traces, &turns, data.track.length_m, i, a, None).expect("report vs field");
            assert_eq!(vs_field.ref_lap, None);
            assert_eq!(vs_field.field_size, run.traces.len() - 1);
            let r = vs_field.entry_rank.as_ref().expect("rank");
            assert!(r.rank >= 1 && r.rank <= r.of);
            assert!(build_corner_prompt(&vs_field).contains(&format!("Turn {}", vs_field.turn)));
        }
        assert!(corner_report(&run.traces, &turns, data.track.length_m, turns.len(), a, None).is_none());
    }
}
