use std::fmt;

#[derive(Debug)]
pub enum BtsmsError {
    Bluetooth(String),
    Database(sqlx::Error),
    DBus(zbus::Error),
    Io(std::io::Error),
    Parse(String),
    NotConnected,
    InvalidFormat(String),
    ContactNotFound,
}

impl fmt::Display for BtsmsError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Bluetooth(msg) => write!(f, "Bluetooth error: {}", msg),
            Self::Database(e) => write!(f, "Database error: {}", e),
            Self::DBus(e) => write!(f, "D-Bus error: {}", e),
            Self::Io(e) => write!(f, "IO error: {}", e),
            Self::Parse(msg) => write!(f, "Parse error: {}", msg),
            Self::NotConnected => write!(f, "Not connected to device"),
            Self::InvalidFormat(msg) => write!(f, "Invalid format: {}", msg),
            Self::ContactNotFound => write!(f, "Contact not found"),
        }
    }
}

impl std::error::Error for BtsmsError {}

impl From<sqlx::Error> for BtsmsError {
    fn from(err: sqlx::Error) -> Self {
        Self::Database(err)
    }
}

impl From<zbus::Error> for BtsmsError {
    fn from(err: zbus::Error) -> Self {
        Self::DBus(err)
    }
}

impl From<std::io::Error> for BtsmsError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

pub type Result<T> = std::result::Result<T, BtsmsError>;
