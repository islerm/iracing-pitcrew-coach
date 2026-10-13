//! A small SQLite store for the other drivers in iRacing sessions, so the field survives
//! restarts and builds up a library of other drivers' laps per track and car class.

use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

use crate::field::{Field, FieldDriver, FieldResult, OpponentLap, Roster};

/// Where the database lives unless told otherwise.
pub const DEFAULT_PATH: &str = "data/pitcrew.db";

/// Schema migrations, applied in order; `PRAGMA user_version` is the number already applied.
/// Append new entries rather than editing old ones.
const MIGRATIONS: &[&str] = &[
    // Version 1.
    "
    CREATE TABLE sessions (
        id INTEGER PRIMARY KEY,
        sub_session_id INTEGER NOT NULL,
        session_num INTEGER NOT NULL,
        session_type TEXT NOT NULL,
        track_name TEXT NOT NULL,
        track_id INTEGER,
        started_at_ms INTEGER NOT NULL
    );
    -- Online sessions are recorded once however often we reconnect; offline ones (sub_session_id
    -- 0) are never the same session twice, so they stay out of the index.
    CREATE UNIQUE INDEX sessions_online ON sessions(sub_session_id, session_num, track_name)
        WHERE sub_session_id > 0;
    CREATE INDEX sessions_track ON sessions(track_name);

    CREATE TABLE drivers (
        session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
        car_idx INTEGER NOT NULL,
        user_id INTEGER,
        name TEXT,
        car TEXT,
        class_id INTEGER,
        class_name TEXT,
        irating INTEGER,
        license TEXT,
        is_player INTEGER NOT NULL,
        PRIMARY KEY(session_id, car_idx)
    );
    CREATE INDEX drivers_class ON drivers(class_id);

    CREATE TABLE results (
        session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
        car_idx INTEGER NOT NULL,
        position INTEGER,
        class_position INTEGER,
        fastest_time_s REAL,
        fastest_lap INTEGER,
        laps_complete INTEGER,
        PRIMARY KEY(session_id, car_idx)
    );

    CREATE TABLE opponent_laps (
        id INTEGER PRIMARY KEY,
        session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
        car_idx INTEGER NOT NULL,
        lap_number INTEGER NOT NULL,
        lap_time_s REAL NOT NULL,
        valid INTEGER NOT NULL,
        track_wetness INTEGER,
        time_s BLOB NOT NULL,
        gear BLOB NOT NULL,
        UNIQUE(session_id, car_idx, lap_number)
    );
    CREATE INDEX opponent_laps_fastest ON opponent_laps(valid, lap_time_s);
    ",
];

pub struct Store {
    conn: Connection,
}

/// What identifies a recorded session.
pub struct SessionKey<'a> {
    /// iRacing's subsession id; 0 offline.
    pub sub_session_id: i64,
    pub session_num: i32,
    pub session_type: &'a str,
    pub track_name: &'a str,
    pub track_id: Option<u32>,
    pub started_at_ms: u128,
}

impl Store {
    /// Opens (creating if needed) the database at `path` and brings its schema up to date.
    pub fn open(path: &Path) -> Result<Store> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        // WAL lets the server read while a recording writes.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        Self::init(conn)
    }

    /// A throwaway database, for tests.
    #[cfg(test)]
    pub fn open_in_memory() -> Result<Store> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Store> {
        // Off by default in SQLite, and the cascades depend on it.
        conn.pragma_update(None, "foreign_keys", true)?;
        let mut store = Store { conn };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&mut self) -> Result<()> {
        let current: usize = self.conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if current > MIGRATIONS.len() {
            bail!("database schema v{current} is newer than this build understands (v{})", MIGRATIONS.len());
        }
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(current) {
            // Schema change and version bump commit together, so a failure leaves no half-migration.
            let tx = self.conn.transaction()?;
            tx.execute_batch(sql).with_context(|| format!("migrating to v{}", i + 1))?;
            tx.pragma_update(None, "user_version", i + 1)?;
            tx.commit()?;
        }
        Ok(())
    }

    /// The id of the session `key` names. Online sessions reuse their row; offline ones
    /// (`sub_session_id` 0) are always new recordings.
    pub fn upsert_session(&mut self, key: &SessionKey) -> Result<i64> {
        if key.sub_session_id > 0 {
            let existing = self
                .conn
                .query_row(
                    "SELECT id FROM sessions WHERE sub_session_id = ?1 AND session_num = ?2 AND track_name = ?3",
                    params![key.sub_session_id, key.session_num, key.track_name],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(id) = existing {
                return Ok(id);
            }
        }
        self.conn.execute(
            "INSERT INTO sessions (sub_session_id, session_num, session_type, track_name, track_id, started_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                key.sub_session_id,
                key.session_num,
                key.session_type,
                key.track_name,
                key.track_id,
                i64::try_from(key.started_at_ms).unwrap_or(i64::MAX),
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Stores the field for a session in one transaction: the roster replaces what was there
    /// (it's the latest session info), laps are added or replaced by (car, lap number).
    pub fn save_field(&mut self, session_id: i64, field: &Field) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM drivers WHERE session_id = ?1", [session_id])?;
        tx.execute("DELETE FROM results WHERE session_id = ?1", [session_id])?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO drivers (session_id, car_idx, user_id, name, car, class_id, class_name, irating, license, is_player)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            )?;
            for d in &field.roster.drivers {
                stmt.execute(params![
                    session_id,
                    d.car_idx,
                    d.user_id,
                    d.name,
                    d.car,
                    d.class_id,
                    d.class_name,
                    d.irating,
                    d.license,
                    d.is_player,
                ])?;
            }
            let mut stmt = tx.prepare(
                "INSERT INTO results (session_id, car_idx, position, class_position, fastest_time_s, fastest_lap, laps_complete)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            for r in &field.roster.results {
                stmt.execute(params![
                    session_id,
                    r.car_idx,
                    r.position,
                    r.class_position,
                    r.fastest_time_s,
                    r.fastest_lap,
                    r.laps_complete,
                ])?;
            }
            let mut stmt = tx.prepare(
                "INSERT OR REPLACE INTO opponent_laps
                    (session_id, car_idx, lap_number, lap_time_s, valid, track_wetness, time_s, gear)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )?;
            for l in &field.laps {
                stmt.execute(params![
                    session_id,
                    l.car_idx,
                    l.lap_number,
                    l.lap_time_s,
                    l.valid,
                    l.track_wetness,
                    encode_time(&l.time_s),
                    encode_gear(&l.gear),
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// The field stored for a session.
    #[allow(dead_code)] // for reading a stored session back; only tests use it so far
    pub fn load_field(&self, session_id: i64) -> Result<Field> {
        let (sub_session_id, session_num) = self
            .conn
            .query_row(
                "SELECT sub_session_id, session_num FROM sessions WHERE id = ?1",
                [session_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or_else(|| anyhow!("no session {session_id}"))?;

        let mut stmt = self.conn.prepare(&format!(
            "SELECT {DRIVER_COLS} FROM drivers WHERE session_id = ?1 ORDER BY car_idx"
        ))?;
        let drivers = stmt
            .query_map([session_id], |r| driver_from_row(r, 0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let mut stmt = self.conn.prepare(
            "SELECT car_idx, position, class_position, fastest_time_s, fastest_lap, laps_complete
             FROM results WHERE session_id = ?1 ORDER BY car_idx",
        )?;
        let results = stmt
            .query_map([session_id], |r| {
                Ok(FieldResult {
                    car_idx: r.get(0)?,
                    position: r.get::<_, Option<_>>(1)?.unwrap_or_default(),
                    class_position: r.get::<_, Option<_>>(2)?.unwrap_or_default(),
                    fastest_time_s: r.get(3)?,
                    fastest_lap: r.get::<_, Option<_>>(4)?.unwrap_or_default(),
                    laps_complete: r.get::<_, Option<_>>(5)?.unwrap_or_default(),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let mut stmt = self.conn.prepare(&format!(
            "SELECT {LAP_COLS} FROM opponent_laps l WHERE l.session_id = ?1 ORDER BY l.car_idx, l.lap_number"
        ))?;
        let laps = stmt
            .query_map([session_id], |r| lap_from_row(r, 0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .collect::<Result<Vec<_>>>()?;

        let player_car_idx = drivers.iter().find(|d| d.is_player).map_or(-1, |d| d.car_idx);
        Ok(Field {
            roster: Roster { sub_session_id, session_num, player_car_idx, drivers, results },
            laps,
        })
    }

    /// The fastest valid laps at `track_name` by drivers in `class_id`, from any session but
    /// `exclude_session`: people seen in earlier sessions to compare against when the current
    /// one has nobody else.
    pub fn library_laps(
        &self,
        track_name: &str,
        class_id: i64,
        exclude_session: Option<i64>,
        limit: usize,
    ) -> Result<Vec<(OpponentLap, FieldDriver)>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {LAP_COLS}, {DRIVER_COLS_D}
             FROM opponent_laps l
             JOIN sessions s ON s.id = l.session_id
             JOIN drivers d ON d.session_id = l.session_id AND d.car_idx = l.car_idx
             WHERE s.track_name = ?1 AND d.class_id = ?2 AND l.valid = 1
               AND (?3 IS NULL OR l.session_id != ?3)
             ORDER BY l.lap_time_s ASC, l.id ASC
             LIMIT ?4"
        ))?;
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let rows = stmt.query_map(params![track_name, class_id, exclude_session, limit], |r| {
            // The lap columns come first, then the driver's.
            Ok((lap_from_row(r, 0)?, driver_from_row(r, LAP_COL_COUNT)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (lap, driver) = row?;
            out.push((lap?, driver));
        }
        Ok(out)
    }
}

#[allow(dead_code)] // used by load_field
const DRIVER_COLS: &str = "car_idx, user_id, name, car, class_id, class_name, irating, license, is_player";
/// `DRIVER_COLS` qualified for queries that join `opponent_laps` (alias `l`) to `drivers` (alias `d`).
const DRIVER_COLS_D: &str =
    "d.car_idx, d.user_id, d.name, d.car, d.class_id, d.class_name, d.irating, d.license, d.is_player";
const LAP_COLS: &str = "l.car_idx, l.lap_number, l.lap_time_s, l.valid, l.track_wetness, l.time_s, l.gear";
const LAP_COL_COUNT: usize = 7;

/// A driver from nine consecutive `DRIVER_COLS` columns starting at `at`. Nullable columns
/// fall back to the type's default, as the in-memory types have no "unknown".
fn driver_from_row(r: &rusqlite::Row, at: usize) -> rusqlite::Result<FieldDriver> {
    Ok(FieldDriver {
        car_idx: r.get(at)?,
        user_id: r.get::<_, Option<_>>(at + 1)?.unwrap_or_default(),
        name: r.get::<_, Option<_>>(at + 2)?.unwrap_or_default(),
        car: r.get::<_, Option<_>>(at + 3)?.unwrap_or_default(),
        class_id: r.get::<_, Option<_>>(at + 4)?.unwrap_or_default(),
        class_name: r.get::<_, Option<_>>(at + 5)?.unwrap_or_default(),
        irating: r.get::<_, Option<_>>(at + 6)?.unwrap_or_default(),
        license: r.get::<_, Option<_>>(at + 7)?.unwrap_or_default(),
        is_player: r.get(at + 8)?,
    })
}

/// A lap from seven consecutive `LAP_COLS` columns starting at `at`. The inner `Result` is the
/// blob decoding, kept apart from SQLite's own errors.
fn lap_from_row(r: &rusqlite::Row, at: usize) -> rusqlite::Result<Result<OpponentLap>> {
    let car_idx = r.get(at)?;
    let lap_number = r.get(at + 1)?;
    let time = r.get::<_, Vec<u8>>(at + 5)?;
    let gear = r.get::<_, Vec<u8>>(at + 6)?;
    Ok(decode_time(&time).map(|time_s| OpponentLap {
        car_idx,
        lap_number,
        lap_time_s: r.get(at + 2).unwrap_or_default(),
        valid: r.get(at + 3).unwrap_or_default(),
        track_wetness: r.get(at + 4).unwrap_or_default(),
        time_s,
        gear: decode_gear(&gear),
    })
    .with_context(|| format!("car {car_idx} lap {lap_number}")))
}

/// `f32`s as little-endian bytes.
fn encode_time(time_s: &[f32]) -> Vec<u8> {
    time_s.iter().flat_map(|t| t.to_le_bytes()).collect()
}

fn decode_time(bytes: &[u8]) -> Result<Vec<f32>> {
    if !bytes.len().is_multiple_of(4) {
        bail!("corrupt time blob: {} bytes is not a multiple of 4", bytes.len());
    }
    Ok(bytes.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes(*c)).collect())
}

fn encode_gear(gear: &[i8]) -> Vec<u8> {
    gear.iter().map(|&g| g as u8).collect()
}

fn decode_gear(bytes: &[u8]) -> Vec<i8> {
    bytes.iter().map(|&b| b as i8).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(sub: i64, track: &str) -> SessionKey<'_> {
        SessionKey {
            sub_session_id: sub,
            session_num: 0,
            session_type: "Race",
            track_name: track,
            track_id: Some(7),
            started_at_ms: 1_700_000_000_000,
        }
    }

    fn driver(car_idx: i32, class_id: i64, is_player: bool) -> FieldDriver {
        FieldDriver {
            car_idx,
            user_id: 1000 + car_idx as i64,
            name: format!("Driver {car_idx}"),
            car: "Porsche 911 GT3 R (992)".into(),
            class_id,
            class_name: "GT3 Class".into(),
            irating: 2500,
            license: "A 3.45".into(),
            is_player,
        }
    }

    fn lap(car_idx: i32, lap_number: i32, lap_time_s: f64, valid: bool) -> OpponentLap {
        OpponentLap {
            car_idx,
            lap_number,
            lap_time_s,
            valid,
            track_wetness: Some(2),
            time_s: vec![0.0, 10.5, 20.25, lap_time_s as f32],
            gear: vec![0, 2, -1, 4],
        }
    }

    fn field(drivers: Vec<FieldDriver>, laps: Vec<OpponentLap>) -> Field {
        let results = drivers
            .iter()
            .map(|d| FieldResult {
                car_idx: d.car_idx,
                position: d.car_idx + 1,
                class_position: d.car_idx + 1,
                fastest_time_s: if d.car_idx == 0 { None } else { Some(90.5 + d.car_idx as f64) },
                fastest_lap: 3,
                laps_complete: 10,
            })
            .collect();
        Field {
            roster: Roster { sub_session_id: 555, session_num: 0, player_car_idx: 0, drivers, results },
            laps,
        }
    }

    fn json(f: &Field) -> serde_json::Value {
        serde_json::to_value(f).unwrap()
    }

    #[test]
    fn field_round_trips() {
        let mut store = Store::open_in_memory().unwrap();
        let id = store.upsert_session(&key(555, "Spa")).unwrap();
        let f = field(
            vec![driver(0, 1, true), driver(3, 1, false), driver(5, 2, false)],
            vec![lap(3, 1, 91.0, true), lap(3, 2, 92.5, false), lap(5, 1, 95.0, true)],
        );
        store.save_field(id, &f).unwrap();
        assert_eq!(json(&store.load_field(id).unwrap()), json(&f));
    }

    #[test]
    fn player_defaults_to_minus_one() {
        let mut store = Store::open_in_memory().unwrap();
        let id = store.upsert_session(&key(555, "Spa")).unwrap();
        store.save_field(id, &field(vec![driver(3, 1, false)], vec![])).unwrap();
        assert_eq!(store.load_field(id).unwrap().roster.player_car_idx, -1);
    }

    #[test]
    fn online_sessions_are_reused_offline_are_not() {
        let mut store = Store::open_in_memory().unwrap();
        let a = store.upsert_session(&key(555, "Spa")).unwrap();
        assert_eq!(a, store.upsert_session(&key(555, "Spa")).unwrap());
        assert_ne!(a, store.upsert_session(&key(556, "Spa")).unwrap());
        let o1 = store.upsert_session(&key(0, "Spa")).unwrap();
        let o2 = store.upsert_session(&key(0, "Spa")).unwrap();
        assert_ne!(o1, o2);
    }

    #[test]
    fn saving_twice_replaces_roster_and_keeps_one_lap_each() {
        let mut store = Store::open_in_memory().unwrap();
        let id = store.upsert_session(&key(555, "Spa")).unwrap();
        let first = field(vec![driver(0, 1, true), driver(3, 1, false)], vec![lap(3, 1, 91.0, true)]);
        store.save_field(id, &first).unwrap();

        let second = field(
            vec![driver(0, 1, true), driver(4, 1, false)],
            vec![lap(3, 1, 90.0, true), lap(3, 2, 92.0, true)],
        );
        store.save_field(id, &second).unwrap();

        let loaded = store.load_field(id).unwrap();
        assert_eq!(loaded.roster.drivers.iter().map(|d| d.car_idx).collect::<Vec<_>>(), [0, 4]);
        assert_eq!(loaded.roster.results.len(), 2);
        assert_eq!(loaded.laps.len(), 2);
        assert_eq!(loaded.laps[0].lap_time_s, 90.0);
    }

    #[test]
    fn library_filters_and_orders() {
        let mut store = Store::open_in_memory().unwrap();
        let s1 = store.upsert_session(&key(1, "Spa")).unwrap();
        let s2 = store.upsert_session(&key(2, "Spa")).unwrap();
        let s3 = store.upsert_session(&key(3, "Monza")).unwrap();
        // Class 1 and class 2 cars; one invalid lap; one other track.
        store
            .save_field(
                s1,
                &field(
                    vec![driver(0, 1, true), driver(1, 1, false), driver(2, 2, false)],
                    vec![lap(1, 1, 93.0, true), lap(1, 2, 80.0, false), lap(2, 1, 70.0, true)],
                ),
            )
            .unwrap();
        store
            .save_field(s2, &field(vec![driver(0, 1, true), driver(1, 1, false)], vec![lap(1, 1, 91.0, true)]))
            .unwrap();
        store
            .save_field(s3, &field(vec![driver(0, 1, true), driver(1, 1, false)], vec![lap(1, 1, 60.0, true)]))
            .unwrap();

        let all = store.library_laps("Spa", 1, None, 10).unwrap();
        let times: Vec<f64> = all.iter().map(|(l, _)| l.lap_time_s).collect();
        assert_eq!(times, [91.0, 93.0]);
        assert!(all.iter().all(|(_, d)| d.class_id == 1 && d.car_idx == 1));
        assert_eq!(all[0].1.name, "Driver 1");

        let excl = store.library_laps("Spa", 1, Some(s2), 10).unwrap();
        assert_eq!(excl.len(), 1);
        assert_eq!(excl[0].0.lap_time_s, 93.0);

        assert_eq!(store.library_laps("Spa", 1, None, 1).unwrap().len(), 1);
        assert_eq!(store.library_laps("Spa", 2, None, 10).unwrap().len(), 1);
    }

    #[test]
    fn corrupt_time_blob_errors() {
        let mut store = Store::open_in_memory().unwrap();
        let id = store.upsert_session(&key(555, "Spa")).unwrap();
        store
            .save_field(id, &field(vec![driver(0, 1, true), driver(1, 1, false)], vec![lap(1, 1, 91.0, true)]))
            .unwrap();
        store.conn.execute("UPDATE opponent_laps SET time_s = x'010203'", []).unwrap();
        let err = store.load_field(id).unwrap_err();
        assert!(format!("{err:#}").contains("corrupt time blob"), "{err:#}");
        assert!(store.library_laps("Spa", 1, None, 10).is_err());
    }

    #[test]
    fn file_db_migrates_once_and_reopens() {
        let dir = std::env::temp_dir().join(format!("pitcrew-store-test-{}", std::process::id()));
        let path = dir.join("nested").join("test.db");
        let _ = std::fs::remove_dir_all(&dir);

        let id = {
            let mut store = Store::open(&path).unwrap();
            let v: i64 = store.conn.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
            assert_eq!(v, 1);
            store.upsert_session(&key(555, "Spa")).unwrap()
        };
        let mut store = Store::open(&path).unwrap();
        let v: i64 = store.conn.pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
        assert_eq!(v, 1);
        assert_eq!(store.upsert_session(&key(555, "Spa")).unwrap(), id);
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

