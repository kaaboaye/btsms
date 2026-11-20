use crate::gui::app::App;
use btsms::config::{MAX_POLL_INTERVAL, MIN_POLL_INTERVAL, region_from_locale};
use btsms::contacts::phone_normalizer;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use libadwaita::prelude::*;
use std::rc::Rc;

fn save(app: &App) {
    if let Err(e) = app.config.borrow().save() {
        app.toast(&format!("Could not save settings: {e}"));
    }
}

pub fn show(app: &Rc<App>) {
    let config = app.config.borrow().clone();
    let dialog = adw::PreferencesDialog::builder()
        .title("Preferences")
        .search_enabled(false)
        .build();
    let page = adw::PreferencesPage::new();

    // ---------- phone ----------
    let phone_group = adw::PreferencesGroup::builder().title("Phone").build();
    let device = app.current_device();
    let device_row = adw::ActionRow::builder()
        .title(
            device
                .as_ref()
                .map(|d| glib::markup_escape_text(&d.name).to_string())
                .unwrap_or_else(|| "No phone selected".into()),
        )
        .subtitle(
            device
                .as_ref()
                .map(|d| d.address.clone())
                .unwrap_or_default(),
        )
        .build();
    let change = gtk4::Button::builder()
        .label(if device.is_some() {
            "Change…"
        } else {
            "Connect…"
        })
        .valign(gtk4::Align::Center)
        .build();
    let weak = Rc::downgrade(app);
    let dialog_ref = dialog.clone();
    change.connect_clicked(move |_| {
        dialog_ref.close();
        if let Some(app) = weak.upgrade() {
            glib::spawn_future_local(async move { app.connect_interactive().await });
        }
    });
    device_row.add_suffix(&change);
    phone_group.add(&device_row);

    let auto_connect = adw::SwitchRow::builder()
        .title("Connect Automatically")
        .subtitle("Connect to the last used phone when the app starts")
        .active(config.auto_connect)
        .build();
    let weak = Rc::downgrade(app);
    auto_connect.connect_active_notify(move |row| {
        if let Some(app) = weak.upgrade() {
            app.config.borrow_mut().auto_connect = row.is_active();
            save(&app);
        }
    });
    phone_group.add(&auto_connect);
    page.add(&phone_group);

    // ---------- messages ----------
    let msg_group = adw::PreferencesGroup::builder()
        .title("Messages")
        .description(
            "iPhones report new messages instantly. Periodic checks matter mostly for Android.",
        )
        .build();
    let polling = adw::SwitchRow::builder()
        .title("Check Periodically")
        .active(config.message_polling_enabled)
        .build();
    let interval = adw::SpinRow::builder()
        .title("Check Interval")
        .subtitle("Seconds")
        .adjustment(&gtk4::Adjustment::new(
            f64::from(config.message_polling_interval),
            f64::from(MIN_POLL_INTERVAL),
            f64::from(MAX_POLL_INTERVAL),
            5.0,
            60.0,
            0.0,
        ))
        .sensitive(config.message_polling_enabled)
        .build();
    let weak = Rc::downgrade(app);
    let interval_ref = interval.clone();
    polling.connect_active_notify(move |row| {
        interval_ref.set_sensitive(row.is_active());
        if let Some(app) = weak.upgrade() {
            app.config.borrow_mut().message_polling_enabled = row.is_active();
            save(&app);
            app.apply_link_settings();
        }
    });
    let weak = Rc::downgrade(app);
    interval.connect_value_notify(move |row| {
        if let Some(app) = weak.upgrade() {
            app.config.borrow_mut().message_polling_interval = row.value() as u32;
            save(&app);
            app.apply_link_settings();
        }
    });
    msg_group.add(&polling);
    msg_group.add(&interval);

    let locale_hint = region_from_locale()
        .map(|r| {
            format!("Region for numbers without a country code. Empty = from system locale ({r}).")
        })
        .unwrap_or_else(|| "Region for numbers without a country code, e.g. PL, DE, US.".into());
    let region = adw::EntryRow::builder()
        .title("Phone Number Region")
        .text(config.default_region.clone().unwrap_or_default())
        .show_apply_button(true)
        .tooltip_text(&locale_hint)
        .build();
    let weak = Rc::downgrade(app);
    region.connect_apply(move |row| {
        let Some(app) = weak.upgrade() else { return };
        let text = row.text().trim().to_ascii_uppercase();
        if !text.is_empty() && phone_normalizer::parse_region(&text).is_none() {
            app.toast(&format!("'{text}' is not a known region code"));
            return;
        }
        let value = (!text.is_empty()).then_some(text);
        app.config.borrow_mut().default_region = value;
        save(&app);
        let region = app.config.borrow().region();
        phone_normalizer::set_default_region(region.as_deref());
        app.toast(&format!(
            "Phone number region: {}",
            region.unwrap_or_else(|| "none".into())
        ));
        glib::spawn_future_local(async move { app.renormalize().await });
    });
    msg_group.add(&region);
    page.add(&msg_group);

    // ---------- troubleshooting ----------
    let debug_group = adw::PreferencesGroup::builder()
        .title("Troubleshooting")
        .build();
    let logs = adw::ActionRow::builder()
        .title("Log Files")
        .subtitle(btsms::logging::log_dir().display().to_string())
        .subtitle_selectable(true)
        .build();
    let open_logs = gtk4::Button::builder()
        .label("Open")
        .valign(gtk4::Align::Center)
        .action_name("app.open-logs")
        .build();
    logs.add_suffix(&open_logs);
    debug_group.add(&logs);
    let db_row = adw::ActionRow::builder()
        .title("Database")
        .subtitle(config.database_path.display().to_string())
        .subtitle_selectable(true)
        .build();
    debug_group.add(&db_row);
    page.add(&debug_group);

    // ---------- danger ----------
    let danger = adw::PreferencesGroup::builder().title("Local Data").build();
    let clear_row = adw::ActionRow::builder()
        .title("Delete All Messages and Contacts")
        .subtitle("Only on this computer")
        .build();
    let clear = gtk4::Button::builder()
        .label("Delete…")
        .valign(gtk4::Align::Center)
        .css_classes(["destructive-action"])
        .build();
    let weak = Rc::downgrade(app);
    let dialog_ref = dialog.clone();
    clear.connect_clicked(move |_| {
        dialog_ref.close();
        if let Some(app) = weak.upgrade() {
            glib::spawn_future_local(async move { app.clear_database().await });
        }
    });
    clear_row.add_suffix(&clear);
    danger.add(&clear_row);
    page.add(&danger);

    dialog.add(&page);
    dialog.present(Some(&app.w.window));
}
