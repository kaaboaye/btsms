use crate::gui::state::{SharedAppState, SharedUiState};
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{Adjustment, ApplicationWindow, Button};
use libadwaita as adw;
use libadwaita::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

pub struct SettingsCallbacks {
    pub on_disconnect: Box<dyn Fn()>,
    pub on_connect: Box<dyn Fn()>,
    pub on_sync_contacts: Box<dyn Fn()>,
    pub on_import_messages: Box<dyn Fn()>,
    pub on_reset_db: Box<dyn Fn()>,
}

fn create_action_row(title: &str, subtitle: &str, button_label: &str) -> (adw::ActionRow, Button) {
    let row = adw::ActionRow::builder()
        .title(title)
        .subtitle(subtitle)
        .build();
    let btn = Button::with_label(button_label);
    btn.set_valign(gtk4::Align::Center);
    row.add_suffix(&btn);
    row.set_activatable_widget(Some(&btn));
    (row, btn)
}

pub fn show_settings_dialog(
    window: &ApplicationWindow,
    app_state: SharedAppState,
    _ui_state: SharedUiState,
    callbacks: SettingsCallbacks,
) {
    let dialog = adw::PreferencesWindow::builder()
        .transient_for(window)
        .modal(true)
        .title("Settings")
        .default_width(600)
        .default_height(700)
        .build();

    // ========== ACTIONS PAGE ==========
    let actions_page = adw::PreferencesPage::new();
    actions_page.set_title("Actions");
    actions_page.set_icon_name(Some("system-run-symbolic"));

    let actions_group = adw::PreferencesGroup::new();
    actions_group.set_title("Connection");

    let (connect_row, connect_btn) = create_action_row(
        "Connect",
        "Connect to a Bluetooth device",
        "Connect",
    );
    connect_btn.add_css_class("suggested-action");
    actions_group.add(&connect_row);

    let (disconnect_row, disconnect_btn) = create_action_row(
        "Disconnect",
        "Disconnect from the current device",
        "Disconnect",
    );
    actions_group.add(&disconnect_row);

    actions_page.add(&actions_group);

    let sync_group = adw::PreferencesGroup::new();
    sync_group.set_title("Synchronization");

    let (sync_row, sync_btn) = create_action_row(
        "Sync Contacts",
        "Pull contacts from the connected phone",
        "Sync",
    );
    sync_group.add(&sync_row);

    let (import_row, import_btn) = create_action_row(
        "Import Messages",
        "Import SMS messages from the phone",
        "Import",
    );
    sync_group.add(&import_row);

    actions_page.add(&sync_group);
    dialog.add(&actions_page);

    // ========== SETTINGS PAGE ==========
    let settings_page = adw::PreferencesPage::new();
    settings_page.set_title("Settings");
    settings_page.set_icon_name(Some("emblem-system-symbolic"));

    let connection_group = adw::PreferencesGroup::new();
    connection_group.set_title("Connection");

    let auto_connect_row = adw::SwitchRow::builder()
        .title("Auto Connect")
        .subtitle("Automatically connect to the last used device on startup")
        .build();
    connection_group.add(&auto_connect_row);

    settings_page.add(&connection_group);

    let polling_group = adw::PreferencesGroup::new();
    polling_group.set_title("Message Polling");

    let polling_enabled_row = adw::SwitchRow::builder()
        .title("Enable Message Polling")
        .subtitle("Periodically check for new messages")
        .build();
    polling_group.add(&polling_enabled_row);

    let frequency_row = adw::SpinRow::new(
        Some(&Adjustment::new(15.0, 5.0, 300.0, 5.0, 15.0, 0.0)),
        1.0,
        0,
    );
    frequency_row.set_title("Polling Interval");
    frequency_row.set_subtitle("Seconds between message checks");
    polling_group.add(&frequency_row);

    settings_page.add(&polling_group);

    let danger_group = adw::PreferencesGroup::new();
    danger_group.set_title("Danger Zone");

    let (reset_row, reset_btn) = create_action_row(
        "Reset Database",
        "Delete all messages and contacts permanently",
        "Reset",
    );
    reset_btn.add_css_class("destructive-action");
    danger_group.add(&reset_row);

    settings_page.add(&danger_group);
    dialog.add(&settings_page);

    // ========== LOAD CONFIG ==========
    let app_state_load = app_state.clone();
    let auto_connect_row_load = auto_connect_row.clone();
    let polling_enabled_row_load = polling_enabled_row.clone();
    let frequency_row_load = frequency_row.clone();
    let disconnect_btn_load = disconnect_btn.clone();
    let sync_btn_load = sync_btn.clone();
    let import_btn_load = import_btn.clone();

    glib::spawn_future_local(async move {
        let state = app_state_load.lock().await;
        let config = &state.config;

        auto_connect_row_load.set_active(config.auto_connect);
        polling_enabled_row_load.set_active(config.message_polling_enabled);
        frequency_row_load.set_value(config.message_polling_interval as f64);

        let is_connected = state.map_client.is_some();
        disconnect_btn_load.set_sensitive(is_connected);
        sync_btn_load.set_sensitive(is_connected);
        import_btn_load.set_sensitive(is_connected);
    });

    // Frequency row sensitivity based on polling enabled
    let frequency_row_ref = frequency_row.clone();
    polling_enabled_row.connect_active_notify(move |row| {
        frequency_row_ref.set_sensitive(row.is_active());
    });

    // ========== SAVE HANDLERS ==========
    let app_state_auto = app_state.clone();
    auto_connect_row.connect_active_notify(move |row| {
        let is_active = row.is_active();
        let state = app_state_auto.clone();
        glib::spawn_future_local(async move {
            let mut state_lock = state.lock().await;
            state_lock.config.auto_connect = is_active;
            let _ = state_lock.config.save();
        });
    });

    let app_state_poll = app_state.clone();
    polling_enabled_row.connect_active_notify(move |row| {
        let is_active = row.is_active();
        let state = app_state_poll.clone();
        glib::spawn_future_local(async move {
            let mut state_lock = state.lock().await;
            state_lock.config.message_polling_enabled = is_active;
            let _ = state_lock.config.save();
        });
    });

    let app_state_freq = app_state.clone();
    frequency_row.connect_value_notify(move |row| {
        let value = row.value() as u32;
        let state = app_state_freq.clone();
        glib::spawn_future_local(async move {
            let mut state_lock = state.lock().await;
            state_lock.config.message_polling_interval = value;
            let _ = state_lock.config.save();
        });
    });

    // ========== ACTION HANDLERS ==========
    let callbacks = Rc::new(RefCell::new(Some(callbacks)));

    let dialog_ref = dialog.clone();
    let callbacks_ref = callbacks.clone();
    connect_btn.connect_clicked(move |_| {
        dialog_ref.close();
        if let Some(ref cb) = *callbacks_ref.borrow() {
            (cb.on_connect)();
        }
    });

    let dialog_ref = dialog.clone();
    let callbacks_ref = callbacks.clone();
    disconnect_btn.connect_clicked(move |_| {
        dialog_ref.close();
        if let Some(ref cb) = *callbacks_ref.borrow() {
            (cb.on_disconnect)();
        }
    });

    let dialog_ref = dialog.clone();
    let callbacks_ref = callbacks.clone();
    sync_btn.connect_clicked(move |_| {
        dialog_ref.close();
        if let Some(ref cb) = *callbacks_ref.borrow() {
            (cb.on_sync_contacts)();
        }
    });

    let dialog_ref = dialog.clone();
    let callbacks_ref = callbacks.clone();
    import_btn.connect_clicked(move |_| {
        dialog_ref.close();
        if let Some(ref cb) = *callbacks_ref.borrow() {
            (cb.on_import_messages)();
        }
    });

    let dialog_ref = dialog.clone();
    let callbacks_ref = callbacks.clone();
    reset_btn.connect_clicked(move |_| {
        dialog_ref.close();
        if let Some(ref cb) = *callbacks_ref.borrow() {
            (cb.on_reset_db)();
        }
    });

    dialog.present();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_settings_callbacks_struct() {
        let _callbacks = SettingsCallbacks {
            on_disconnect: Box::new(|| {}),
            on_connect: Box::new(|| {}),
            on_sync_contacts: Box::new(|| {}),
            on_import_messages: Box::new(|| {}),
            on_reset_db: Box::new(|| {}),
        };
    }

    #[test]
    fn test_create_action_row() {
        // This would require GTK initialization in tests, so we just verify compilation
        let _: fn(&str, &str, &str) -> (adw::ActionRow, Button) = create_action_row;
    }
}
