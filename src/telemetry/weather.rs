//! Session weather: iRacing's snapshot from the session info, plus the weather channels
//! recorded with every sample. With dynamic weather the track warms, cools, dries or gets
//! wet during a run, so per-sample values are preferred and the snapshot is the fallback.

use serde::Serialize;

use crate::stats::{finite_max, mean};
use crate::telemetry::trace::Frame;
use crate::telemetry::trace::LapMetrics;

/// Telemetry channels read into `WeatherSample`, in `from_channels` order.
pub const CHANNELS: [&str; 10] = [
    "AirTemp", "TrackTempCrew", "TrackTemp", "TrackWetness", "Precipitation", "Skies", "WindVel", "WindDir",
    "RelativeHumidity", "WeatherDeclaredWet",
];

/// Weather channels for one sample. NaN when the source doesn't record them.
#[derive(Debug, Clone, Copy)]
pub struct WeatherSample {
    pub air_temp_c: f32,
    pub track_temp_c: f32,
    /// `TrackWetness` (1 dry … 7 extremely wet, 0 unknown).
    pub wetness: f32,
    /// `Precipitation`, 0–1.
    pub precipitation: f32,
    /// `Skies`: 0 clear, 1 partly cloudy, 2 mostly cloudy, 3 overcast.
    pub skies: f32,
    pub wind_ms: f32,
    pub wind_dir_rad: f32,
    /// `RelativeHumidity`, 0–1.
    pub humidity: f32,
    /// `WeatherDeclaredWet` as 1.0/0.0.
    pub declared_wet: f32,
}

impl Default for WeatherSample {
    fn default() -> Self {
        Self {
            air_temp_c: f32::NAN,
            track_temp_c: f32::NAN,
            wetness: f32::NAN,
            precipitation: f32::NAN,
            skies: f32::NAN,
            wind_ms: f32::NAN,
            wind_dir_rad: f32::NAN,
            humidity: f32::NAN,
            declared_wet: f32::NAN,
        }
    }
}

impl WeatherSample {
    /// Builds a sample from `get(i)`, the value of `CHANNELS[i]` (None when missing).
    pub fn from_channels(get: impl Fn(usize) -> Option<f64>) -> Self {
        let v = |i: usize| get(i).map(|x| x as f32).unwrap_or(f32::NAN);
        // `TrackTemp` is iRacing's older name; newer builds record `TrackTempCrew`.
        let track = if v(1).is_finite() { v(1) } else { v(2) };
        Self {
            air_temp_c: v(0),
            track_temp_c: track,
            wetness: v(3),
            precipitation: v(4),
            skies: v(5),
            wind_ms: v(6),
            wind_dir_rad: v(7),
            humidity: v(8),
            declared_wet: v(9),
        }
    }
}

/// Conditions iRacing reports in the session info. Usually taken when the session loaded,
/// so they can be out of date by the end of a long run with dynamic weather.
#[derive(Debug, Clone, Default)]
pub struct WeatherSnapshot {
    /// "Realistic" (dynamic) or "Constant".
    pub weather_type: String,
    /// "Partly Cloudy", "Dynamic", …
    pub skies: String,
    /// In-sim time of day, e.g. "1:40 pm".
    pub time_of_day: String,
    pub air_temp_c: Option<f64>,
    pub track_temp_c: Option<f64>,
    pub wind_ms: Option<f64>,
    pub wind_dir_rad: Option<f64>,
    pub humidity_pct: Option<f64>,
    pub precipitation_pct: Option<f64>,
}

/// Leading number of a session-info value like "19.29 C" or "4.24 m/s".
fn leading_number(value: &str) -> Option<f64> {
    value.split_whitespace().next()?.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// Speed in m/s from a value like "4.24 m/s" or "15.3 km/h".
pub fn wind_ms(value: &str) -> Option<f64> {
    leading_number(value).map(|v| if value.contains("km/h") { v / 3.6 } else { v })
}

impl WeatherSnapshot {
    /// Reads the snapshot from session-info keys via `get` (empty string when missing).
    pub fn from_session_info(get: impl Fn(&str) -> String) -> Self {
        let num = |key: &str| leading_number(&get(key));
        Self {
            weather_type: get("TrackWeatherType"),
            skies: get("TrackSkies"),
            time_of_day: get("TimeOfDay"),
            air_temp_c: num("TrackAirTemp"),
            track_temp_c: num("TrackSurfaceTempCrew").or_else(|| num("TrackSurfaceTemp")),
            wind_ms: wind_ms(&get("TrackWindVel")),
            wind_dir_rad: num("TrackWindDir"),
            humidity_pct: num("TrackRelativeHumidity"),
            precipitation_pct: num("TrackPrecipitation"),
        }
    }
}

/// A value over the run: where it started and ended, and how far it moved.
#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub struct Range {
    pub start: f64,
    pub end: f64,
    pub min: f64,
    pub max: f64,
}

impl Range {
    fn of(values: impl Iterator<Item = f64>) -> Option<Self> {
        let mut range: Option<Range> = None;
        for v in values.filter(|v| v.is_finite()) {
            range = Some(match range {
                None => Range { start: v, end: v, min: v, max: v },
                Some(r) => Range { end: v, min: r.min.min(v), max: r.max.max(v), ..r },
            });
        }
        range
    }

    fn fixed(v: f64) -> Self {
        Range { start: v, end: v, min: v, max: v }
    }

    pub fn spread(&self) -> f64 {
        self.max - self.min
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionWeather {
    pub weather_type: Option<String>,
    /// Sky at the start of the run, and at the end when it changed.
    pub skies: Option<String>,
    pub skies_end: Option<String>,
    pub time_of_day: Option<String>,
    pub air_temp_c: Option<Range>,
    pub track_temp_c: Option<Range>,
    pub wind_kph: Option<f64>,
    /// Compass point, e.g. "NW".
    pub wind_dir: Option<String>,
    pub humidity_pct: Option<f64>,
    /// Track wetness at the start, and at the end when it changed.
    pub wetness: Option<String>,
    pub wetness_end: Option<String>,
    /// Heaviest rain during the run, %.
    pub rain_pct: Option<f64>,
    pub declared_wet: bool,
    /// Track temperature during the fastest lap.
    pub fastest_lap_track_temp_c: Option<f64>,
}

pub fn skies_label(code: f64) -> Option<&'static str> {
    if !code.is_finite() {
        return None;
    }
    match code.round() as i32 {
        0 => Some("Clear"),
        1 => Some("Partly cloudy"),
        2 => Some("Mostly cloudy"),
        3 => Some("Overcast"),
        _ => None,
    }
}

pub fn wetness_label(code: f64) -> Option<&'static str> {
    if !code.is_finite() {
        return None;
    }
    match code.round() as i32 {
        1 => Some("Dry"),
        2 => Some("Mostly dry"),
        3 => Some("Very lightly wet"),
        4 => Some("Lightly wet"),
        5 => Some("Moderately wet"),
        6 => Some("Very wet"),
        7 => Some("Extremely wet"),
        _ => None,
    }
}

fn compass(rad: f64) -> String {
    const POINTS: [&str; 8] = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];
    let deg = rad.to_degrees().rem_euclid(360.0);
    POINTS[((deg / 45.0).round() as usize) % 8].to_string()
}

/// First and last labels of a coded channel; the end is only kept when it differs.
fn start_end(frames: &[Frame], code: impl Fn(&Frame) -> f32, label: fn(f64) -> Option<&'static str>) -> (Option<String>, Option<String>) {
    let mut labels = frames.iter().filter_map(|f| label(code(f) as f64));
    let first = labels.next();
    let last = labels.last().filter(|l| Some(*l) != first);
    (first.map(String::from), last.map(String::from))
}

pub fn session_weather(frames: &[Frame], snap: &WeatherSnapshot) -> Option<SessionWeather> {
    let w = |f: &Frame| f.weather;
    let air = Range::of(frames.iter().map(|f| w(f).air_temp_c as f64)).or(snap.air_temp_c.map(Range::fixed));
    let track = Range::of(frames.iter().map(|f| w(f).track_temp_c as f64)).or(snap.track_temp_c.map(Range::fixed));

    let wind_ms = mean(frames.iter().map(|f| w(f).wind_ms as f64)).or(snap.wind_ms);
    // Average direction as a vector, so N and NNW don't average to S.
    let (sx, sy, n) = frames
        .iter()
        .map(|f| w(f).wind_dir_rad as f64)
        .filter(|d| d.is_finite())
        .fold((0.0, 0.0, 0usize), |(x, y, n), d| (x + d.sin(), y + d.cos(), n + 1));
    let wind_dir_rad = if n > 0 { Some(sx.atan2(sy)) } else { snap.wind_dir_rad };

    let (mut skies, skies_end) = start_end(frames, |f| f.weather.skies, skies_label);
    if skies.is_none() && !snap.skies.is_empty() && snap.skies != "Dynamic" {
        skies = Some(snap.skies.clone());
    }
    let (wetness, wetness_end) = start_end(frames, |f| f.weather.wetness, wetness_label);
    let rain = finite_max(frames.iter().map(|f| f.weather.precipitation as f64 * 100.0)).or(snap.precipitation_pct);

    let weather = SessionWeather {
        weather_type: Some(snap.weather_type.clone()).filter(|s| !s.is_empty()),
        skies,
        skies_end,
        time_of_day: Some(snap.time_of_day.clone()).filter(|s| !s.is_empty()),
        air_temp_c: air,
        track_temp_c: track,
        wind_kph: wind_ms.map(|v| v * 3.6),
        wind_dir: wind_dir_rad.map(compass),
        humidity_pct: mean(frames.iter().map(|f| f.weather.humidity as f64 * 100.0)).or(snap.humidity_pct),
        wetness,
        wetness_end,
        rain_pct: rain,
        declared_wet: frames.iter().any(|f| f.weather.declared_wet > 0.5),
        fastest_lap_track_temp_c: None,
    };
    let empty = weather.air_temp_c.is_none() && weather.track_temp_c.is_none() && weather.skies.is_none() && weather.wetness.is_none() && weather.wind_kph.is_none();
    (!empty).then_some(weather)
}

/// Average air and track temperature over a lap, and the wettest the track got.
pub fn lap_weather(frames: &[Frame]) -> (Option<f64>, Option<f64>, Option<u8>) {
    let air = mean(frames.iter().map(|f| f.weather.air_temp_c as f64));
    let track = mean(frames.iter().map(|f| f.weather.track_temp_c as f64));
    let wet = finite_max(frames.iter().map(|f| f.weather.wetness as f64).filter(|v| *v >= 1.0)).map(|v| v.round() as u8);
    (air, track, wet)
}

/// How much the track temperature has to move before it's worth a suggestion.
const TRACK_TEMP_SWING_C: f64 = 4.0;

/// One-line description of the conditions, for the coach prompt.
pub fn describe(w: &SessionWeather) -> String {
    let fmt_range = |r: &Range| {
        if (r.end - r.start).abs() >= 0.5 {
            format!("{:.1}C at the start, {:.1}C at the end", r.start, r.end)
        } else {
            format!("{:.1}C", r.start)
        }
    };
    let mut parts = Vec::new();
    if let Some(s) = &w.skies {
        parts.push(match &w.skies_end {
            Some(end) => format!("{s} turning {}", end.to_lowercase()),
            None => s.clone(),
        });
    }
    if let Some(r) = &w.air_temp_c {
        parts.push(format!("air {}", fmt_range(r)));
    }
    if let Some(r) = &w.track_temp_c {
        parts.push(format!("track {}", fmt_range(r)));
    }
    if let Some(kph) = w.wind_kph {
        parts.push(format!("wind {kph:.0} km/h{}", w.wind_dir.as_ref().map(|d| format!(" {d}")).unwrap_or_default()));
    }
    if let Some(h) = w.humidity_pct {
        parts.push(format!("humidity {h:.0}%"));
    }
    if let Some(wet) = &w.wetness {
        parts.push(match &w.wetness_end {
            Some(end) => format!("track {} turning {}", wet.to_lowercase(), end.to_lowercase()),
            None => format!("track {}", wet.to_lowercase()),
        });
    }
    if let Some(rain) = w.rain_pct.filter(|r| *r > 0.5) {
        parts.push(format!("rain up to {rain:.0}%"));
    }
    parts.join(", ")
}

/// Fills in the fastest lap's track temperature and returns suggestions about the conditions.
pub fn annotate(w: &mut SessionWeather, laps: &[LapMetrics], fastest_lap: i32) -> Vec<String> {
    w.fastest_lap_track_temp_c = laps.iter().find(|l| l.lap_number == fastest_lap).and_then(|l| l.track_temp_c);
    let mut out = Vec::new();
    if let Some(r) = w.track_temp_c.filter(|r| r.spread() >= TRACK_TEMP_SWING_C) {
        let warmer = r.end > r.start;
        out.push(format!(
            "The track {} from {:.0}C to {:.0}C during the run. Compare laps driven at similar track temperatures: {} grip as it {}.",
            if warmer { "warmed up" } else { "cooled down" },
            r.start,
            r.end,
            if warmer { "expect less" } else { "expect more" },
            if warmer { "heats up" } else { "cools" },
        ));
    }
    let wet = w.wetness.as_deref().is_some_and(|s| s != "Dry") || w.wetness_end.as_deref().is_some_and(|s| s != "Dry");
    if wet || w.declared_wet {
        out.push("The track was wet for part of the run, so lap times across the session aren't directly comparable. Brake earlier and look for grip off the dry line.".to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(air: f32, track: f32, wet: f32, skies: f32) -> Frame {
        Frame { weather: WeatherSample { air_temp_c: air, track_temp_c: track, wetness: wet, skies, wind_ms: 5.0, wind_dir_rad: 0.1, ..WeatherSample::default() }, ..Frame::default() }
    }

    #[test]
    fn summarizes_changing_conditions() {
        let frames = vec![frame(19.0, 25.0, 1.0, 1.0), frame(20.0, 28.0, 1.0, 1.0), frame(21.0, 31.0, 3.0, 3.0)];
        let w = session_weather(&frames, &WeatherSnapshot::default()).unwrap();
        assert_eq!(w.track_temp_c, Some(Range { start: 25.0, end: 31.0, min: 25.0, max: 31.0 }));
        assert_eq!(w.skies.as_deref(), Some("Partly cloudy"));
        assert_eq!(w.skies_end.as_deref(), Some("Overcast"));
        assert_eq!(w.wetness.as_deref(), Some("Dry"));
        assert_eq!(w.wetness_end.as_deref(), Some("Very lightly wet"));
        assert_eq!(w.wind_dir.as_deref(), Some("N"));
        assert!((w.wind_kph.unwrap() - 18.0).abs() < 1e-6);
        let text = describe(&w);
        assert!(text.contains("track 25.0C at the start, 31.0C at the end"), "{text}");
    }

    #[test]
    fn falls_back_to_session_info() {
        let yaml = [
            ("TrackWeatherType", "Realistic"),
            ("TrackSkies", "Partly Cloudy"),
            ("TrackSurfaceTemp", "25.29 C"),
            ("TrackAirTemp", "19.29 C"),
            ("TrackWindVel", "4.24 m/s"),
            ("TrackWindDir", "3.08 rad"),
            ("TrackRelativeHumidity", "93 %"),
            ("TimeOfDay", "1:40 pm"),
        ];
        let snap = WeatherSnapshot::from_session_info(|k| yaml.iter().find(|(key, _)| *key == k).map(|(_, v)| v.to_string()).unwrap_or_default());
        let w = session_weather(&[Frame::default()], &snap).unwrap();
        assert_eq!(w.track_temp_c.unwrap().start, 25.29);
        assert_eq!(w.skies.as_deref(), Some("Partly Cloudy"));
        assert_eq!(w.wind_dir.as_deref(), Some("S"));
        assert_eq!(w.humidity_pct, Some(93.0));
        assert_eq!(w.time_of_day.as_deref(), Some("1:40 pm"));
        assert!(!w.declared_wet);
    }

    #[test]
    fn no_weather_without_data() {
        assert!(session_weather(&[Frame::default()], &WeatherSnapshot::default()).is_none());
        assert_eq!(lap_weather(&[Frame::default()]), (None, None, None));
    }

    #[test]
    fn suggests_comparing_at_similar_track_temps() {
        let frames = vec![frame(19.0, 25.0, 1.0, 1.0), frame(21.0, 31.0, 1.0, 1.0)];
        let mut w = session_weather(&frames, &WeatherSnapshot::default()).unwrap();
        let notes = annotate(&mut w, &[], 1);
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains("warmed up from 25C to 31C"), "{}", notes[0]);
    }
}
