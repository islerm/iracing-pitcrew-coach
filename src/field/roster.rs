//! The roster and results from iRacing's session info.

use serde::Deserialize;

use super::{FieldDriver, FieldResult, Roster};

#[derive(Deserialize, Default)]
#[serde(default)]
struct Doc {
    #[serde(rename = "WeekendInfo")]
    weekend: Option<Weekend>,
    #[serde(rename = "SessionInfo")]
    session: Option<SessionInfo>,
    #[serde(rename = "DriverInfo")]
    drivers: Option<DriverInfo>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Weekend {
    #[serde(rename = "SubSessionID")]
    sub_session_id: i64,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "PascalCase")]
struct SessionInfo {
    current_session_num: Option<i32>,
    sessions: Option<Vec<Session>>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "PascalCase")]
struct Session {
    session_num: i32,
    results_positions: Option<Vec<Position>>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "PascalCase")]
struct Position {
    position: i32,
    class_position: i32,
    car_idx: i32,
    fastest_lap: i32,
    fastest_time: f64,
    laps_complete: i32,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "PascalCase")]
struct DriverInfo {
    driver_car_idx: i32,
    drivers: Option<Vec<Driver>>,
}

#[derive(Deserialize, Default)]
#[serde(default, rename_all = "PascalCase")]
struct Driver {
    car_idx: i32,
    user_name: String,
    #[serde(rename = "UserID")]
    user_id: i64,
    car_screen_name: String,
    #[serde(rename = "CarClassID")]
    car_class_id: i64,
    car_class_short_name: Option<String>,
    i_rating: i64,
    lic_string: String,
    is_spectator: i32,
    car_is_pace_car: i32,
}

/// Keys whose values iRacing writes unquoted and can contain YAML-breaking characters.
const SANITIZE_KEYS: [&str; 8] = [
    "UserName", "TeamName", "AbbrevName", "Initials", "ClubName", "DivisionName",
    "CarDesignStr", "CarNumber",
];

/// Rewrites the values of the free-text keys into single-quoted YAML strings (iRacing's own
/// advice), since names like `O'Brien: Racing` or `#1 [Team]` are not valid plain scalars.
fn sanitize(yaml: &str) -> String {
    let mut out = String::with_capacity(yaml.len() + 256);
    for line in yaml.lines() {
        let body = line.trim_start();
        let indent = &line[..line.len() - body.len()];
        // List items put the first key after "- ".
        let (dash, rest) = match body.strip_prefix("- ") {
            Some(r) => ("- ", r),
            None => ("", body),
        };
        let quoted = rest.split_once(':').filter(|(key, value)| {
            SANITIZE_KEYS.contains(key) && (value.is_empty() || value.starts_with(' '))
        });
        match quoted {
            Some((key, value)) => {
                out.push_str(indent);
                out.push_str(dash);
                out.push_str(key);
                out.push_str(": '");
                out.push_str(&value.trim().replace('\'', "''"));
                out.push('\'');
            }
            None => out.push_str(line),
        }
        out.push('\n');
    }
    out
}

/// Builds the roster from the session info YAML of an `.ibt` file.
///
/// Uses `session_num` if given, else `CurrentSessionNum`, else the last session listed.
/// `class_position` is stored as iRacing gives it, which is 0-based in some versions and
/// 1-based in others. Returns an empty roster if the YAML can't be parsed.
pub fn from_session_yaml(yaml: &str, session_num: Option<i32>) -> Roster {
    let Ok(doc) = serde_yaml::from_str::<Doc>(&sanitize(yaml)) else {
        return Roster::default();
    };
    let info = doc.session.unwrap_or_default();
    let sessions = info.sessions.unwrap_or_default();
    let wanted = session_num.or(info.current_session_num);
    let session = wanted
        .and_then(|n| sessions.iter().find(|s| s.session_num == n))
        .or_else(|| sessions.last());

    let driver_info = doc.drivers.unwrap_or_default();
    let player = driver_info.driver_car_idx;
    let drivers: Vec<FieldDriver> = driver_info
        .drivers
        .unwrap_or_default()
        .into_iter()
        .filter(|d| d.is_spectator == 0 && d.car_is_pace_car == 0 && d.user_name != "Pace Car")
        .map(|d| FieldDriver {
            car_idx: d.car_idx,
            user_id: d.user_id,
            name: d.user_name,
            car: d.car_screen_name,
            class_id: d.car_class_id,
            class_name: d.car_class_short_name.unwrap_or_default(),
            irating: d.i_rating,
            license: d.lic_string,
            is_player: d.car_idx == player,
        })
        .collect();

    let results = session
        .and_then(|s| s.results_positions.as_ref())
        .map(|rs| {
            rs.iter()
                .filter(|r| drivers.iter().any(|d| d.car_idx == r.car_idx))
                .map(|r| FieldResult {
                    car_idx: r.car_idx,
                    position: r.position,
                    class_position: r.class_position,
                    fastest_time_s: (r.fastest_time > 0.0).then_some(r.fastest_time),
                    fastest_lap: r.fastest_lap,
                    laps_complete: r.laps_complete,
                })
                .collect()
        })
        .unwrap_or_default();

    Roster {
        sub_session_id: doc.weekend.unwrap_or_default().sub_session_id,
        session_num: session.map(|s| s.session_num).or(wanted).unwrap_or(0),
        player_car_idx: player,
        drivers,
        results,
    }
}

/// Builds the roster from a live session's parsed session info. The crate has no pace-car
/// flag, so the pace car is recognised by name and by having no user id.
#[cfg(windows)]
pub fn from_session_details(
    details: &iracing::session::SessionDetails,
    session_num: Option<i32>,
) -> Roster {
    let player = details.drivers.car_index as i32;
    let sessions = &details.session.sessions;
    let session = session_num
        .and_then(|n| sessions.iter().find(|s| s.session_number as i32 == n))
        .or_else(|| sessions.last());

    let drivers: Vec<FieldDriver> = details
        .drivers
        .other_drivers
        .iter()
        .filter(|d| {
            let is_player = d.index as i32 == player;
            d.is_spectator == 0 && d.user_name != "Pace Car" && (is_player || d.user_id > 0)
        })
        .map(|d| FieldDriver {
            car_idx: d.index as i32,
            user_id: d.user_id,
            name: d.user_name.clone(),
            car: d.car_screen_name.clone(),
            class_id: d.car_class_id as i64,
            class_name: d.car_class_short_name.clone(),
            irating: d.i_rating,
            license: d.license.clone(),
            is_player: d.index as i32 == player,
        })
        .collect();

    let results = session
        .and_then(|s| s.results.as_ref())
        .map(|rs| {
            rs.iter()
                .filter(|r| drivers.iter().any(|d| d.car_idx == r.car_idx))
                .map(|r| FieldResult {
                    car_idx: r.car_idx,
                    position: r.position,
                    class_position: r.class_position,
                    fastest_time_s: (r.fastest_time > 0.0).then_some(r.fastest_time as f64),
                    fastest_lap: r.fastest_lap,
                    laps_complete: r.laps_complete,
                })
                .collect()
        })
        .unwrap_or_default();

    Roster {
        sub_session_id: details.weekend.sub_session_id as i64,
        session_num: session.map(|s| s.session_number as i32).or(session_num).unwrap_or(0),
        player_car_idx: player,
        drivers,
        results,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const YAML: &str = r#"---
WeekendInfo:
 TrackName: nurburgring
 SubSessionID: 1234
SessionInfo:
 CurrentSessionNum: 1
 Sessions:
 - SessionNum: 0
   SessionType: Practice
   ResultsPositions:
   - Position: 1
     ClassPosition: 0
     CarIdx: 2
     FastestLap: 3
     FastestTime: 400.5
     LapsComplete: 4
 - SessionNum: 1
   SessionType: Race
   ResultsPositions:
   - Position: 1
     ClassPosition: 0
     CarIdx: 2
     FastestLap: 5
     FastestTime: 101.25
     LapsComplete: 10
   - Position: 2
     ClassPosition: 0
     CarIdx: 1
     FastestLap: 6
     FastestTime: 110.5
     LapsComplete: 10
   - Position: 3
     ClassPosition: 1
     CarIdx: 3
     FastestLap: -1
     FastestTime: -1.0000
     LapsComplete: 0
   - Position: 4
     ClassPosition: 2
     CarIdx: 4
     FastestLap: 1
     FastestTime: 50.0
     LapsComplete: 1
   - Position: 5
     ClassPosition: 3
     CarIdx: 0
     FastestLap: 1
     FastestTime: 50.0
     LapsComplete: 1
DriverInfo:
 DriverCarIdx: 1
 Drivers:
 - CarIdx: 0
   UserName: Pace Car
   UserID: -1
   IsSpectator: 0
   CarIsPaceCar: 1
 - CarIdx: 1
   UserName: Me Myself
   TeamName: #1 [Team]: Foo
   UserID: 100
   CarScreenName: Porsche 911 GT3 R (992)
   CarClassID: 11
   CarClassShortName: GT3 Class
   IRating: 2500
   LicString: A 3.45
   IsSpectator: 0
   CarIsPaceCar: 0
 - CarIdx: 2
   UserName: O'Brien: Racing
   UserID: 200
   CarScreenName: Ferrari 296 GT3
   CarClassID: 11
   CarClassShortName: GT3 Class
   IRating: 3000
   LicString: A 4.10
   IsSpectator: 0
 - CarIdx: 3
   UserName: Slow Guy
   UserID: 300
   CarScreenName: Mazda MX-5
   CarClassID: 22
   CarClassShortName: MX5
   IRating: 1200
   LicString: C 2.00
   IsSpectator: 0
 - CarIdx: 4
   UserName: Watcher
   UserID: 400
   IsSpectator: 1
"#;

    #[test]
    fn parses_multiclass_roster() {
        let r = from_session_yaml(YAML, None);
        assert_eq!((r.sub_session_id, r.session_num, r.player_car_idx), (1234, 1, 1));
        assert_eq!(r.drivers.len(), 3);
        assert_eq!(r.driver(2).unwrap().name, "O'Brien: Racing");
        assert!(r.driver(0).is_none() && r.driver(4).is_none());
        let me = r.player().unwrap();
        assert!(me.is_player && me.irating == 2500 && me.license == "A 3.45");
        assert_eq!(me.class_name, "GT3 Class");
        assert!(r.in_player_class(2) && !r.in_player_class(3));
        assert_eq!(r.results.len(), 3);
        assert_eq!(r.results[0].fastest_time_s, Some(101.25));
        assert_eq!(r.results[2].fastest_time_s, None);
        assert_eq!(r.results[2].class_position, 1);
    }

    #[test]
    fn selects_session() {
        let practice = from_session_yaml(YAML, Some(0));
        assert_eq!(practice.session_num, 0);
        assert_eq!(practice.results[0].fastest_time_s, Some(400.5));
        let no_current = YAML.replace(" CurrentSessionNum: 1\n", "");
        assert_eq!(from_session_yaml(&no_current, None).session_num, 1);
    }

    #[test]
    fn missing_or_null_results_are_empty() {
        let y = "SessionInfo:\n Sessions:\n - SessionNum: 0\n   ResultsPositions: ~\nDriverInfo:\n DriverCarIdx: 3\n";
        let r = from_session_yaml(y, None);
        assert!(r.results.is_empty() && r.drivers.is_empty());
        assert_eq!(r.player_car_idx, 3);
    }

    #[test]
    fn garbage_gives_default() {
        let r = from_session_yaml("{{{ not: [yaml", None);
        assert!(r.drivers.is_empty() && r.results.is_empty() && r.player_car_idx == 0);
    }

    #[test]
    fn fixtures_do_not_panic() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "ibt") {
                let yaml = crate::telemetry::ibt::read_session_yaml(&path).unwrap();
                let r = from_session_yaml(&yaml, None);
                println!("{}: {} drivers, player {}", path.display(), r.drivers.len(), r.player_car_idx);
            }
        }
    }

    #[test]
    #[ignore]
    fn real_nordschleife_file() {
        let path = std::path::Path::new("C:/Users/isler/OneDrive/Documents/iRacing/telemetry/streetstock_nurburgring nordschleife 2026-10-12 21-45-16.ibt");
        if !path.exists() {
            return;
        }
        let yaml = crate::telemetry::ibt::read_session_yaml(path).unwrap();
        let r = from_session_yaml(&yaml, None);
        let mut classes: Vec<_> = r.drivers.iter().map(|d| d.class_id).collect();
        classes.sort();
        classes.dedup();
        println!(
            "sub={} session={} player={} drivers={} classes={:?} results={}",
            r.sub_session_id, r.session_num, r.player_car_idx, r.drivers.len(), classes, r.results.len()
        );
        for res in r.results.iter().take(5) {
            println!("{res:?}");
        }
    }
}
