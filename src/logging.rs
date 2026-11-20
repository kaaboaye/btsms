//! Logging setup shared by the GUI and the CLI.
//!
//! * Everything at `debug` and above (for this crate) goes to a daily-rotated log file in
//!   `$XDG_STATE_HOME/btsms/logs/` (usually `~/.local/state/btsms/logs/`), 7 files kept.
//! * stderr gets a shorter view whose level depends on `-v` flags.
//! * `BTSMS_LOG` (or `RUST_LOG`) overrides both filters, e.g. `BTSMS_LOG=btsms=trace,zbus=debug`.
//! * Panics are logged with a backtrace before the default hook runs.
//!
//! Message bodies are only ever logged at `trace` level.

use std::io::IsTerminal;
use std::path::PathBuf;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::fmt::time::ChronoLocal;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{EnvFilter, fmt};

const ENV_VAR: &str = "BTSMS_LOG";
const KEEP_LOG_FILES: usize = 7;

/// Keeps the background log writer alive. Drop it last (at the end of `main`) so buffered
/// lines are flushed.
pub struct LogGuard {
    _file_guard: Option<WorkerGuard>,
    pub log_dir: Option<PathBuf>,
}

/// Directory holding the log files.
pub fn log_dir() -> PathBuf {
    dirs::state_dir()
        .or_else(dirs::data_local_dir)
        .unwrap_or_else(std::env::temp_dir)
        .join("btsms")
        .join("logs")
}

/// Filter for the stderr layer: `0` = default, each `-v` raises it one level.
pub fn stderr_filter_directive(default_level: &str, verbosity: u8) -> String {
    let level = match verbosity {
        0 => default_level,
        1 => "info",
        2 => "debug",
        _ => "trace",
    };
    // zbus is extremely chatty at debug; only show it when tracing everything.
    if verbosity >= 3 {
        level.to_string()
    } else {
        format!("{level},zbus=warn,sqlx=warn")
    }
}

/// Filter for the file layer.
pub fn file_filter_directive() -> &'static str {
    "info,btsms=debug,btsms_cli=debug,zbus=warn,sqlx=warn"
}

fn env_override() -> Option<String> {
    std::env::var(ENV_VAR)
        .ok()
        .or_else(|| std::env::var("RUST_LOG").ok())
        .filter(|s| !s.trim().is_empty())
}

/// Initialise logging. `app` names the log file (`btsms` or `btsms-cli`).
/// `stderr_default` is the stderr level used without `-v` (`info` for the GUI, `warn` for the CLI).
pub fn init(app: &str, stderr_default: &str, verbosity: u8) -> LogGuard {
    let override_filter = env_override();
    let make_filter = |default: &str| match &override_filter {
        Some(f) => EnvFilter::try_new(f).unwrap_or_else(|_| EnvFilter::new(default)),
        None => EnvFilter::new(default),
    };

    let timer = ChronoLocal::new("%Y-%m-%d %H:%M:%S%.3f".to_string());

    let stderr_layer = fmt::layer()
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .with_timer(timer.clone())
        .with_target(verbosity >= 2)
        .with_filter(make_filter(&stderr_filter_directive(
            stderr_default,
            verbosity,
        )));

    let dir = log_dir();
    let (file_layer, file_guard, log_dir) = match std::fs::create_dir_all(&dir)
        .map_err(|e| e.to_string())
        .and_then(|_| {
            RollingFileAppender::builder()
                .rotation(Rotation::DAILY)
                .filename_prefix(app)
                .filename_suffix("log")
                .max_log_files(KEEP_LOG_FILES)
                .build(&dir)
                .map_err(|e| e.to_string())
        }) {
        Ok(appender) => {
            let (writer, guard) = tracing_appender::non_blocking(appender);
            let layer = fmt::layer()
                .with_writer(writer)
                .with_ansi(false)
                .with_timer(timer)
                .with_thread_names(true)
                .with_target(true)
                .with_filter(make_filter(file_filter_directive()));
            (Some(layer), Some(guard), Some(dir))
        }
        Err(e) => {
            eprintln!("btsms: cannot write log files to {}: {e}", dir.display());
            (None, None, None)
        }
    };

    // try_init fails if a subscriber is already set (e.g. in tests) - that is fine.
    let _ = tracing_subscriber::registry()
        .with(stderr_layer)
        .with(file_layer)
        .try_init();

    install_panic_hook();

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        pid = std::process::id(),
        log_dir = ?log_dir,
        "{app} starting"
    );

    LogGuard {
        _file_guard: file_guard,
        log_dir,
    }
}

fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        let thread = std::thread::current();
        tracing::error!(
            thread = thread.name().unwrap_or("<unnamed>"),
            "PANIC: {info}\n{backtrace}"
        );
        default_hook(info);
    }));
}

/// Short, log-safe description of a message body (never the content itself).
pub fn body_summary(body: &str) -> String {
    format!("<{} chars>", body.chars().count())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stderr_filter_respects_verbosity() {
        assert_eq!(
            stderr_filter_directive("warn", 0),
            "warn,zbus=warn,sqlx=warn"
        );
        assert_eq!(
            stderr_filter_directive("warn", 1),
            "info,zbus=warn,sqlx=warn"
        );
        assert_eq!(
            stderr_filter_directive("warn", 2),
            "debug,zbus=warn,sqlx=warn"
        );
        assert_eq!(stderr_filter_directive("warn", 3), "trace");
        assert_eq!(stderr_filter_directive("warn", 9), "trace");
    }

    #[test]
    fn filters_parse() {
        assert!(EnvFilter::try_new(file_filter_directive()).is_ok());
        for v in 0..4 {
            assert!(EnvFilter::try_new(stderr_filter_directive("info", v)).is_ok());
        }
    }

    #[test]
    fn log_dir_is_app_specific() {
        let dir = log_dir();
        assert!(dir.ends_with("btsms/logs"));
    }

    #[test]
    fn body_summary_hides_content() {
        assert_eq!(body_summary("tajne hasło"), "<11 chars>");
        assert_eq!(body_summary(""), "<0 chars>");
    }
}
