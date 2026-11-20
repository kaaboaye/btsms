use thiserror::Error;

#[derive(Debug, Error)]
pub enum BtsmsError {
    #[error("Bluetooth error: {0}")]
    Bluetooth(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("D-Bus error: {0}")]
    DBus(#[from] zbus::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Parse error: {0}")]
    Parse(String),

    #[error("Not connected to a device")]
    NotConnected,

    #[error("Invalid format: {0}")]
    InvalidFormat(String),

    #[error("Timed out: {0}")]
    Timeout(String),
}

impl From<zbus::fdo::Error> for BtsmsError {
    fn from(err: zbus::fdo::Error) -> Self {
        Self::DBus(err.into())
    }
}

pub type Result<T> = std::result::Result<T, BtsmsError>;

/// Returns the D-Bus error name of a method error reply, e.g.
/// `org.bluez.obex.Error.Forbidden` or `org.freedesktop.DBus.Error.UnknownObject`.
pub fn dbus_error_name(err: &zbus::Error) -> Option<String> {
    match err {
        zbus::Error::MethodError(name, _, _) => Some(name.to_string()),
        zbus::Error::FDO(fdo) => Some(fdo_error_name(fdo).to_string()),
        _ => None,
    }
}

fn fdo_error_name(err: &zbus::fdo::Error) -> &'static str {
    use zbus::fdo::Error as E;
    match err {
        E::UnknownObject(_) => "org.freedesktop.DBus.Error.UnknownObject",
        E::UnknownMethod(_) => "org.freedesktop.DBus.Error.UnknownMethod",
        E::UnknownInterface(_) => "org.freedesktop.DBus.Error.UnknownInterface",
        E::UnknownProperty(_) => "org.freedesktop.DBus.Error.UnknownProperty",
        E::ServiceUnknown(_) => "org.freedesktop.DBus.Error.ServiceUnknown",
        E::NoReply(_) => "org.freedesktop.DBus.Error.NoReply",
        E::AccessDenied(_) => "org.freedesktop.DBus.Error.AccessDenied",
        E::InvalidArgs(_) => "org.freedesktop.DBus.Error.InvalidArgs",
        E::TimedOut(_) | E::Timeout(_) => "org.freedesktop.DBus.Error.Timeout",
        _ => "org.freedesktop.DBus.Error.Failed",
    }
}

/// True when the error says the remote object (session, transfer, message) no longer exists.
pub fn is_unknown_object(err: &zbus::Error) -> bool {
    dbus_error_name(err).is_some_and(|n| {
        n.ends_with(".UnknownObject")
            || n.ends_with(".UnknownMethod")
            || n.ends_with(".UnknownInterface")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fdo_unknown_object_is_detected() {
        let err: zbus::Error = zbus::fdo::Error::UnknownObject("gone".into()).into();
        assert_eq!(
            dbus_error_name(&err).as_deref(),
            Some("org.freedesktop.DBus.Error.UnknownObject")
        );
        assert!(is_unknown_object(&err));
    }

    #[test]
    fn non_method_errors_have_no_name() {
        let err = zbus::Error::Failure("boom".into());
        assert_eq!(dbus_error_name(&err), None);
        assert!(!is_unknown_object(&err));
    }

    #[test]
    fn display_includes_context() {
        let err = BtsmsError::Timeout("waiting for transfer".into());
        assert_eq!(err.to_string(), "Timed out: waiting for transfer");
    }
}
