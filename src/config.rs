use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub database_path: PathBuf,
    pub device_address: Option<String>,
    pub device_type: DeviceType,
    pub auto_connect: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum DeviceType {
    IPhone,
    Android,
}

impl Default for Config {
    fn default() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let data_dir = PathBuf::from(home).join(".local/share/btsms");

        Self {
            database_path: data_dir.join("btsms.db"),
            device_address: None,
            device_type: DeviceType::IPhone,
            auto_connect: false,
        }
    }
}
