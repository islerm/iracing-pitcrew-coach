use serde::{Deserialize, Serialize};

use crate::weather::SessionWeather;

#[derive(Debug, Clone, Deserialize)]
pub struct LapRecord {
    #[serde(rename = "lap")]
    pub lap_number: Option<i32>,
    #[serde(rename = "lap_time_s")]
    pub lap_time_s: Option<f64>,
    #[serde(rename = "lap_time")]
    pub lap_time_alt: Option<f64>,
    #[serde(rename = "sector_1_s")]
    pub sector_1_s: Option<f64>,
    #[serde(rename = "sector_1")]
    pub sector_1_alt: Option<f64>,
    #[serde(rename = "sector_2_s")]
    pub sector_2_s: Option<f64>,
    #[serde(rename = "sector_2")]
    pub sector_2_alt: Option<f64>,
    #[serde(rename = "sector_3_s")]
    pub sector_3_s: Option<f64>,
    #[serde(rename = "sector_3")]
    pub sector_3_alt: Option<f64>,
    #[serde(rename = "avg_speed_kph")]
    pub avg_speed_kph: Option<f64>,
    #[serde(rename = "average_speed_kph")]
    pub average_speed_kph: Option<f64>,
    #[serde(rename = "tyre_temp_avg_c")]
    pub tyre_temp_avg_c: Option<f64>,
    #[serde(rename = "tyre_temp_avg")]
    pub tyre_temp_avg_alt: Option<f64>,
    #[serde(rename = "tyre_temp_delta_c")]
    pub tyre_temp_delta_c: Option<f64>,
    #[serde(rename = "temp_delta_c")]
    pub temp_delta_c: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LapMetrics {
    pub lap_number: i32,
    pub lap_time_s: f64,
    pub is_complete: bool,
    /// Sector times in order. Empty when the source has no sector data.
    pub sectors: Vec<f64>,
    pub avg_speed_kph: Option<f64>,
    pub tyre_temp_avg_c: Option<f64>,
    pub tyre_temp_delta_c: Option<f64>,
    /// Lap fractions where the car left the track. Empty when it stayed on, or when the
    /// source has no track-surface channel.
    pub off_track_pcts: Vec<f64>,
    /// Incident points picked up during the lap. `None` when the source has no incident count.
    pub incidents: Option<u32>,
    /// Average air and track temperature over the lap, and the wettest the track got
    /// (`TrackWetness`: 1 dry … 7 extremely wet). `None` when the source has no weather channels.
    pub air_temp_c: Option<f64>,
    pub track_temp_c: Option<f64>,
    pub track_wetness: Option<u8>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionSummary {
    pub fastest_lap: i32,
    pub fastest_lap_time_s: f64,
    pub average_lap_time_s: f64,
    pub slowest_sector_name: Option<String>,
    pub slowest_sector_time_s: Option<f64>,
    pub average_speed_kph: Option<f64>,
    pub tyre_temp_avg_c: Option<f64>,
    pub tyre_temp_delta_c: Option<f64>,
    pub suggestions: Vec<String>,
    /// Corner-by-corner findings from the lap traces (see `handling::corner_notes`).
    pub corner_notes: Vec<String>,
    /// Conditions during the run (see `weather::session_weather`).
    pub weather: Option<SessionWeather>,
}
