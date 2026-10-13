//! The other drivers in a session: who they are (the roster and results from iRacing's
//! session info, which imports have too) and, for runs recorded live, their laps rebuilt from
//! iRacing's per-car channels, so the driver can see where the quicker cars gain time.
//!
//! iRacing only publishes other cars' lap distance, gear, RPM and pit / surface state, never
//! their pedals or line, and only to live telemetry: `.ibt` files have none of it. Speed,
//! where a car starts slowing for a corner and its apex speed are derived from how its lap
//! distance changes over time, the same way for the player's laps when they're compared.

use serde::{Deserialize, Serialize};

pub mod capture;
pub mod compare;
pub mod roster;

/// One car in the session, from the session info's driver list.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FieldDriver {
    pub car_idx: i32,
    pub user_id: i64,
    pub name: String,
    /// The car's screen name, e.g. "Porsche 911 GT3 R (992)".
    pub car: String,
    pub class_id: i64,
    /// The class's short name, e.g. "GT3 Class". Empty in single-class sessions.
    pub class_name: String,
    pub irating: i64,
    /// "A 3.45" style licence string.
    pub license: String,
    pub is_player: bool,
}

/// A car's standing in the session, from the session info's results.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FieldResult {
    pub car_idx: i32,
    pub position: i32,
    pub class_position: i32,
    /// `None` until the car has set a timed lap.
    pub fastest_time_s: Option<f64>,
    pub fastest_lap: i32,
    pub laps_complete: i32,
}

/// Who was in the session and how they stood: everything the session info says about the
/// field, for imports and live runs alike.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Roster {
    /// iRacing's subsession id (0 offline), with `session_num` identifying the session.
    pub sub_session_id: i64,
    pub session_num: i32,
    pub player_car_idx: i32,
    /// Real drivers only: spectators and the pace car are left out.
    pub drivers: Vec<FieldDriver>,
    pub results: Vec<FieldResult>,
}

impl Roster {
    pub fn player(&self) -> Option<&FieldDriver> {
        self.drivers.iter().find(|d| d.car_idx == self.player_car_idx)
    }

    pub fn driver(&self, car_idx: i32) -> Option<&FieldDriver> {
        self.drivers.iter().find(|d| d.car_idx == car_idx)
    }

    /// Whether `car_idx` races in the player's class (everyone does in a single-class session).
    /// In multi-class series (IMSA and the like) the other classes are much quicker or slower
    /// by design, so they're never compared with.
    pub fn in_player_class(&self, car_idx: i32) -> bool {
        match (self.player(), self.driver(car_idx)) {
            (Some(me), Some(them)) => me.same_class(them),
            _ => false,
        }
    }
}

impl FieldDriver {
    /// Same car class. iRacing always gives a class id; when it's missing (0, from incomplete
    /// session info) only the very same car counts, rather than lumping unknowns together.
    pub fn same_class(&self, other: &FieldDriver) -> bool {
        if self.class_id > 0 || other.class_id > 0 {
            self.class_id == other.class_id
        } else {
            !self.car.is_empty() && self.car == other.car
        }
    }
}

/// One lap by another car, rebuilt from live per-car channels and resampled onto the same
/// lap-distance grid as the player's traces (`n + 1` points from 0.0 to 1.0).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpponentLap {
    pub car_idx: i32,
    pub lap_number: i32,
    /// iRacing's time for the lap when it came through, else from the line crossings.
    pub lap_time_s: f64,
    /// Driven from line to line on track: no pit lane, off-track or gap in the data.
    pub valid: bool,
    /// The wettest the track got during the lap (`TrackWetness`, 1 dry … 7 extremely wet).
    pub track_wetness: Option<u8>,
    /// Seconds since the lap started, at each grid point.
    pub time_s: Vec<f32>,
    /// Gear at each grid point (0 when unknown).
    pub gear: Vec<i8>,
}

/// The rest of the field for one recording: the roster, and the other cars' laps when it was
/// recorded live.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Field {
    pub roster: Roster,
    pub laps: Vec<OpponentLap>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn driver(car_idx: i32, class_id: i64, car: &str) -> FieldDriver {
        FieldDriver { car_idx, class_id, car: car.into(), is_player: car_idx == 0, ..Default::default() }
    }

    #[test]
    fn only_the_players_class_is_compared() {
        // IMSA-style: the player in GT3, with GTP and LMP2 cars around.
        let roster = Roster {
            player_car_idx: 0,
            drivers: vec![driver(0, 3, "GT3 A"), driver(1, 3, "GT3 B"), driver(2, 1, "GTP"), driver(3, 2, "LMP2")],
            ..Default::default()
        };
        assert!(roster.in_player_class(1));
        assert!(!roster.in_player_class(2) && !roster.in_player_class(3));
        // A car not in the roster (joined since it was read) is never assumed to be a rival.
        assert!(!roster.in_player_class(9));
    }

    #[test]
    fn a_missing_class_id_only_matches_the_same_car() {
        assert!(driver(0, 0, "GT3 A").same_class(&driver(1, 0, "GT3 A")));
        assert!(!driver(0, 0, "GT3 A").same_class(&driver(1, 0, "GTP")));
        assert!(!driver(0, 0, "").same_class(&driver(1, 0, "")));
        assert!(!driver(0, 0, "GT3 A").same_class(&driver(1, 3, "GT3 A")));
    }
}
