mod app;
mod conversation_row;
mod dialogs;
mod message_bubble;
mod preferences;
mod runtime;
mod style;
mod window;

use app::App;
use btsms::config::Config;
use btsms::contacts::phone_normalizer;
use gtk4::prelude::*;
use gtk4::{gio, glib};
use libadwaita as adw;
use libadwaita::prelude::*;
use std::rc::Rc;

pub const APP_ID: &str = "com.github.btsms";

/// Wire up the application: CSS on startup, one window on activate.
pub fn setup(application: &adw::Application) {
    application.connect_startup(|_| style::load());

    let state: Rc<std::cell::RefCell<Option<Rc<App>>>> = Rc::default();
    let state_activate = state.clone();
    application.connect_activate(move |application| {
        if let Some(app) = state_activate.borrow().as_ref() {
            app.w.window.present();
            return;
        }
        let config = Config::load();
        phone_normalizer::set_default_region(config.region().as_deref());
        let app = App::new(application, config);
        install_actions(application, &app);
        app.w.window.present();
        glib::spawn_future_local(app.clone().start());
        *state_activate.borrow_mut() = Some(app);
    });

    application.connect_shutdown(move |_| {
        if let Some(app) = state.borrow_mut().take() {
            app.shutdown();
        }
    });
}

fn add_action(
    application: &adw::Application,
    app: &Rc<App>,
    name: &str,
    accels: &[&str],
    f: impl Fn(Rc<App>) + 'static,
) {
    let action = gio::SimpleAction::new(name, None);
    let weak = Rc::downgrade(app);
    action.connect_activate(move |_, _| {
        if let Some(app) = weak.upgrade() {
            f(app);
        }
    });
    application.add_action(&action);
    if !accels.is_empty() {
        application.set_accels_for_action(&format!("app.{name}"), accels);
    }
}

fn install_actions(application: &adw::Application, app: &Rc<App>) {
    add_action(application, app, "connect", &[], |app| {
        glib::spawn_future_local(async move { app.connect_interactive().await });
    });
    add_action(application, app, "sync", &["<Control>r", "F5"], |app| {
        app.sync_now()
    });
    add_action(application, app, "disconnect", &[], |app| app.disconnect());
    add_action(application, app, "sync-contacts", &[], |app| {
        glib::spawn_future_local(async move { app.sync_contacts().await });
    });
    add_action(application, app, "new-message", &["<Control>n"], |app| {
        app.new_message()
    });
    add_action(application, app, "search", &["<Control>f"], |app| {
        app.w.split.set_show_content(false);
        app.w.search.grab_focus();
    });
    add_action(
        application,
        app,
        "preferences",
        &["<Control>comma"],
        |app| {
            preferences::show(&app);
        },
    );
    add_action(application, app, "open-logs", &[], |app| {
        let dir = btsms::logging::log_dir();
        let launcher = gtk4::FileLauncher::new(Some(&gio::File::for_path(&dir)));
        let weak = Rc::downgrade(&app);
        launcher.launch(Some(&app.w.window), gio::Cancellable::NONE, move |res| {
            if let (Err(e), Some(app)) = (res, weak.upgrade()) {
                app.toast(&format!("Cannot open {}: {e}", dir.display()));
            }
        });
    });
    add_action(application, app, "about", &[], |app| show_about(&app));
    add_action(application, app, "quit", &["<Control>q"], |app| {
        app.application.quit()
    });

    let open = gio::SimpleAction::new("open-conversation", Some(glib::VariantTy::STRING));
    let weak = Rc::downgrade(app);
    open.connect_activate(move |_, param| {
        let (Some(app), Some(peer)) = (weak.upgrade(), param.and_then(|p| p.get::<String>()))
        else {
            return;
        };
        app.w.window.present();
        glib::spawn_future_local(async move { app.open_conversation(&peer).await });
    });
    application.add_action(&open);
}

fn show_about(app: &App) {
    let device = app
        .current_device()
        .map(|d| format!("{} ({})", d.name, d.address))
        .unwrap_or_else(|| "none".into());
    let config = app.config.borrow();
    let debug_info = format!(
        "btsms {}\nGTK {}.{}.{}\nlibadwaita {}.{}.{}\nPhone: {device}\nRegion: {}\nDatabase: {}\nLogs: {}\n",
        env!("CARGO_PKG_VERSION"),
        gtk4::major_version(),
        gtk4::minor_version(),
        gtk4::micro_version(),
        adw::major_version(),
        adw::minor_version(),
        adw::micro_version(),
        config.region().unwrap_or_else(|| "none".into()),
        config.database_path.display(),
        btsms::logging::log_dir().display(),
    );
    let about = adw::AboutDialog::builder()
        .application_name("BT SMS")
        .application_icon("btsms")
        .version(env!("CARGO_PKG_VERSION"))
        .comments("Send and receive SMS from your phone over Bluetooth.")
        .license_type(gtk4::License::MitX11)
        .debug_info(debug_info)
        .debug_info_filename("btsms-debug-info.txt")
        .build();
    about.present(Some(&app.w.window));
}
