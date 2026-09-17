use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Ordering {
    Random,
    Sequential,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub enabled: bool,
    pub min_minutes: f64,
    pub max_minutes: f64,
    pub ordering: Ordering,
    pub next_print_at: Option<f64>,
    pub last_item_id: Option<String>,
    pub last_error: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            enabled: false,
            min_minutes: 15.0,
            max_minutes: 20.0,
            ordering: Ordering::Sequential,
            next_print_at: None,
            last_item_id: None,
            last_error: None,
        }
    }
}

/// Missing or unreadable state file -> safe defaults (disabled). Corrupt
/// JSON is treated the same way rather than crashing the whole service on
/// startup -- an unattended, offline installation must never fail to boot
/// because of a damaged state file.
pub fn load(path: &Path) -> Settings {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Writes to a temp file in the same directory, then renames -- atomic on
/// the same filesystem, so a crash mid-write can never leave a truncated
/// state file that `load` would silently treat as "reset to defaults".
pub fn save(path: &Path, settings: &Settings) -> std::io::Result<()> {
    let temp_path = path.with_extension("json.tmp");
    fs::write(&temp_path, serde_json::to_vec_pretty(settings).unwrap())?;
    fs::rename(&temp_path, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_returns_defaults_when_the_file_does_not_exist() {
        let settings = load(Path::new("/tmp/does-not-exist-infinite-scroll-state.json"));
        assert!(!settings.enabled);
        assert_eq!(settings.ordering, Ordering::Sequential);
    }

    #[test]
    fn save_then_load_round_trips() {
        let path = std::env::temp_dir().join(format!("printer-state-test-{}.json", std::process::id()));
        let mut settings = Settings::default();
        settings.enabled = true;
        settings.ordering = Ordering::Random;
        settings.last_item_id = Some("abc".into());
        save(&path, &settings).unwrap();
        let loaded = load(&path);
        assert!(loaded.enabled);
        assert_eq!(loaded.ordering, Ordering::Random);
        assert_eq!(loaded.last_item_id, Some("abc".into()));
        let _ = fs::remove_file(&path);
    }
}
