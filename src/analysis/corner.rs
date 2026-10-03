//! One corner on one lap, split into entry (segment start to apex) and exit (apex to segment
//! end), compared against either a reference lap or the driver's other laps.

use serde::Serialize;

use crate::stats::{grid_index, median};
use crate::telemetry::trace::{turn_segments, LapTrace, Turn};

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
    /// Mean combined g while braking or cornering, as % of the session peak. Feeds the notes; not shipped to the UI.
    #[serde(skip)]
    pub grip_pct: Option<f64>,
    /// Gear at the slowest point.
    pub apex_gear: i8,
    /// Metres from the turn's apex marker to where the car was slowest (negative = before it):
    /// where this lap actually apexed.
    pub min_speed_at_m: f64,
    /// Metres before the apex where the brake came off (below 5%) after the braking for this
    /// corner; negative when it was trailed past the apex.
    pub brake_release_m: Option<f64>,
    /// Lateral g around the slowest point, and the radius of the car's path there (v²/a): at
    /// the same grip, a tighter line means a lower apex speed. `None` without accelerometer
    /// data, or where the corner is barely a bend.
    pub apex_lat_g: Option<f64>,
    pub apex_radius_m: Option<f64>,
}

/// Below this lateral g around the slowest point, the path radius is mostly noise.
const MIN_APEX_LAT_G: f64 = 0.25;

pub fn phase_stats(trace: &LapTrace, segment: (f64, f64, f64), length_m: f64, peak_g: Option<f64>) -> PhaseStats {
    let n = trace.time_s.len() - 1;
    let idx = |pct: f64| grid_index(pct, n);
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

    // Off the brake: the first point after the braking started where it's below 5%.
    let brake_release = brake_idx.and_then(|start| (start..=b).find(|&j| trace.brake[j] < 5.0));

    // Grip and path radius around the slowest point, averaged over ~±5 m to calm the noise.
    let (apex_lat_g, apex_radius_m) = if has_g {
        let k = ((5.0 / ds).round() as usize).max(1);
        let window = min_idx.saturating_sub(k).max(a)..=(min_idx + k).min(b);
        let count = window.clone().count() as f64;
        let lat_g = window.clone().map(|j| (trace.lat_g[j] as f64).abs()).sum::<f64>() / count;
        let speed_ms = window.map(|j| trace.speed_kph[j] as f64 / 3.6).sum::<f64>() / count;
        if lat_g >= MIN_APEX_LAT_G {
            (Some(lat_g), Some(speed_ms * speed_ms / (lat_g * 9.81)))
        } else {
            (None, None)
        }
    } else {
        (None, None)
    };

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
        min_speed_at_m: from_apex(min_idx),
        brake_release_m: brake_release.map(|j| -from_apex(j)),
        apex_lat_g,
        apex_radius_m,
    }
}

/// How far past its threshold a cause has to be before it's named. Below 1.0 on purpose: the
/// apex speed is already known to be down, so the likeliest cause is worth naming even when
/// it's only most of the way to a clear-cut difference.
const MIN_CAUSE_SCORE: f64 = 0.6;

/// Why a lap's minimum speed through a corner was lower than the baseline's, as (why, what to
/// do) in words: whichever is furthest past its threshold among braking too early, coming off
/// the brake early and coasting in, a tighter line, not using the grip, apexing early or late,
/// and understeer. `None` when nothing comes close.
pub fn slow_apex_cause(sel: &PhaseStats, base: &PhaseStats) -> Option<(&'static str, &'static str)> {
    let mut causes: Vec<(f64, &'static str, &'static str)> = Vec::new();

    if let (Some(s), Some(b)) = (sel.brake_point_m, base.brake_point_m) {
        // Metres before the apex: larger is earlier.
        causes.push(((s - b) / 6.0, "you're braking early and scrubbing off too much speed before the apex", "Brake later and roll more speed in."));
    }
    if let (Some(s), Some(b)) = (sel.brake_release_m, base.brake_release_m) {
        causes.push(((s - b) / 8.0, "you're off the brakes early and coasting to the apex", "Trail the brake in further so the car turns and carries its speed."));
    }
    if let (Some(sg), Some(bg), Some(sr), Some(br)) = (sel.apex_lat_g, base.apex_lat_g, sel.apex_radius_m, base.apex_radius_m) {
        if sg >= bg - 0.08 {
            // Same grip on a smaller circle: the line, not the commitment.
            causes.push(((1.0 - sr / br) / 0.08, "your line's too tight there, so the apex speed drops", "Use more of the track on the way in for a wider, faster arc."));
        } else {
            causes.push(((bg - sg) / 0.1, "you're not using all the grip mid-corner", "The car's got more, trust it and carry more speed in."));
        }
    }
    let apex_shift = sel.min_speed_at_m - base.min_speed_at_m;
    causes.push((-apex_shift / 10.0, "you're apexing early, then having to wait for the exit", "Turn in a touch later."));
    causes.push((apex_shift / 10.0, "you're apexing late, with the car slowest too deep in the corner", "Turn in a touch earlier and get it rotated sooner."));
    if let (Some(s), Some(b)) = (sel.entry_balance_deg, base.entry_balance_deg) {
        let threshold = (0.25 * b.abs()).max(3.0);
        causes.push(((s - b) / threshold, "understeer's scrubbing the speed off", "Trail the brake to keep the nose loaded, and use less lock."));
    }

    causes
        .into_iter()
        .filter(|c| c.0 >= MIN_CAUSE_SCORE)
        .max_by(|x, y| x.0.total_cmp(&y.0))
        .map(|(_, why, what)| (why, what))
}

/// A "typical lap" through the corner: the median of each measurement over `laps`. Optional
/// measurements are only kept when at least half the laps have them (e.g. most laps braked).
pub fn median_stats(laps: &[PhaseStats]) -> PhaseStats {
    let med = |f: fn(&PhaseStats) -> f64| median(&mut laps.iter().map(f).collect::<Vec<_>>()).unwrap_or(0.0);
    let med_opt = |f: fn(&PhaseStats) -> Option<f64>| {
        let mut values: Vec<f64> = laps.iter().filter_map(f).collect();
        (values.len() * 2 >= laps.len()).then(|| median(&mut values)).flatten()
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
        min_speed_at_m: med(|s| s.min_speed_at_m),
        brake_release_m: med_opt(|s| s.brake_release_m),
        apex_lat_g: med_opt(|s| s.apex_lat_g),
        apex_radius_m: med_opt(|s| s.apex_radius_m),
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
}

pub fn terrain(trace: &LapTrace, segment: (f64, f64, f64), length_m: f64) -> Option<Terrain> {
    let alt = &trace.alt_m;
    if alt.is_empty() || length_m <= 0.0 {
        return None;
    }
    let n = alt.len() - 1;
    let idx = |pct: f64| grid_index(pct, n);
    let (a, apex, b) = (idx(segment.0), idx(segment.1), idx(segment.2));
    let ds = length_m / n as f64;
    let steps = |m: f64| (m / ds).round().max(1.0) as usize;
    let grade = |from: usize, to: usize| if to > from { (alt[to] - alt[from]) as f64 / ((to - from) as f64 * ds) * 100.0 } else { 0.0 };
    let k = steps(40.0);
    Some(Terrain {
        entry_grade_pct: grade(apex.saturating_sub(steps(150.0)).max(a), apex),
        exit_grade_pct: grade(apex, (apex + steps(150.0)).min(b)),
        apex_crest_m: alt[apex] as f64 - (alt[apex.saturating_sub(k)] as f64 + alt[(apex + k).min(n)] as f64) / 2.0,
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
/// driver counts; the rankings are over those. `peak_g` is the session's grip-use reference.
pub fn corner_report(
    traces: &[&LapTrace],
    turns: &[Turn],
    length_m: f64,
    peak_g: Option<f64>,
    turn_index: usize,
    lap: i32,
    ref_lap: Option<i32>,
) -> Option<CornerReport> {
    let turn = turns.get(turn_index)?;
    let segment = turn_segments(turns)[turn_index];
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
        let mut note = format!("Minimum speed {:.0} kph lower ({:.0} vs {:.0})", -min_diff, sel.min_speed_kph, base.min_speed_kph);
        if let Some((why, what)) = slow_apex_cause(sel, base) {
            note += &format!(": {why}. {what}");
        }
        entry.to_work_on.push(note);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telemetry::trace::{build_run, detect_turns};
    use std::path::Path;

    #[test]
    fn corner_report_on_fixture() {
        let data = crate::telemetry::ibt::read_ibt(Path::new("tests/fixtures/roadatlanta-full.ibt")).expect("read fixture");
        let run = build_run(&data.frames, &data.track);
        assert!(run.traces.len() >= 2, "fixture needs two traced laps");
        let turns = detect_turns(run.map_points.as_ref().unwrap(), data.track.length_m);
        let (a, b) = (run.traces[0].lap_number, run.traces[1].lap_number);
        let traces: Vec<&LapTrace> = run.traces.iter().collect();
        let peak_g = crate::analysis::handling::peak_combined_g(&run.traces);
        let length_m = data.track.length_m;

        for i in 0..turns.len() {
            let vs_lap = corner_report(&traces, &turns, length_m, peak_g, i, a, Some(b)).expect("report vs lap");
            assert_eq!(vs_lap.ref_lap, Some(b));
            let s = &vs_lap.sel;
            assert!(s.entry_time_s > 0.0 && s.exit_time_s > 0.0, "turn {} times {s:?}", vs_lap.turn);
            assert!(s.min_speed_kph <= s.entry_speed_kph + 0.1 && s.min_speed_kph <= s.exit_speed_kph + 0.1);
            // Every phase gets at least its time summary.
            assert!(!(vs_lap.entry.went_well.is_empty() && vs_lap.entry.to_work_on.is_empty()));
            assert!(!(vs_lap.exit.went_well.is_empty() && vs_lap.exit.to_work_on.is_empty()));

            let vs_field = corner_report(&traces, &turns, length_m, peak_g, i, a, None).expect("report vs field");
            assert_eq!(vs_field.ref_lap, None);
            assert_eq!(vs_field.field_size, run.traces.len() - 1);
            let r = vs_field.entry_rank.as_ref().expect("rank");
            assert!(r.rank >= 1 && r.rank <= r.of);
            assert!(crate::coach::prompt::corner_prompt(&vs_field).contains(&format!("Turn {}", vs_field.turn)));
        }
        assert!(corner_report(&traces, &turns, length_m, peak_g, turns.len(), a, None).is_none());
    }

    fn apex(brake_point_m: f64, brake_release_m: f64, lat_g: f64, radius_m: f64, min_speed_at_m: f64, entry_balance_deg: f64) -> PhaseStats {
        PhaseStats {
            entry_time_s: 3.0,
            exit_time_s: 3.0,
            entry_speed_kph: 200.0,
            brake_point_m: Some(brake_point_m),
            min_speed_kph: 100.0,
            throttle_on_m: Some(0.0),
            full_throttle_m: Some(40.0),
            exit_speed_kph: 160.0,
            coast_m: 0.0,
            abs_pct: Some(0.0),
            entry_balance_deg: Some(entry_balance_deg),
            exit_balance_deg: Some(0.0),
            grip_pct: None,
            apex_gear: 3,
            min_speed_at_m,
            brake_release_m: Some(brake_release_m),
            apex_lat_g: Some(lat_g),
            apex_radius_m: Some(radius_m),
        }
    }

    #[test]
    fn slow_apex_names_the_cause() {
        let best = apex(80.0, 10.0, 1.5, 60.0, 0.0, 2.0);
        let why = |s: &PhaseStats| slow_apex_cause(s, &best).map(|(why, _)| why).unwrap_or("none");
        assert!(why(&apex(95.0, 10.0, 1.5, 60.0, 0.0, 2.0)).contains("braking early"));
        assert!(why(&apex(80.0, 25.0, 1.5, 60.0, 0.0, 2.0)).contains("off the brakes early"));
        assert!(why(&apex(80.0, 10.0, 1.5, 50.0, 0.0, 2.0)).contains("line's too tight"));
        assert!(why(&apex(80.0, 10.0, 1.2, 60.0, 0.0, 2.0)).contains("not using all the grip"));
        assert!(why(&apex(80.0, 10.0, 1.5, 60.0, -15.0, 2.0)).contains("apexing early"));
        assert!(why(&apex(80.0, 10.0, 1.5, 60.0, 15.0, 2.0)).contains("apexing late"));
        assert!(why(&apex(80.0, 10.0, 1.5, 60.0, 0.0, 8.0)).contains("understeer"));
        // The same pass as the best one: nothing to blame.
        assert_eq!(why(&best), "none");
    }
}
