use std::path::PathBuf;

use serde::{Deserialize, Serialize};

type Error = Box<dyn std::error::Error>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub clips_directory: String,
    pub buffer_seconds: u32,
    pub fps: u32,
    pub codec: String,
    pub container: String,
    pub audio_codec: String,
    pub audio_source: String,
    pub hotkey_save_replay: String,
    pub hotkey_start_stop_recording: String,
    pub hotkey_start_stop_buffer: String,
    pub r2_endpoint: String,
    pub r2_bucket: String,
    pub r2_access_key: String,
    pub r2_secret_key: String,
    pub r2_custom_domain: String,
    pub expiry_days: u32,
    pub theme_path: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        let clips_dir = dirs::video_dir()
            .or_else(|| dirs::home_dir().map(|h| h.join("Videos")))
            .unwrap_or_default()
            .join("Klyppd");

        Self {
            clips_directory: clips_dir.to_string_lossy().into(),
            buffer_seconds: 120,
            fps: 60,
            codec: "h264".into(),
            container: "mp4".into(),
            audio_codec: "aac".into(),
            audio_source: "default_output".into(),
            hotkey_save_replay: "Alt+R".into(),
            hotkey_start_stop_recording: "Alt+Shift+R".into(),
            hotkey_start_stop_buffer: "Alt+F8".into(),
            r2_endpoint: String::new(),
            r2_bucket: String::new(),
            r2_access_key: String::new(),
            r2_secret_key: String::new(),
            r2_custom_domain: String::new(),
            expiry_days: 14,
            theme_path: String::new(),
        }
    }
}

fn path() -> PathBuf {
    dirs::config_dir().unwrap_or_default().join("klyppd").join("settings.json")
}

pub fn load() -> Result<AppSettings, Error> {
    let p = path();
    if !p.exists() {
        let s = AppSettings::default();
        save(&s)?;
        return Ok(s);
    }
    // FIXME: if user hand-edits the JSON and adds a typo, this silently falls back to defaults
    // should probably warn or show a toast in the UI
    Ok(serde_json::from_str(&std::fs::read_to_string(p)?)?)
}

pub fn save(settings: &AppSettings) -> Result<(), Error> {
    let p = path();
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(p, serde_json::to_string_pretty(settings)?)?;
    Ok(())
}
