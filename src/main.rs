mod gui;

use gtk4::{glib, prelude::*};
use libadwaita as adw;

const APP_ID: &str = "com.github.btsms";

fn main() -> glib::ExitCode {
    // Create Tokio runtime
    let runtime = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");

    // Enter the runtime context
    let _guard = runtime.enter();

    // Initialize GTK
    let app = adw::Application::builder().application_id(APP_ID).build();

    // Connect to activate signal
    app.connect_activate(|app| {
        gui::build_ui(app);
    });

    // Run the application
    let exit_code = app.run();

    // Shutdown the runtime
    runtime.shutdown_timeout(std::time::Duration::from_secs(5));

    exit_code
}
