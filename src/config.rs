//! Persisted settings (audio devices, tone, latency calibration).

use serde::{Deserialize, Serialize};

use crate::audio::AudioSettings;
use crate::monitor::ToneSettings;
use crate::progress::data_dir;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub audio: AudioSettings,
    pub tone: ToneSettings,
    /// Manual latency calibration (tap along to clicks), ms; None = uncalibrated.
    pub latency_ms: Option<f64>,
    pub count_in_bars: Option<u32>,
}

impl Config {
    pub fn load() -> Self {
        std::fs::read_to_string(data_dir().join("config.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }
    pub fn save(&self) {
        let dir = data_dir();
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(dir.join("config.json"), s);
        }
    }
    pub fn count_in(&self) -> u32 {
        self.count_in_bars.unwrap_or(2)
    }
}
