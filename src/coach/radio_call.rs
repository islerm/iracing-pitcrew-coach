//! What the pit engineer says on the radio: a short call after each lap (the one corner that
//! cost the most, why, and what to do about it) and a fuller debrief when the car comes into
//! the pit lane. Kept light on numbers so it can be taken in while driving. Written for the
//! screen; `voice::speech` turns it into spoken form ("0.30s" → "three tenths").

use serde::Serialize;

use crate::analysis::corner::{median_stats, phase_stats, slow_apex_cause, PhaseStats};
use crate::analysis::handling::peak_combined_g;
use crate::stats::median;
use crate::telemetry::trace::{turn_segments, LapMetrics, LapTrace, RunData, Turn};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CallKind {
    /// After crossing the line.
    Lap,
    /// On entering the pit lane: the stint debrief.
    Pit,
}

#[derive(Debug, Clone, Serialize)]
pub struct LapCall {
    /// The lap the call is about (for a pit debrief, the lap the car came in on).
    pub lap_number: i32,
    pub kind: CallKind,
    pub text: String,
}

/// Smallest loss in one corner worth a call.
const MIN_LOSS_S: f64 = 0.05;

/// Longest stub of the lap in progress a lap snapshot can end with: snapshots are taken 2.5 s
/// after the line (see `live::Recorder`).
const IN_PROGRESS_MAX_S: f64 = 4.0;

/// The call for the lap that just finished. `run` is built from a snapshot taken shortly after
/// crossing the line. The few seconds of the next lap in it only show up as a lap when they
/// cover enough of the track (on a short one), so a short untimed last lap is skipped.
pub fn lap_call(run: &RunData, turns: &[Turn], length_m: f64) -> Option<LapCall> {
    let mut latest = run.laps.iter().rev();
    let lap = match latest.next()? {
        stub if !stub.is_complete && stub.lap_time_s < IN_PROGRESS_MAX_S => latest.next()?,
        lap => lap,
    };
    let text = call_for(&run.laps, &run.traces, lap, turns, length_m)?;
    Some(LapCall { lap_number: lap.lap_number, kind: CallKind::Lap, text })
}

/// The debrief for the stint so far, when the car enters the pit lane. It's filed under the
/// last lap that could have had a call of its own, not the in-lap still in progress.
pub fn pit_debrief(run: &RunData, turns: &[Turn], length_m: f64) -> Option<LapCall> {
    let lap = run.laps.iter().rev().find(|l| l.is_complete || !l.off_track_pcts.is_empty()).or(run.laps.last())?;
    Some(LapCall { lap_number: lap.lap_number, kind: CallKind::Pit, text: debrief(&run.laps, &run.traces, turns, length_m) })
}

/// One line of a run's radio transcript.
#[derive(Debug, Clone, Serialize)]
pub struct RadioEntry {
    #[serde(flatten)]
    pub call: LapCall,
    /// True when the call went out over the radio during the run; false when it was worked
    /// out afterwards (imported files, or laps the live radio skipped).
    pub live: bool,
}

/// The run's radio transcript: the calls made live (pit debriefs included), with every other
/// lap filled in with what the engineer would have said then, from the laps driven up to that
/// point.
pub fn transcript(laps: &[LapMetrics], traces: &[LapTrace], turns: &[Turn], length_m: f64, live: &[LapCall]) -> Vec<RadioEntry> {
    let said = |n: i32| live.iter().find(|c| c.kind == CallKind::Lap && c.lap_number == n);
    let mut entries: Vec<RadioEntry> = laps
        .iter()
        .enumerate()
        .filter_map(|(i, lap)| match said(lap.lap_number) {
            Some(call) => Some(RadioEntry { call: call.clone(), live: true }),
            None => {
                let text = call_for(&laps[..=i], traces, lap, turns, length_m)?;
                Some(RadioEntry { call: LapCall { lap_number: lap.lap_number, kind: CallKind::Lap, text }, live: false })
            }
        })
        .collect();
    entries.extend(live.iter().filter(|c| c.kind == CallKind::Pit).map(|call| RadioEntry { call: call.clone(), live: true }));
    entries.sort_by_key(|e| (e.call.lap_number, e.call.kind));
    entries
}

fn corner_time(s: &PhaseStats) -> f64 {
    s.entry_time_s + s.exit_time_s
}

/// Index of the turn whose stretch of track contains `pct`.
fn turn_index_at(turns: &[Turn], pct: f64) -> Option<usize> {
    turn_segments(turns).iter().position(|(start, _, end)| pct >= *start && pct <= *end)
}

/// Complete laps that stayed on the track: the only fair comparisons.
fn is_clean(lap: &LapMetrics) -> bool {
    lap.is_complete && lap.off_track_pcts.is_empty()
}

fn call_for(laps: &[LapMetrics], traces: &[LapTrace], lap: &LapMetrics, turns: &[Turn], length_m: f64) -> Option<String> {
    let off_at = lap.off_track_pcts.first().map(|&pct| turn_index_at(turns, pct).map(|i| format!("Turn {}", turns[i].label)));
    let where_off = |at: &Option<String>| at.as_ref().map(|t| format!(" at {t}")).unwrap_or_default();

    if !lap.is_complete {
        // Pit laps and resets aren't worth a call; a lap lost to track limits is. Untimed laps
        // have no trace, so there's nothing more to say about them.
        let at = off_at?;
        return Some(format!("That one's gone, you were off{}. Reset and go again.", where_off(&at)));
    }

    let earlier: Vec<&LapMetrics> = laps.iter().filter(|l| is_clean(l) && l.lap_number != lap.lap_number).collect();
    let best_before = earlier.iter().map(|l| l.lap_time_s).fold(f64::INFINITY, f64::min);
    // Only laps on a similar surface say anything about the driving: half a second lost on a
    // wet lap against a dry one is the weather, not the driver.
    let others: Vec<&LapMetrics> = earlier.iter().copied().filter(|l| l.same_conditions(lap)).collect();
    let changed = match earlier.last() {
        Some(prev) if others.is_empty() => Some(if lap.track_wetness > prev.track_wetness {
            "Track's wetter now, so that's the new baseline."
        } else {
            "Track's drying, so that's the new baseline."
        }),
        _ => None,
    };

    // The corner where the car went off, and the one after it (rejoining), obviously lost time:
    // the advice looks at the rest of the lap.
    let skip: Vec<usize> = lap
        .off_track_pcts
        .iter()
        .filter_map(|&pct| turn_index_at(turns, pct))
        .flat_map(|i| [i, (i + 1) % turns.len().max(1)])
        .collect();

    let opener = match (&off_at, changed) {
        (Some(at), _) => Some(format!("You were off{}, that one won't count.", where_off(at))),
        (None, _) if best_before.is_finite() && lap.lap_time_s < best_before => Some("Purple lap.".to_string()),
        (None, Some(changed)) => Some(changed.to_string()),
        (None, None) => None,
    };

    let trace_of = |n: i32| traces.iter().find(|t| t.lap_number == n);
    let other_traces: Vec<&LapTrace> = others.iter().filter_map(|l| trace_of(l.lap_number)).collect();
    let focus = match (trace_of(lap.lap_number), turns.is_empty() || length_m <= 0.0) {
        (Some(trace), false) => {
            let peak_g = peak_combined_g(traces);
            if other_traces.is_empty() {
                first_lap_focus(trace, turns, &skip, length_m, peak_g)
            } else {
                biggest_loss(trace, &other_traces, turns, &skip, length_m, peak_g)
            }
        }
        _ => None,
    };

    Some(match (opener, focus) {
        (Some(opener), Some(focus)) if off_at.is_some() => format!("{opener} Still, {}", mid_sentence(&focus)),
        (Some(opener), Some(focus)) => format!("{opener} {focus}"),
        (None, Some(focus)) => focus,
        (Some(opener), None) if off_at.is_some() => opener,
        (Some(opener), None) if other_traces.is_empty() => format!("{opener} Build from there."),
        (Some(opener), None) => format!("{opener} Nothing big to find, keep that rhythm."),
        (None, None) if other_traces.is_empty() => "Good start. Build from there.".to_string(),
        (None, None) => "Clean lap, nothing big to find. Keep that rhythm.".to_string(),
    })
}

/// `text` continuing a sentence: "Lots of ABS…" becomes "lots of ABS…"; names like "Turn 4" keep their capital.
fn mid_sentence(text: &str) -> String {
    match text.chars().next() {
        Some(c) if c.is_uppercase() && !text.starts_with("Turn ") => c.to_lowercase().chain(text.chars().skip(1)).collect(),
        _ => text.to_string(),
    }
}

/// With nothing to compare against yet: the corner with the most coasting, or the heaviest ABS.
fn first_lap_focus(trace: &LapTrace, turns: &[Turn], skip: &[usize], length_m: f64, peak_g: Option<f64>) -> Option<String> {
    let stats: Vec<(&Turn, PhaseStats)> = turn_segments(turns)
        .into_iter()
        .zip(turns)
        .enumerate()
        .filter(|(i, _)| !skip.contains(i))
        .map(|(_, (seg, turn))| (turn, phase_stats(trace, seg, length_m, peak_g)))
        .collect();
    let coasting = stats.iter().max_by(|a, b| a.1.coast_m.total_cmp(&b.1.coast_m)).filter(|(_, s)| s.coast_m >= 15.0);
    if let Some((turn, _)) = coasting {
        return Some(format!("Coasting into Turn {}. Get from brake to throttle sooner.", turn.label));
    }
    let abs = stats.iter().filter_map(|(t, s)| Some((t, s.abs_pct?))).max_by(|a, b| a.1.total_cmp(&b.1)).filter(|(_, pct)| *pct >= 40.0);
    abs.map(|(turn, _)| format!("Lots of ABS into Turn {}. Ease the peak pressure a touch.", turn.label))
}

/// "0.30s lost in Turn 5, your line's too tight there, so the apex speed drops. Use more of the track…"
fn loss_line(loss: f64, turn: &Turn, typical: &PhaseStats, best: &PhaseStats) -> String {
    let (why, what) = reason(typical, best);
    format!("{loss:.2}s lost in Turn {}, {why}. {what}", turn.label)
}

/// The corner where this lap lost the most against the best pass through it on another lap,
/// and the one thing that made the difference.
fn biggest_loss(trace: &LapTrace, others: &[&LapTrace], turns: &[Turn], skip: &[usize], length_m: f64, peak_g: Option<f64>) -> Option<String> {
    let (turn, loss, mine, best) = turn_segments(turns)
        .into_iter()
        .zip(turns)
        .enumerate()
        .filter(|(i, _)| !skip.contains(i))
        .filter_map(|(_, (seg, turn))| {
            let mine = phase_stats(trace, seg, length_m, peak_g);
            let best = others.iter().map(|t| phase_stats(t, seg, length_m, peak_g)).min_by(|a, b| corner_time(a).total_cmp(&corner_time(b)))?;
            Some((turn, corner_time(&mine) - corner_time(&best), mine, best))
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))?;
    (loss >= MIN_LOSS_S).then(|| loss_line(loss, turn, &mine, &best))
}

/// Why `mine` was slower than `best` through a corner, and what to do: the difference
/// furthest past its noise threshold. Words only, no numbers.
fn reason(mine: &PhaseStats, best: &PhaseStats) -> (&'static str, &'static str) {
    let mut options: Vec<(f64, &'static str, &'static str)> = Vec::new();

    if let (Some(m), Some(b)) = (mine.brake_point_m, best.brake_point_m) {
        // Metres before the apex: larger is earlier.
        let earlier = m - b;
        if earlier >= 6.0 {
            options.push((earlier / 6.0, "you're braking earlier than your best there", "Go deeper on the brakes."));
        }
    }
    let slower = best.min_speed_kph - mine.min_speed_kph;
    if slower >= 2.0 {
        // A lower apex speed is a symptom: name what caused it when the telemetry shows it.
        let (why, what) = slow_apex_cause(mine, best).unwrap_or(("you're slower through the middle", "Carry more speed to the apex."));
        options.push((slower / 2.0, why, what));
    }
    if let (Some(m), Some(b)) = (mine.throttle_on_m, best.throttle_on_m) {
        let later = m - b;
        if later >= 6.0 {
            options.push((later / 6.0, "you're late back on the power", "Pick up the throttle earlier."));
        }
    }
    let coast = mine.coast_m - best.coast_m;
    if coast >= 10.0 {
        options.push((coast / 10.0, "too much coasting", "Roll straight from brake to throttle."));
    }
    if let (Some(m), Some(b)) = (mine.abs_pct, best.abs_pct) {
        if m - b >= 15.0 {
            options.push(((m - b) / 15.0, "too much ABS on the way in", "Ease the peak brake pressure."));
        }
    }
    // Balance grows with cornering load, so the threshold scales with the better lap's value.
    let balance = [("entry", mine.entry_balance_deg, best.entry_balance_deg), ("exit", mine.exit_balance_deg, best.exit_balance_deg)];
    for (phase, m, b) in balance {
        let (Some(m), Some(b)) = (m, b) else { continue };
        let threshold = (0.25 * b.abs()).max(3.0);
        let diff = m - b;
        if diff.abs() < threshold {
            continue;
        }
        let (why, what) = match (phase, diff > 0.0) {
            ("entry", true) => ("understeer on the way in", "Trail the brake closer to the apex to keep the nose loaded."),
            ("entry", false) => ("the rear's moving on entry", "Come off the brake more smoothly as you turn in."),
            (_, true) => ("understeer on exit", "Let it rotate before you go to the throttle."),
            _ => ("the rear's stepping out on exit", "Squeeze the throttle in, don't stab it."),
        };
        options.push((diff.abs() / threshold, why, what));
    }

    if let Some((_, why, what)) = options.into_iter().max_by(|a, b| a.0.total_cmp(&b.0)) {
        return (why, what);
    }
    if mine.entry_time_s - best.entry_time_s >= mine.exit_time_s - best.exit_time_s {
        ("it's all on the way in", "Compare your entry with your best.")
    } else {
        ("it's all on the exit", "Compare your exit with your best.")
    }
}

const ORDINALS: [&str; 3] = ["First", "Second", "Third"];

/// The stint debrief: how the pace went, where a typical lap gives time away against the best
/// pass through each corner (up to three corners, with what to change), and where the car
/// keeps going off.
fn debrief(laps: &[LapMetrics], traces: &[LapTrace], turns: &[Turn], length_m: f64) -> String {
    let mut out = vec!["Okay, you're in. Here's the debrief.".to_string()];

    // Compare like with like: the clean laps on the surface the next stint will start on (the
    // latest clean lap's), not the dry ones against the wet ones.
    let all_clean: Vec<&LapMetrics> = laps.iter().filter(|l| is_clean(l)).collect();
    let clean: Vec<&LapMetrics> = match all_clean.last() {
        Some(latest) => all_clean.iter().copied().filter(|l| l.same_conditions(latest)).collect(),
        None => Vec::new(),
    };
    if clean.len() < all_clean.len() {
        out.push("Conditions changed during the stint, so this only looks at the laps in the current conditions.".into());
    }
    let clean_traces: Vec<&LapTrace> = clean.iter().filter_map(|l| traces.iter().find(|t| t.lap_number == l.lap_number)).collect();

    // Where the car went off, most often first.
    let mut offs: Vec<(usize, usize)> = Vec::new();
    for pct in laps.iter().flat_map(|l| &l.off_track_pcts) {
        if let Some(i) = turn_index_at(turns, *pct) {
            match offs.iter_mut().find(|(t, _)| *t == i) {
                Some((_, n)) => *n += 1,
                None => offs.push((i, 1)),
            }
        }
    }
    offs.sort_by_key(|o| std::cmp::Reverse(o.1));

    if clean_traces.len() < 2 || turns.is_empty() || length_m <= 0.0 {
        out.push("Not enough clean laps to compare yet. Next time out, string a few clean ones together and we'll have more for you.".into());
    } else {
        let times: Vec<f64> = clean.iter().map(|l| l.lap_time_s).collect();
        let best_lap = clean.iter().min_by(|a, b| a.lap_time_s.total_cmp(&b.lap_time_s)).expect("two clean laps");
        let mean = times.iter().sum::<f64>() / times.len() as f64;
        let spread = (times.iter().map(|t| (t - mean).powi(2)).sum::<f64>() / times.len() as f64).sqrt();
        out.push(format!("Best lap was lap {}.", best_lap.lap_number));
        if clean.len() >= 3 && spread < 0.3 {
            out.push("Nice and consistent.".into());
        } else if spread > 0.8 {
            out.push("Lap times are moving around a lot, so consistency first.".into());
        }

        // Per corner: the best pass, and how far a typical lap is off it.
        let peak_g = peak_combined_g(traces);
        let mut corners: Vec<(f64, &Turn, PhaseStats, PhaseStats)> = turn_segments(turns)
            .into_iter()
            .zip(turns)
            .filter_map(|(seg, turn)| {
                let stats: Vec<PhaseStats> = clean_traces.iter().map(|t| phase_stats(t, seg, length_m, peak_g)).collect();
                let best_i = (0..stats.len()).min_by(|&a, &b| corner_time(&stats[a]).total_cmp(&corner_time(&stats[b])))?;
                let mut corner_times: Vec<f64> = stats.iter().map(corner_time).collect();
                let gap = median(&mut corner_times)? - corner_time(&stats[best_i]);
                let rest: Vec<PhaseStats> = stats.iter().enumerate().filter(|(i, _)| *i != best_i).map(|(_, s)| s.clone()).collect();
                Some((gap, turn, median_stats(&rest), stats[best_i].clone()))
            })
            .collect();
        let on_the_table: f64 = corners.iter().map(|c| c.0.max(0.0)).sum();
        corners.sort_by(|a, b| b.0.total_cmp(&a.0));
        corners.retain(|c| c.0 >= MIN_LOSS_S);
        corners.truncate(ORDINALS.len());

        if on_the_table >= 0.1 {
            out.push(format!("A typical lap leaves {on_the_table:.1}s in the corners against your best through each one."));
        }
        if corners.is_empty() {
            out.push("No corner stands out, it's about stringing it all together now.".into());
        } else {
            let things = ["One thing", "Two things", "Three things"][corners.len() - 1];
            out.push(format!("{things} for next time out."));
            let mut said: Vec<&str> = Vec::new();
            for (k, (_, turn, typical, best)) in corners.iter().enumerate() {
                let (why, what) = reason(typical, best);
                let lead = if corners.len() == 1 { String::new() } else { format!("{}, ", ORDINALS[k]) };
                if said.contains(&why) {
                    out.push(format!("{lead}Turn {}: same again, {}", turn.label, what.to_lowercase()));
                } else {
                    out.push(format!("{lead}Turn {}: {why}. {what}", turn.label));
                    said.push(why);
                }
            }
        }
    }

    if let Some(&(i, n)) = offs.first() {
        let times = match n {
            1 => "once".to_string(),
            2 => "twice".to_string(),
            n => format!("{n} times"),
        };
        out.push(format!("And watch Turn {}, you went off there {times}.", turns[i].label));
    }
    out.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telemetry::ibt::read_ibt;
    use crate::telemetry::trace::{build_run, detect_turns};
    use std::path::Path;

    fn fixture() -> (RunData, Vec<Turn>, f64) {
        let data = read_ibt(Path::new("tests/fixtures/roadatlanta-full.ibt")).expect("fixture");
        let run = build_run(&data.frames, &data.track);
        let turns = detect_turns(run.map_points.as_ref().expect("outline"), data.track.length_m);
        (run, turns, data.track.length_m)
    }

    /// Numbers in a call, not counting turn labels.
    fn numbers(text: &str) -> usize {
        text.replace("Turn ", "Turn_").split_whitespace().filter(|w| !w.starts_with("Turn_") && w.chars().any(|c| c.is_ascii_digit())).count()
    }

    #[test]
    fn transcript_calls_every_lap_from_the_laps_so_far() {
        let (run, turns, length_m) = fixture();
        let complete: Vec<&LapMetrics> = run.laps.iter().filter(|l| l.is_complete).collect();
        assert!(complete.len() >= 3);

        let live = [LapCall { lap_number: complete[1].lap_number, kind: CallKind::Lap, text: "Said live.".to_string() }];
        let lines = transcript(&run.laps, &run.traces, &turns, length_m, &live);
        let line = |n: i32| lines.iter().find(|l| l.call.lap_number == n).expect("line for lap");

        let first = line(complete[0].lap_number);
        assert!(!first.live && !first.call.text.starts_with("Purple"), "{}", first.call.text);
        let said = line(complete[1].lap_number);
        assert!(said.live && said.call.text == "Said live.");
        for lap in &complete[2..] {
            let text = &line(lap.lap_number).call.text;
            // One number at most: the time lost.
            assert!(numbers(text) <= 1, "{text}");
            assert!(text.contains("lost in Turn ") || text.contains("nothing big"), "{text}");
            assert!(text.len() < 180, "too long for the radio: {text}");
        }
    }

    #[test]
    fn off_track_laps_still_get_advice_away_from_the_off() {
        let (mut run, turns, length_m) = fixture();
        let last = run.laps.iter().rposition(|l| l.is_complete).expect("timed lap");
        let off_turn = turns[2].clone();
        run.laps[last].off_track_pcts = vec![off_turn.pct];

        let text = call_for(&run.laps, &run.traces, &run.laps[last], &turns, length_m).expect("call");
        let opener = format!("You were off at Turn {}, that one won't count.", off_turn.label);
        assert!(text.starts_with(&opener), "{text}");
        if let Some(advice) = text.strip_prefix(&opener).and_then(|rest| rest.strip_prefix(" Still, ")) {
            assert!(!advice.contains(&format!("in Turn {},", off_turn.label)), "{text}");
            assert!(!advice.contains(&format!("in Turn {},", turns[3].label)), "{text}");
        }
    }

    #[test]
    fn pit_debrief_names_corners_to_work_on() {
        let (mut run, turns, length_m) = fixture();
        let first = run.laps.iter().position(|l| l.is_complete).expect("timed lap");
        run.laps[first].off_track_pcts = vec![turns[1].pct];

        let call = pit_debrief(&run, &turns, length_m).expect("debrief");
        assert_eq!(call.kind, CallKind::Pit);
        let text = &call.text;
        assert!(text.starts_with("Okay, you're in."), "{text}");
        assert!(text.contains("for next time out.") || text.contains("No corner stands out"), "{text}");
        assert!(text.contains(&format!("And watch Turn {}, you went off there once.", turns[1].label)), "{text}");
    }

    #[test]
    fn pit_debrief_without_clean_laps_says_so() {
        let (mut run, turns, length_m) = fixture();
        run.laps.iter_mut().for_each(|l| l.is_complete = false);
        let text = pit_debrief(&run, &turns, length_m).expect("debrief").text;
        assert!(text.contains("Not enough clean laps"), "{text}");
    }

    #[test]
    fn lap_call_is_about_the_lap_that_just_ended() {
        let (mut run, turns, length_m) = fixture();
        // The fixture ends partway round a lap: end on the last timed one instead.
        let timed = run.laps.iter().rposition(|l| l.is_complete).expect("timed lap");
        run.laps.truncate(timed + 1);
        let last = run.laps.last().expect("laps").lap_number;
        // Snapshot without the next lap's first seconds: the last lap is the one that ended.
        assert_eq!(lap_call(&run, &turns, length_m).expect("call").lap_number, last);
        // With them (a short track): still the lap that ended.
        let mut stub = run.laps.last().expect("laps").clone();
        (stub.lap_number, stub.is_complete, stub.lap_time_s) = (last + 1, false, 2.5);
        run.laps.push(stub);
        assert_eq!(lap_call(&run, &turns, length_m).expect("call").lap_number, last);
        // The pit debrief is filed under the last lap that had a call, not the in-lap.
        assert_eq!(pit_debrief(&run, &turns, length_m).expect("debrief").lap_number, last);
    }

    #[test]
    fn wet_and_dry_laps_are_not_compared() {
        let (mut run, turns, length_m) = fixture();
        let timed = run.laps.iter().rposition(|l| l.is_complete).expect("timed lap");
        run.laps.truncate(timed + 1);
        // Every earlier lap dry, the last one wet: nothing to compare it with.
        run.laps.iter_mut().for_each(|l| l.track_wetness = Some(1));
        run.laps[timed].track_wetness = Some(5);
        run.laps[timed].lap_time_s += 5.0;

        let call = lap_call(&run, &turns, length_m).expect("call").text;
        assert!(call.starts_with("Track's wetter now, so that's the new baseline."), "{call}");
        assert!(!call.contains("lost in Turn"), "{call}");

        let debrief = pit_debrief(&run, &turns, length_m).expect("debrief").text;
        assert!(debrief.contains("Conditions changed during the stint"), "{debrief}");
        assert!(debrief.contains("Not enough clean laps"), "{debrief}");
    }
}
