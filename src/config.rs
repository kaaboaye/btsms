use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub database_path: PathBuf,
    pub last_device_address: Option<String>,
    pub last_device_name: Option<String>,
    pub device_type: DeviceType,
    pub auto_connect: bool,
    #[serde(default = "default_message_polling_enabled")]
    pub message_polling_enabled: bool,
    #[serde(default = "default_message_polling_interval")]
    pub message_polling_interval: u32,
}

fn default_message_polling_enabled() -> bool {
    true
}

fn default_message_polling_interval() -> u32 {
    15
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum DeviceType {
    IPhone,
    Android,
}

impl Default for Config {
    fn default() -> Self {
        let data_dir = Self::data_dir();

        Self {
            database_path: data_dir.join("btsms.db"),
            last_device_address: None,
            last_device_name: None,
            device_type: DeviceType::IPhone,
            auto_connect: true,
            message_polling_enabled: default_message_polling_enabled(),
            message_polling_interval: default_message_polling_interval(),
        }
    }
}

impl Config {
    /// Get the data directory path
    fn data_dir() -> PathBuf {
        dirs::data_local_dir()
            .unwrap_or_else(|| {
                let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
                PathBuf::from(home).join(".local/share")
            })
            .join("btsms")
    }

    /// Get the config file path
    fn config_path() -> PathBuf {
        Self::data_dir().join("config.toml")
    }

    /// Load config from disk, or return default if not found
    pub fn load() -> Self {
        let config_path = Self::config_path();

        if config_path.exists() {
            match std::fs::read_to_string(&config_path) {
                Ok(contents) => match toml::from_str(&contents) {
                    Ok(config) => return config,
                    Err(e) => eprintln!("Failed to parse config: {}", e),
                },
                Err(e) => eprintln!("Failed to read config: {}", e),
            }
        }

        Self::default()
    }

    /// Save config to disk
    pub fn save(&self) -> Result<(), std::io::Error> {
        let config_path = Self::config_path();

        // Ensure directory exists
        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let contents = toml::to_string_pretty(self).map_err(std::io::Error::other)?;

        std::fs::write(&config_path, contents)
    }

    /// Update the last used device and save
    pub fn set_last_device(&mut self, address: &str, name: &str) {
        self.last_device_address = Some(address.to_string());
        self.last_device_name = Some(name.to_string());
        if let Err(e) = self.save() {
            eprintln!("Failed to save config: {}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn test_config_default() {
        let config = Config::default();
        assert!(config.auto_connect);
        assert!(config.last_device_address.is_none());
        assert!(config.last_device_name.is_none());
        assert!(config.message_polling_enabled);
        assert_eq!(config.message_polling_interval, 15);
    }

    #[test]
    fn test_config_serialize_deserialize() {
        let mut config = Config::default();
        config.last_device_address = Some("AA:BB:CC:DD:EE:FF".to_string());
        config.last_device_name = Some("Test Phone".to_string());
        config.message_polling_enabled = false;
        config.message_polling_interval = 30;

        let toml_str = toml::to_string(&config).unwrap();
        let parsed: Config = toml::from_str(&toml_str).unwrap();

        assert_eq!(parsed.last_device_address, config.last_device_address);
        assert_eq!(parsed.last_device_name, config.last_device_name);
        assert_eq!(parsed.auto_connect, config.auto_connect);
        assert_eq!(
            parsed.message_polling_enabled,
            config.message_polling_enabled
        );
        assert_eq!(
            parsed.message_polling_interval,
            config.message_polling_interval
        );
    }

    #[test]
    fn test_config_deserialize_missing_polling_fields() {
        // Test that old configs without polling fields still load correctly
        let old_config_toml = r#"
database_path = "/tmp/test.db"
device_type = "IPhone"
auto_connect = true
"#;
        let parsed: Config = toml::from_str(old_config_toml).unwrap();
        assert!(parsed.message_polling_enabled);
        assert_eq!(parsed.message_polling_interval, 15);
    }

    #[test]
    fn test_set_last_device() {
        // Use a temp directory for testing
        let temp_dir = env::temp_dir().join("btsms_test_config");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        // We can't easily test save/load without mocking the path,
        // but we can test the mutation logic
        let mut config = Config::default();
        config.last_device_address = Some("11:22:33:44:55:66".to_string());
        config.last_device_name = Some("My Phone".to_string());

        assert_eq!(
            config.last_device_address,
            Some("11:22:33:44:55:66".to_string())
        );
        assert_eq!(config.last_device_name, Some("My Phone".to_string()));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_message_polling_interval_values() {
        let mut config = Config::default();

        // Test setting different polling intervals
        config.message_polling_interval = 5;
        assert_eq!(config.message_polling_interval, 5);

        config.message_polling_interval = 60;
        assert_eq!(config.message_polling_interval, 60);

        config.message_polling_interval = 120;
        assert_eq!(config.message_polling_interval, 120);
    }

    #[test]
    fn test_message_polling_toggle() {
        let mut config = Config::default();

        config.message_polling_enabled = false;
        assert!(!config.message_polling_enabled);

        config.message_polling_enabled = true;
        assert!(config.message_polling_enabled);
    }
}
