mod gui;

use gtk4::glib;
use libadwaita as adw;
use libadwaita::prelude::*;

fn main() -> glib::ExitCode {
    let verbosity = std::env::args()
        .filter(|a| a == "-v" || a == "--verbose")
        .count()
        .min(3) as u8;
    let _log = btsms::logging::init("btsms", "info", verbosity);

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("btsms-worker")
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            tracing::error!(error = %e, "cannot start the Tokio runtime");
            return glib::ExitCode::FAILURE;
        }
    };
    // GTK callbacks run on this thread; entering the runtime lets them spawn Tokio tasks.
    let _guard = runtime.enter();

    let application = adw::Application::builder()
        .application_id(gui::APP_ID)
        .build();
    gui::setup(&application);
    // Our own flags (-v) are not GTK options.
    let exit_code = application.run_with_args(&[std::env::args().next().unwrap_or_default()]);

    drop(_guard);
    runtime.shutdown_timeout(std::time::Duration::from_secs(3));
    tracing::info!("exiting");
    exit_code
}
