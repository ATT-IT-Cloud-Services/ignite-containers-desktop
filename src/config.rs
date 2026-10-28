//! Small plain-text settings file for the desktop client.
//!
//! The API key is only written when the user explicitly enables "remember".
//! On Unix the file is created with mode 0600.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SavedConfig {
    #[serde(default)]
    pub api_url: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub remember: bool,
    #[serde(default)]
    pub last_tab: Option<String>,
    #[serde(default)]
    pub last_server: Option<String>,
}

#[must_use]
pub fn config_path() -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("IGNITE_CONTAINERS_DESKTOP_CONFIG") {
        let trimmed = explicit.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        let trimmed = xdg.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed).join("ignite-containers/desktop.json"));
        }
    }
    let home = std::env::var("HOME").ok()?;
    if home.trim().is_empty() {
        return None;
    }
    Some(PathBuf::from(home).join(".config/ignite-containers/desktop.json"))
}

#[must_use]
pub fn load() -> Option<SavedConfig> {
    let path = config_path()?;
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn save(config: &SavedConfig) -> Result<PathBuf, String> {
    let path = config_path().ok_or_else(|| "no home directory for settings".to_string())?;
    let parent = path
        .parent()
        .ok_or_else(|| "invalid settings path".to_string())?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    let text = serde_json::to_string_pretty(config)
        .map_err(|error| format!("could not encode settings: {error}"))?;
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, text)
        .map_err(|error| format!("could not write {}: {error}", temporary.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&temporary, &path)
        .map_err(|error| format!("could not replace {}: {error}", path.display()))?;
    Ok(path)
}
