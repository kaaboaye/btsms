//! Running Tokio work from GTK callbacks.
//!
//! GTK callbacks run on the main thread's GLib loop. Bluetooth and database work runs on the
//! Tokio runtime; these helpers spawn it there and hand the result back to a GLib future.

use btsms::error::{BtsmsError, Result};
use std::future::Future;

/// Run `fut` on the Tokio runtime and await its result from the GLib main loop.
pub async fn run<F, T>(fut: F) -> Result<T>
where
    F: Future<Output = Result<T>> + Send + 'static,
    T: Send + 'static,
{
    match tokio::spawn(fut).await {
        Ok(result) => result,
        Err(e) => {
            tracing::error!(error = %e, "background task failed");
            Err(BtsmsError::Bluetooth(format!("internal error: {e}")))
        }
    }
}
