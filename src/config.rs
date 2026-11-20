use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Smallest allowed polling interval in seconds (MAP listing is cheap, but not free).
pub const MIN_POLL_INTERVAL: u32 = 5;
/// Largest allowed polling interval in seconds.
pub const MAX_POLL_INTERVAL: u32 = 3600;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub database_path: PathBuf,
    pub last_device_address: Option<String>,
    pub last_device_name: Option<String>,
    pub auto_connect: bool,
    pub message_polling_enabled: bool,
    /// Seconds between MAP inbox checks. ANCS (iPhone) triggers an immediate check on
    /// every new message, so this mostly matters for Android.
    pub message_polling_interval: u32,
    /// ISO 3166 region used for numbers written without a country code, e.g. "PL".
    /// When unset, it is derived from the locale (`LANG=pl_PL.UTF-8` -> "PL").
    pub default_region: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            database_path: Self::data_dir().join("btsms.db"),
            last_device_address: None,
            last_device_name: None,
            auto_connect: true,
            message_polling_enabled: true,
            message_polling_interval: 60,
            default_region: None,
        }
    }
}

impl Config {
    /// Directory for the database and config file (`~/.local/share/btsms`).
    pub fn data_dir() -> PathBuf {
        dirs::data_local_dir()
            .unwrap_or_else(|| {
                let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
                PathBuf::from(home).join(".local/share")
            })
            .join("btsms")
    }

    /// Path of the config file.
    pub fn config_path() -> PathBuf {
        Self::data_dir().join("config.toml")
    }

    /// Older GUI builds ignored `database_path` and always used this file.
    pub fn legacy_database_path() -> PathBuf {
        Self::data_dir().join("messages.db")
    }

    /// Load the config from the default location, falling back to defaults.
    pub fn load() -> Self {
        Self::load_from(&Self::config_path())
    }

    /// Load the config from `path`, falling back to defaults (with a logged warning) when the
    /// file is missing or invalid.
    pub fn load_from(path: &Path) -> Self {
        let contents = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::info!(path = %path.display(), "no config file yet, using defaults");
                return Self::default();
            }
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "cannot read config, using defaults");
                return Self::default();
            }
        };
        match toml::from_str::<Config>(&contents) {
            Ok(mut config) => {
                config.sanitize();
                tracing::debug!(path = %path.display(), ?config, "config loaded");
                config
            }
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "invalid config, using defaults");
                Self::default()
            }
        }
    }

    /// Clamp values that would make the app misbehave.
    fn sanitize(&mut self) {
        self.message_polling_interval = self
            .message_polling_interval
            .clamp(MIN_POLL_INTERVAL, MAX_POLL_INTERVAL);
        if let Some(region) = &self.default_region
            && region.trim().is_empty()
        {
            self.default_region = None;
        }
    }

    /// Save to the default location.
    pub fn save(&self) -> std::io::Result<()> {
        self.save_to(&Self::config_path())
    }

    /// Save atomically (write a temp file, then rename) so a crash never leaves a
    /// half-written config behind.
    pub fn save_to(&self, path: &Path) -> std::io::Result<()> {
        let dir = path.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(dir)?;
        let contents = toml::to_string_pretty(self).map_err(std::io::Error::other)?;
        let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
        std::io::Write::write_all(&mut tmp, contents.as_bytes())?;
        tmp.persist(path).map_err(|e| e.error)?;
        tracing::debug!(path = %path.display(), "config saved");
        Ok(())
    }

    /// Remember the last used device and persist the config.
    pub fn set_last_device(&mut self, address: &str, name: &str) {
        self.last_device_address = Some(address.to_string());
        self.last_device_name = Some(name.to_string());
        if let Err(e) = self.save() {
            tracing::warn!(error = %e, "failed to save config");
        }
    }

    /// The region for parsing local phone numbers: config first, then the locale.
    pub fn region(&self) -> Option<String> {
        self.default_region
            .as_deref()
            .map(|r| r.trim().to_ascii_uppercase())
            .filter(|r| !r.is_empty())
            .or_else(region_from_locale)
    }
}

/// Region from `LC_ALL`, `LC_MESSAGES` or `LANG` (`pl_PL.UTF-8` -> `PL`).
pub fn region_from_locale() -> Option<String> {
    ["LC_ALL", "LC_TELEPHONE", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|var| std::env::var(var).ok())
        .find_map(|value| region_from_locale_str(&value))
}

/// Extract the region part of a POSIX locale string.
pub fn region_from_locale_str(locale: &str) -> Option<String> {
    let base = locale.split(['.', '@']).next()?;
    let (_, region) = base.split_once('_')?;
    (region.len() == 2 && region.chars().all(|c| c.is_ascii_alphabetic()))
        .then(|| region.to_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        let config = Config::default();
        assert!(config.auto_connect);
        assert!(config.message_polling_enabled);
        assert_eq!(config.message_polling_interval, 60);
        assert!(config.database_path.ends_with("btsms/btsms.db"));
        assert!(config.default_region.is_none());
    }

    #[test]
    fn roundtrip_through_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("config.toml");
        let config = Config {
            last_device_address: Some("AA:BB:CC:DD:EE:FF".into()),
            last_device_name: Some("Test Phone".into()),
            message_polling_interval: 30,
            default_region: Some("PL".into()),
            ..Config::default()
        };
        config.save_to(&path).unwrap();

        assert_eq!(Config::load_from(&path), config);
    }

    #[test]
    fn old_config_with_removed_fields_still_loads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "database_path = \"/tmp/test.db\"\ndevice_type = \"IPhone\"\nauto_connect = false\n",
        )
        .unwrap();
        let config = Config::load_from(&path);
        assert_eq!(config.database_path, PathBuf::from("/tmp/test.db"));
        assert!(!config.auto_connect);
        assert!(config.message_polling_enabled);
    }

    #[test]
    fn missing_or_broken_file_gives_defaults() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            Config::load_from(&dir.path().join("nope.toml")),
            Config::default()
        );
        let path = dir.path().join("broken.toml");
        std::fs::write(&path, "this is = = not toml").unwrap();
        assert_eq!(Config::load_from(&path), Config::default());
    }

    #[test]
    fn polling_interval_is_clamped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "message_polling_interval = 0\n").unwrap();
        assert_eq!(
            Config::load_from(&path).message_polling_interval,
            MIN_POLL_INTERVAL
        );
        std::fs::write(&path, "message_polling_interval = 999999\n").unwrap();
        assert_eq!(
            Config::load_from(&path).message_polling_interval,
            MAX_POLL_INTERVAL
        );
    }

    #[test]
    fn region_from_locale_strings() {
        assert_eq!(region_from_locale_str("pl_PL.UTF-8").as_deref(), Some("PL"));
        assert_eq!(region_from_locale_str("en_US").as_deref(), Some("US"));
        assert_eq!(region_from_locale_str("de_DE@euro").as_deref(), Some("DE"));
        assert_eq!(region_from_locale_str("C.UTF-8"), None);
        assert_eq!(region_from_locale_str("POSIX"), None);
        assert_eq!(region_from_locale_str(""), None);
    }

    #[test]
    fn configured_region_wins() {
        let config = Config {
            default_region: Some(" pl ".into()),
            ..Config::default()
        };
        assert_eq!(config.region().as_deref(), Some("PL"));
    }
}
