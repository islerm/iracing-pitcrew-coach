//! Small user settings kept in `data/settings.json`.

use std::path::Path;

use serde::{Deserialize, Serialize};

const PATH: &str = "data/settings.json";
pub const MIN_RADIO_VOLUME: f32 = 0.5;
pub const MAX_RADIO_VOLUME: f32 = 3.0;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Loudness multiplier for the live radio calls.
    pub radio_volume: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self { radio_volume: 1.0 }
    }
}

pub fn clamp_volume(volume: f32) -> f32 {
    if volume.is_finite() { volume.clamp(MIN_RADIO_VOLUME, MAX_RADIO_VOLUME) } else { 1.0 }
}

impl Settings {
    /// Missing or invalid file gives the defaults.
    pub fn load() -> Self {
        let mut settings: Self = std::fs::read_to_string(PATH)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        settings.radio_volume = clamp_volume(settings.radio_volume);
        settings
    }

    pub fn save(&self) -> std::io::Result<()> {
        if let Some(dir) = Path::new(PATH).parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(PATH, text)
    }
}
