use crate::gui::connection::{
    complete_connection_setup, connect_to_device, disconnect_device, ConnectResult,
};
use crate::gui::dialogs::{
    select_paired_device, show_error_dialog_with_copy, show_pairing_instructions,
    PhoneSelectionResult,
};
use crate::gui::handlers::{
    import_inbox_messages, import_sent_messages, refresh_conversations, save_message_to_db,
};
use crate::gui::helpers::clear_list_box;
use crate::gui::message_bubble::{add_message_bubble, scroll_to_bottom};
use crate::gui::settings::{show_settings_dialog, SettingsCallbacks};
use crate::gui::state::{SharedAppState, SharedUiState};
use btsms::bluetooth::{DeviceManager, PbapClient};
use btsms::contacts::normalize_e164;
use btsms::db;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{ApplicationWindow, Box as GtkBox, Button, Entry, Label, ListBox, Orientation, Popover};
use libadwaita as adw;
use libadwaita::prelude::*;

/// Sets up the new message button click handler.
pub fn setup_new_message_handler(
    new_message_btn: &Button,
    app_state: SharedAppState,
    ui_state: SharedUiState,
) {
    let ui_state_clone = ui_state.clone();
    let app_state_clone = app_state.clone();

    new_message_btn.connect_clicked(move |_| {
        let ui = ui_state_clone.borrow();
        ui.conversation_list.unselect_all();
        ui.recipient_entry.set_text("");
        ui.recipient_entry.set_sensitive(true);
        ui.recipient_entry.grab_focus();
        clear_list_box(&ui.message_list);

        let app_state_inner = app_state_clone.clone();
        glib::spawn_future_local(async move {
            let mut state = app_state_inner.lock().await;
            state.current_conversation = None;
        });
    });
}

/// Sets up the conversation list selection handler.
pub fn setup_conversation_selection_handler(
    conversation_list: &ListBox,
    app_state: SharedAppState,
    ui_state: SharedUiState,
) {
    let ui_state_clone = ui_state.clone();
    let app_state_clone = app_state.clone();

    conversation_list.connect_row_selected(move |_, row| {
        if let Some(row) = row {
            let phone = row.widget_name().to_string();
            if phone.is_empty() {
                return;
            }

            let display_name = row
                .child()
                .and_then(|child| child.first_child())
                .and_then(|child| child.first_child())
                .and_then(|child| child.first_child())
                .and_then(|widget| widget.downcast::<Label>().ok())
                .map(|label| label.text().to_string())
                .unwrap_or_else(|| phone.clone());

            let ui = ui_state_clone.borrow();
            ui.recipient_entry.set_text(&display_name);
            ui.recipient_entry.set_sensitive(false);

            let app_state_inner = app_state_clone.clone();
            let ui_state_inner = ui_state_clone.clone();
            let phone_clone = phone.clone();

            glib::spawn_future_local(async move {
                let mut state = app_state_inner.lock().await;
                state.current_conversation = Some(phone_clone.clone());

                if let Some(pool) = &state.db_pool {
                    let _ = db::mark_conversation_read(pool, &phone_clone).await;

                    if let Ok(messages) =
                        db::get_messages_for_conversation(pool, &phone_clone, 100).await
                    {
                        let ui = ui_state_inner.borrow();
                        clear_list_box(&ui.message_list);

                        for msg in messages {
                            let is_outgoing = msg.direction == db::MessageDirection::Outgoing;
                            add_message_bubble(
                                &ui.message_list,
                                &msg.body,
                                is_outgoing,
                                &msg.received_at,
                            );
                        }

                        scroll_to_bottom(&ui.message_scroll);
                    }
                }
            });
        }
    });
}

/// Sets up the device switcher button handler.
pub fn setup_device_switcher_handler(
    device_switch_button: &Button,
    app_state: SharedAppState,
    ui_state: SharedUiState,
    status_label: Label,
    send_button: Button,
) {
    let app_state_clone = app_state.clone();
    let ui_state_clone = ui_state.clone();
    let status_clone = status_label.clone();
    let send_btn_clone = send_button.clone();

    device_switch_button.connect_clicked(move |btn| {
        let state = app_state_clone.clone();
        let status = status_clone.clone();
        let ui_state = ui_state_clone.clone();
        let send_btn = send_btn_clone.clone();
        let switch_btn = btn.clone();

        glib::spawn_future_local(async move {
            let popover = Popover::new();
            let content = GtkBox::new(Orientation::Vertical, 4);
            content.set_margin_start(8);
            content.set_margin_end(8);
            content.set_margin_top(8);
            content.set_margin_bottom(8);

            let current_addr = {
                let state_lock = state.lock().await;
                state_lock.device_address.clone()
            };

            let manager = match DeviceManager::new().await {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("Failed to get device manager: {}", e);
                    return;
                }
            };

            let phones = match manager.get_all_paired_phones().await {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("Failed to get phones: {}", e);
                    return;
                }
            };

            if phones.is_empty() {
                let label = Label::new(Some("No devices available"));
                label.add_css_class("dim-label");
                content.append(&label);
            } else {
                let label = Label::new(Some("Switch to:"));
                label.set_halign(gtk4::Align::Start);
                label.add_css_class("heading");
                content.append(&label);

                for phone in phones {
                    let is_current = current_addr.as_ref() == Some(&phone.address);
                    let device_btn = gtk4::Button::new();

                    let btn_box = GtkBox::new(Orientation::Vertical, 2);
                    btn_box.set_margin_start(4);
                    btn_box.set_margin_end(4);
                    btn_box.set_margin_top(4);
                    btn_box.set_margin_bottom(4);

                    let name_label = Label::new(Some(&phone.name));
                    name_label.set_halign(gtk4::Align::Start);
                    if is_current {
                        name_label.add_css_class("heading");
                    }

                    let status_text = if is_current {
                        "Current device".to_string()
                    } else if phone.connected {
                        "Connected".to_string()
                    } else {
                        "Not connected".to_string()
                    };
                    let status_label_btn = Label::new(Some(&status_text));
                    status_label_btn.set_halign(gtk4::Align::Start);
                    status_label_btn.add_css_class("dim-label");
                    status_label_btn.add_css_class("caption");

                    btn_box.append(&name_label);
                    btn_box.append(&status_label_btn);
                    device_btn.set_child(Some(&btn_box));

                    if is_current {
                        device_btn.set_sensitive(false);
                    }

                    let state_switch = state.clone();
                    let status_switch = status.clone();
                    let ui_state_switch = ui_state.clone();
                    let send_btn_switch = send_btn.clone();
                    let popover_clone = popover.clone();
                    let switch_btn_clone = switch_btn.clone();

                    device_btn.connect_clicked(move |_| {
                        let device = phone.clone();
                        let state_inner = state_switch.clone();
                        let status_inner = status_switch.clone();
                        let ui_state_inner = ui_state_switch.clone();
                        let send_btn_inner = send_btn_switch.clone();
                        let switch_btn_inner = switch_btn_clone.clone();

                        popover_clone.popdown();

                        glib::spawn_future_local(async move {
                            {
                                let mut state_lock = state_inner.lock().await;
                                if let Some(mut map_client) = state_lock.map_client.take() {
                                    let _ = map_client.disconnect().await;
                                }
                                state_lock.device_address = None;
                                state_lock.device_name = None;
                            }

                            status_inner.set_text(&format!("Switching to {}...", device.name));

                            match connect_to_device(device.clone(), state_inner.clone(), &status_inner)
                                .await
                            {
                                ConnectResult::Success { name } => {
                                    complete_connection_setup(
                                        state_inner,
                                        ui_state_inner,
                                        &send_btn_inner,
                                        &switch_btn_inner,
                                        &status_inner,
                                        &name,
                                    )
                                    .await;
                                }
                                ConnectResult::Failed(e) => {
                                    eprintln!("Failed to switch device: {}", e);
                                    status_inner.set_text("Switch failed");
                                    send_btn_inner.set_sensitive(false);
                                }
                            }
                        });
                    });

                    content.append(&device_btn);
                }
            }

            popover.set_child(Some(&content));
            popover.set_parent(&switch_btn);
            popover.popup();
        });
    });
}

/// Sets up the send button and message entry handlers.
pub fn setup_send_handler(
    send_button: &Button,
    message_entry: &Entry,
    app_state: SharedAppState,
    ui_state: SharedUiState,
    status_label: Label,
    window: ApplicationWindow,
) {
    let send_handler = {
        let app_state = app_state.clone();
        let ui_state = ui_state.clone();
        let status = status_label.clone();
        let window = window.clone();

        move || {
            let ui = ui_state.borrow();
            let recipient_text = ui.recipient_entry.text().to_string();
            let message = ui.message_entry.text().to_string();

            if recipient_text.is_empty() || message.is_empty() {
                return;
            }

            let app_state_clone = app_state.clone();
            let ui_state_clone = ui_state.clone();
            let status_clone = status.clone();
            let window_clone = window.clone();

            glib::spawn_future_local(async move {
                let mut state_lock = app_state_clone.lock().await;

                let is_new_conversation = state_lock.current_conversation.is_none();
                let recipient = state_lock
                    .current_conversation
                    .clone()
                    .unwrap_or(recipient_text);

                if let Some(map_client) = &state_lock.map_client {
                    status_clone.set_text("Sending...");

                    match map_client.send_sms(&recipient, &message).await {
                        Ok(_) => {
                            status_clone.set_text("Sent");

                            {
                                let ui = ui_state_clone.borrow();
                                add_message_bubble(
                                    &ui.message_list,
                                    &message,
                                    true,
                                    &chrono::Local::now().format("%H:%M").to_string(),
                                );
                                scroll_to_bottom(&ui.message_scroll);
                                ui.message_entry.set_text("");
                            }

                            if let Some(pool) = &state_lock.db_pool {
                                save_message_to_db(pool, &recipient, &message, "OUTGOING").await;
                            }

                            let normalized =
                                normalize_e164(&recipient).unwrap_or_else(|_| recipient.clone());

                            if is_new_conversation {
                                state_lock.current_conversation = Some(normalized.clone());

                                let ui = ui_state_clone.borrow();
                                ui.recipient_entry.set_sensitive(false);
                            }

                            drop(state_lock);
                            refresh_conversations(app_state_clone.clone(), ui_state_clone.clone())
                                .await;

                            if is_new_conversation {
                                let ui = ui_state_clone.borrow();
                                let mut row_index = 0;
                                while let Some(row) = ui.conversation_list.row_at_index(row_index) {
                                    if row.widget_name() == normalized {
                                        ui.conversation_list.select_row(Some(&row));
                                        break;
                                    }
                                    row_index += 1;
                                }
                            }
                        }
                        Err(e) => {
                            status_clone.set_text("Send failed");
                            show_error_dialog_with_copy(
                                &window_clone,
                                "Send Error",
                                &format!("{}", e),
                            );
                        }
                    }
                }
            });
        }
    };

    let send_handler_click = send_handler.clone();
    send_button.connect_clicked(move |_| {
        send_handler_click();
    });

    let send_handler_enter = send_handler;
    message_entry.connect_activate(move |_| {
        send_handler_enter();
    });
}

/// Sets up the settings button handler with all settings callbacks.
pub fn setup_settings_handler(
    settings_button: &Button,
    app_state: SharedAppState,
    ui_state: SharedUiState,
    window: ApplicationWindow,
    status_label: Label,
    device_switch_button: Button,
    send_button: Button,
) {
    let app_state_clone = app_state.clone();
    let ui_state_clone = ui_state.clone();
    let window_clone = window.clone();
    let status_clone = status_label.clone();
    let device_switch_clone = device_switch_button.clone();
    let send_btn_clone = send_button.clone();

    settings_button.connect_clicked(move |_| {
        let app_state = app_state_clone.clone();
        let ui_state = ui_state_clone.clone();
        let window = window_clone.clone();
        let status = status_clone.clone();
        let device_switch = device_switch_clone.clone();
        let send_btn = send_btn_clone.clone();

        let callbacks = create_settings_callbacks(
            app_state.clone(),
            ui_state.clone(),
            window.clone(),
            status.clone(),
            device_switch.clone(),
            send_btn.clone(),
        );

        show_settings_dialog(&window, app_state, ui_state, callbacks);
    });
}

fn create_settings_callbacks(
    app_state: SharedAppState,
    ui_state: SharedUiState,
    window: ApplicationWindow,
    status: Label,
    device_switch: Button,
    send_btn: Button,
) -> SettingsCallbacks {
    SettingsCallbacks {
        on_disconnect: Box::new({
            let app_state = app_state.clone();
            let status = status.clone();
            let device_switch = device_switch.clone();
            let send_btn = send_btn.clone();
            move || {
                let state = app_state.clone();
                let status = status.clone();
                let device_switch = device_switch.clone();
                let send_btn = send_btn.clone();
                glib::spawn_future_local(async move {
                    disconnect_device(state, &status, &device_switch, &send_btn).await;
                });
            }
        }),
        on_connect: Box::new({
            let app_state = app_state.clone();
            let ui_state = ui_state.clone();
            let window = window.clone();
            let status = status.clone();
            let device_switch = device_switch.clone();
            let send_btn = send_btn.clone();
            move || {
                let state = app_state.clone();
                let ui_state = ui_state.clone();
                let window = window.clone();
                let status = status.clone();
                let device_switch = device_switch.clone();
                let send_btn = send_btn.clone();

                glib::spawn_future_local(async move {
                    status.set_text("Connecting...");

                    match select_paired_device(&window).await {
                        PhoneSelectionResult::Selected(device) => {
                            match connect_to_device(device, state.clone(), &status).await {
                                ConnectResult::Success { name } => {
                                    complete_connection_setup(
                                        state,
                                        ui_state,
                                        &send_btn,
                                        &device_switch,
                                        &status,
                                        &name,
                                    )
                                    .await;
                                }
                                ConnectResult::Failed(error_msg) => {
                                    eprintln!("MAP connection failed: {}", error_msg);
                                    status.set_text("Connection failed");

                                    let error_text = format!(
                                        "Failed to connect to MAP:\n\n{}\n\n\
                                        Try: systemctl --user start obex\n\n\
                                        FOR IPHONE: Enable 'Show Notifications' in:\n\
                                        Settings -> Bluetooth -> [Computer] -> Show Notifications",
                                        error_msg
                                    );

                                    show_error_dialog_with_copy(
                                        &window,
                                        "Connection Failed",
                                        &error_text,
                                    );
                                }
                            }
                        }
                        PhoneSelectionResult::NoneFound => {
                            status.set_text("No paired phone found");
                            show_pairing_instructions(&window);
                        }
                        PhoneSelectionResult::Cancelled => {
                            status.set_text("Cancelled");
                        }
                        PhoneSelectionResult::Error(e) => {
                            status.set_text("Error");
                            show_error_dialog_with_copy(&window, "Connection Error", &e);
                        }
                    }
                });
            }
        }),
        on_sync_contacts: Box::new({
            let app_state = app_state.clone();
            let window = window.clone();
            let status = status.clone();
            move || {
                let state = app_state.clone();
                let window = window.clone();
                let status = status.clone();

                glib::spawn_future_local(async move {
                    status.set_text("Syncing contacts...");
                    let state_lock = state.lock().await;

                    if let Some(device_addr) = &state_lock.device_address {
                        let mut pbap_client = PbapClient::new(device_addr.clone());

                        match pbap_client.connect().await {
                            Ok(_) => {
                                match pbap_client.pull_all_contacts().await {
                                    Ok(vcards) => {
                                        if let Some(contact_mgr) = &state_lock.contact_manager {
                                            match contact_mgr
                                                .sync_from_vcards(&vcards, device_addr)
                                                .await
                                            {
                                                Ok(count) => {
                                                    status.set_text(&format!(
                                                        "Synced {} contacts",
                                                        count
                                                    ));
                                                }
                                                Err(e) => {
                                                    status.set_text("Sync failed");
                                                    show_error_dialog_with_copy(
                                                        &window,
                                                        "Sync Error",
                                                        &format!("{}", e),
                                                    );
                                                }
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        status.set_text("Failed to pull contacts");
                                        show_error_dialog_with_copy(
                                            &window,
                                            "Error",
                                            &format!("{}", e),
                                        );
                                    }
                                }
                                let _ = pbap_client.disconnect().await;
                            }
                            Err(e) => {
                                status.set_text("PBAP failed");
                                show_error_dialog_with_copy(&window, "Error", &format!("{}", e));
                            }
                        }
                    } else {
                        status.set_text("Not connected");
                    }
                });
            }
        }),
        on_import_messages: Box::new({
            let app_state = app_state.clone();
            let ui_state = ui_state.clone();
            let window = window.clone();
            let status = status.clone();
            move || {
                let state = app_state.clone();
                let ui_state = ui_state.clone();
                let window = window.clone();
                let status = status.clone();

                glib::spawn_future_local(async move {
                    status.set_text("Importing SMS...");
                    let state_lock = state.lock().await;

                    if let Some(map_client) = &state_lock.map_client {
                        let mut imported_count = 0;
                        let mut error_messages = Vec::new();

                        status.set_text("Importing inbox...");
                        if let Some(pool) = &state_lock.db_pool {
                            match import_inbox_messages(map_client, pool).await {
                                Ok(count) => imported_count += count,
                                Err(e) => error_messages.push(e),
                            }

                            status.set_text("Importing sent...");
                            imported_count += import_sent_messages(map_client, pool).await;
                        }

                        drop(state_lock);
                        refresh_conversations(state.clone(), ui_state).await;

                        if error_messages.is_empty() {
                            status.set_text(&format!("Imported {} messages", imported_count));
                        } else {
                            status.set_text("Import failed");
                            show_error_dialog_with_copy(
                                &window,
                                "Import Errors",
                                &error_messages.join("\n"),
                            );
                        }
                    } else {
                        status.set_text("Not connected");
                    }
                });
            }
        }),
        on_reset_db: Box::new({
            let app_state = app_state.clone();
            let ui_state = ui_state.clone();
            let window = window.clone();
            let status = status.clone();
            move || {
                let state = app_state.clone();
                let ui_state = ui_state.clone();
                let window = window.clone();
                let status = status.clone();

                let dialog = adw::AlertDialog::builder()
                    .heading("Reset Database?")
                    .body("This will delete all messages and contacts. This action cannot be undone.")
                    .build();

                dialog.add_response("cancel", "Cancel");
                dialog.add_response("reset", "Reset");
                dialog.set_response_appearance("reset", adw::ResponseAppearance::Destructive);
                dialog.set_default_response(Some("cancel"));
                dialog.set_close_response("cancel");

                let state_clone = state.clone();
                let ui_state_inner = ui_state.clone();
                let status_inner = status.clone();

                dialog.connect_response(None, move |_, response| {
                    if response == "reset" {
                        let state = state_clone.clone();
                        let ui_state = ui_state_inner.clone();
                        let status = status_inner.clone();

                        glib::spawn_future_local(async move {
                            status.set_text("Resetting database...");

                            let db_path = db::default_database_path();
                            let path_str = db_path.to_str().unwrap();

                            // Close current pool and reset
                            {
                                let mut state_lock = state.lock().await;
                                // Drop the old pool
                                if let Some(pool) = state_lock.db_pool.take() {
                                    pool.close().await;
                                }

                                // Reset and get new pool
                                match db::reset_database(path_str).await {
                                    Ok(new_pool) => {
                                        state_lock.db_pool = Some(new_pool);
                                    }
                                    Err(e) => {
                                        eprintln!("Failed to reset database: {}", e);
                                        status.set_text("Reset failed");
                                        return;
                                    }
                                }
                            }

                            // Clear UI
                            {
                                let ui = ui_state.borrow();
                                clear_list_box(&ui.conversation_list);
                                clear_list_box(&ui.message_list);
                                ui.recipient_entry.set_text("");
                                ui.message_entry.set_text("");
                            }

                            status.set_text("Database reset complete");
                        });
                    }
                });

                dialog.present(Some(&window));
            }
        }),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_module_compiles() {
        assert!(true);
    }
}
