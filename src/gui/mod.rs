mod chat_view;
mod connection;
mod conversation_row;
mod dialogs;
mod handlers;
mod header_bar;
mod message_bubble;
mod sidebar;
mod state;

use btsms::bluetooth::{DeviceManager, PbapClient};
use btsms::contacts::{normalize_e164, ContactManager};
use btsms::db;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{ApplicationWindow, Box as GtkBox, Label, Orientation, Paned, Popover};
use libadwaita::prelude::*;
use libadwaita as adw;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use tokio::sync::Mutex;

use chat_view::build_chat_view;
use connection::{
    check_obexd_service, connect_to_device, determine_auto_connect_device, start_ancs_listener,
    AutoConnectResult, ConnectResult,
};
use dialogs::{
    select_paired_device, show_error_dialog_with_copy, show_pairing_instructions,
    PhoneSelectionResult,
};
use handlers::{
    import_inbox_messages, import_sent_messages, load_conversations, refresh_conversations,
    save_message_to_db, start_message_poll_timer, start_refresh_timer,
};
use header_bar::build_header_bar;
use message_bubble::{add_message_bubble, scroll_to_bottom};
use sidebar::build_sidebar;
use state::{AppState, UiState};

pub fn build_ui(app: &adw::Application) {
    // Create main window
    let window = ApplicationWindow::builder()
        .application(app)
        .title("Bluetooth SMS")
        .default_width(1000)
        .default_height(700)
        .build();

    // Main container with header
    let main_box = GtkBox::new(Orientation::Vertical, 0);

    // Build header bar
    let header_widgets = build_header_bar();
    main_box.append(&header_widgets.header);

    // Main content: Paned layout with sidebar and chat view
    let paned = Paned::new(Orientation::Horizontal);
    paned.set_position(280);
    paned.set_shrink_start_child(false);
    paned.set_shrink_end_child(false);

    // Build sidebar
    let sidebar_widgets = build_sidebar();
    paned.set_start_child(Some(&sidebar_widgets.container));

    // Build chat view
    let chat_widgets = build_chat_view();
    paned.set_end_child(Some(&chat_widgets.container));

    main_box.append(&paned);
    window.set_child(Some(&main_box));

    // Extract widgets for handlers
    let status_label = header_widgets.status_label;
    let reset_button = header_widgets.reset_button;
    let connect_button = header_widgets.connect_button;
    let sync_button = header_widgets.sync_button;
    let import_button = header_widgets.import_button;
    let device_switch_button = header_widgets.device_switch_button;

    let new_message_btn = sidebar_widgets.new_message_button;
    let conversation_list = sidebar_widgets.conversation_list;

    let recipient_entry = chat_widgets.recipient_entry;
    let message_list = chat_widgets.message_list;
    let message_scroll = chat_widgets.message_scroll;
    let message_entry = chat_widgets.message_entry;
    let send_button = chat_widgets.send_button;

    // ========== SHARED STATE ==========
    let app_state = Arc::new(Mutex::new(AppState::new()));

    let ui_state = Rc::new(RefCell::new(UiState {
        conversation_list: conversation_list.clone(),
        message_list: message_list.clone(),
        recipient_entry: recipient_entry.clone(),
        message_entry: message_entry.clone(),
        message_scroll: message_scroll.clone(),
    }));

    // ========== DATABASE INITIALIZATION & AUTO-CONNECT ==========
    let app_state_init = app_state.clone();
    let status_init = status_label.clone();
    let ui_state_init = ui_state.clone();
    let window_init = window.clone();
    let connect_btn_init = connect_button.clone();
    let sync_btn_init = sync_button.clone();
    let send_btn_init = send_button.clone();
    let import_btn_init = import_button.clone();
    let device_switch_init = device_switch_button.clone();

    glib::spawn_future_local(async move {
        let db_path = dirs::data_local_dir()
            .unwrap_or_else(|| {
                eprintln!("Warning: Could not find data_local_dir, using current directory");
                std::path::PathBuf::from(".")
            })
            .join("btsms")
            .join("messages.db");

        eprintln!("Database path: {:?}", db_path);

        if let Err(e) = std::fs::create_dir_all(db_path.parent().unwrap()) {
            eprintln!("Failed to create database directory: {}", e);
        }

        match db::init_database(db_path.to_str().unwrap()).await {
            Ok(pool) => {
                let contact_manager = ContactManager::new(pool.clone());
                {
                    let mut state = app_state_init.lock().await;
                    state.db_pool = Some(pool.clone());
                    state.contact_manager = Some(contact_manager);
                }

                // Check if obexd service is available
                let obexd_available = match check_obexd_service().await {
                    Ok(true) => {
                        status_init.set_text("Ready");
                        true
                    }
                    Ok(false) | Err(_) => {
                        status_init.set_text("obexd not running");
                        false
                    }
                };

                // Load conversations into sidebar
                load_conversations(pool, ui_state_init.clone()).await;

                // Auto-connect if obexd is available
                if obexd_available {
                    let config = {
                        let state = app_state_init.lock().await;
                        state.config.clone()
                    };

                    if config.auto_connect {
                        status_init.set_text("Auto-connecting...");

                        match determine_auto_connect_device(&config).await {
                            AutoConnectResult::Device(device) => {
                                match connect_to_device(device, app_state_init.clone(), &status_init)
                                    .await
                                {
                                    ConnectResult::Success { name } => {
                                        sync_btn_init.set_sensitive(true);
                                        send_btn_init.set_sensitive(true);
                                        import_btn_init.set_sensitive(true);
                                        device_switch_init.set_visible(true);

                                        // Start ANCS listener, auto-refresh, and message polling
                                        status_init.set_text("Connecting to ANCS...");
                                        start_ancs_listener(
                                            app_state_init.clone(),
                                            ui_state_init.clone(),
                                            status_init.clone(),
                                        )
                                        .await;
                                        start_refresh_timer(app_state_init.clone(), ui_state_init.clone());
                                        start_message_poll_timer(app_state_init.clone(), ui_state_init);

                                        status_init.set_text(&format!("Connected to {}", name));
                                        connect_btn_init.set_label("Disconnect");
                                    }
                                    ConnectResult::Failed(e) => {
                                        eprintln!("Auto-connect failed: {}", e);
                                        status_init.set_text("Ready (auto-connect failed)");
                                    }
                                }
                            }
                            AutoConnectResult::MultipleDevices => {
                                status_init.set_text("Ready (select device)");
                            }
                            AutoConnectResult::NoDevices => {
                                status_init.set_text("Ready (no devices)");
                            }
                            AutoConnectResult::Error(e) => {
                                eprintln!("Auto-connect error: {}", e);
                                status_init.set_text("Ready");
                            }
                        }
                    }
                }
            }
            Err(e) => {
                let error_msg = format!(
                    "Failed to initialize database:\n\n{}\n\n\
                    Database path: {:?}\n\n\
                    This usually means:\n\
                    • Parent directory doesn't exist\n\
                    • No write permissions\n\
                    • Disk is full",
                    e, db_path
                );
                eprintln!("{}", error_msg);
                status_init.set_text("Database error");
                show_error_dialog_with_copy(&window_init, "Database Error", &error_msg);
            }
        }
    });

    // ========== NEW MESSAGE BUTTON ==========
    let ui_state_new = ui_state.clone();
    let app_state_new = app_state.clone();
    new_message_btn.connect_clicked(move |_| {
        let ui = ui_state_new.borrow();
        ui.conversation_list.unselect_all();
        ui.recipient_entry.set_text("");
        ui.recipient_entry.set_sensitive(true);
        ui.recipient_entry.grab_focus();
        while let Some(child) = ui.message_list.first_child() {
            ui.message_list.remove(&child);
        }
        let app_state_clone = app_state_new.clone();
        glib::spawn_future_local(async move {
            let mut state = app_state_clone.lock().await;
            state.current_conversation = None;
        });
    });

    // ========== CONVERSATION SELECTION ==========
    let ui_state_select = ui_state.clone();
    let app_state_select = app_state.clone();
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

            let ui = ui_state_select.borrow();
            ui.recipient_entry.set_text(&display_name);
            ui.recipient_entry.set_sensitive(false);

            let app_state_clone = app_state_select.clone();
            let ui_state_clone = ui_state_select.clone();
            let phone_clone = phone.clone();

            glib::spawn_future_local(async move {
                let mut state = app_state_clone.lock().await;
                state.current_conversation = Some(phone_clone.clone());

                if let Some(pool) = &state.db_pool {
                    let _ = db::mark_conversation_read(pool, &phone_clone).await;

                    if let Ok(messages) =
                        db::get_messages_for_conversation(pool, &phone_clone, 100).await
                    {
                        let ui = ui_state_clone.borrow();
                        while let Some(child) = ui.message_list.first_child() {
                            ui.message_list.remove(&child);
                        }

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

    // ========== CONNECT BUTTON ==========
    let app_state_connect = app_state.clone();
    let status_connect = status_label.clone();
    let sync_btn_connect = sync_button.clone();
    let send_btn_connect = send_button.clone();
    let import_btn_connect = import_button.clone();
    let ui_state_connect = ui_state.clone();
    let window_connect = window.clone();
    let device_switch_connect = device_switch_button.clone();

    connect_button.connect_clicked(move |btn| {
        let state = app_state_connect.clone();
        let status = status_connect.clone();
        let sync_btn = sync_btn_connect.clone();
        let send_btn = send_btn_connect.clone();
        let import_btn = import_btn_connect.clone();
        let ui_state_clone = ui_state_connect.clone();
        let button = btn.clone();
        let window = window_connect.clone();
        let device_switch = device_switch_connect.clone();

        button.set_sensitive(false);
        status.set_text("Connecting...");

        glib::spawn_future_local(async move {
            let selection_result = select_paired_device(&window).await;

            match selection_result {
                PhoneSelectionResult::Selected(device) => {
                    match connect_to_device(device, state.clone(), &status).await {
                        ConnectResult::Success { name } => {
                            sync_btn.set_sensitive(true);
                            send_btn.set_sensitive(true);
                            import_btn.set_sensitive(true);
                            device_switch.set_visible(true);

                            status.set_text("Connecting to ANCS...");
                            start_ancs_listener(state.clone(), ui_state_clone.clone(), status.clone())
                                .await;
                            start_refresh_timer(state.clone(), ui_state_clone.clone());
                            start_message_poll_timer(state.clone(), ui_state_clone.clone());

                            status.set_text(&format!("Connected to {}", name));
                            button.set_label("Disconnect");
                            button.set_sensitive(true);
                        }
                        ConnectResult::Failed(error_msg) => {
                            eprintln!("MAP connection failed: {}", error_msg);
                            status.set_text("Connection failed");

                            let error_text = format!(
                                "Failed to connect to MAP:\n\n{}\n\n\
                                Try: systemctl --user start obex\n\n\
                                FOR IPHONE: Enable 'Show Notifications' in:\n\
                                Settings → Bluetooth → [Computer] → Show Notifications",
                                error_msg
                            );

                            show_error_dialog_with_copy(&window, "Connection Failed", &error_text);
                            button.set_sensitive(true);
                        }
                    }
                }
                PhoneSelectionResult::NoneFound => {
                    status.set_text("No paired phone found");
                    show_pairing_instructions(&window);
                    button.set_sensitive(true);
                }
                PhoneSelectionResult::Cancelled => {
                    status.set_text("Cancelled");
                    button.set_sensitive(true);
                }
                PhoneSelectionResult::Error(e) => {
                    status.set_text("Error");
                    show_error_dialog_with_copy(&window, "Connection Error", &e);
                    button.set_sensitive(true);
                }
            }
        });
    });

    // ========== DEVICE SWITCHER BUTTON HANDLER ==========
    let app_state_device = app_state.clone();
    let status_device = status_label.clone();
    let ui_state_device = ui_state.clone();
    let sync_btn_device = sync_button.clone();
    let send_btn_device = send_button.clone();
    let import_btn_device = import_button.clone();
    let connect_btn_device = connect_button.clone();

    device_switch_button.connect_clicked(move |btn| {
        let state = app_state_device.clone();
        let status = status_device.clone();
        let ui_state_clone = ui_state_device.clone();
        let sync_btn = sync_btn_device.clone();
        let send_btn = send_btn_device.clone();
        let import_btn = import_btn_device.clone();
        let connect_btn = connect_btn_device.clone();
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
                    let ui_state_switch = ui_state_clone.clone();
                    let sync_btn_switch = sync_btn.clone();
                    let send_btn_switch = send_btn.clone();
                    let import_btn_switch = import_btn.clone();
                    let connect_btn_switch = connect_btn.clone();
                    let popover_clone = popover.clone();

                    device_btn.connect_clicked(move |_| {
                        let device = phone.clone();
                        let state_inner = state_switch.clone();
                        let status_inner = status_switch.clone();
                        let ui_state_inner = ui_state_switch.clone();
                        let sync_btn_inner = sync_btn_switch.clone();
                        let send_btn_inner = send_btn_switch.clone();
                        let import_btn_inner = import_btn_switch.clone();
                        let connect_btn_inner = connect_btn_switch.clone();

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

                            match connect_to_device(device, state_inner.clone(), &status_inner).await
                            {
                                ConnectResult::Success { name } => {
                                    sync_btn_inner.set_sensitive(true);
                                    send_btn_inner.set_sensitive(true);
                                    import_btn_inner.set_sensitive(true);

                                    status_inner.set_text("Connecting to ANCS...");
                                    start_ancs_listener(
                                        state_inner.clone(),
                                        ui_state_inner.clone(),
                                        status_inner.clone(),
                                    )
                                    .await;
                                    start_refresh_timer(state_inner.clone(), ui_state_inner.clone());
                                    start_message_poll_timer(state_inner.clone(), ui_state_inner);

                                    status_inner.set_text(&format!("Connected to {}", name));
                                    connect_btn_inner.set_label("Disconnect");
                                }
                                ConnectResult::Failed(e) => {
                                    eprintln!("Failed to switch device: {}", e);
                                    status_inner.set_text("Switch failed");
                                    sync_btn_inner.set_sensitive(false);
                                    send_btn_inner.set_sensitive(false);
                                    import_btn_inner.set_sensitive(false);
                                    connect_btn_inner.set_label("Connect");
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

    // ========== SYNC CONTACTS BUTTON ==========
    let app_state_sync = app_state.clone();
    let status_sync = status_label.clone();
    let window_sync = window.clone();

    sync_button.connect_clicked(move |_| {
        let state = app_state_sync.clone();
        let status = status_sync.clone();
        let window = window_sync.clone();

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
                                    match contact_mgr.sync_from_vcards(&vcards, device_addr).await {
                                        Ok(count) => {
                                            status.set_text(&format!("Synced {} contacts", count));
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
                                show_error_dialog_with_copy(&window, "Error", &format!("{}", e));
                            }
                        }
                        let _ = pbap_client.disconnect().await;
                    }
                    Err(e) => {
                        status.set_text("PBAP failed");
                        show_error_dialog_with_copy(&window, "Error", &format!("{}", e));
                    }
                }
            }
        });
    });

    // ========== IMPORT SMS BUTTON ==========
    let app_state_import = app_state.clone();
    let status_import = status_label.clone();
    let ui_state_import = ui_state.clone();
    let window_import = window.clone();

    import_button.connect_clicked(move |btn| {
        let state = app_state_import.clone();
        let status = status_import.clone();
        let ui_state_clone = ui_state_import.clone();
        let window = window_import.clone();
        let button = btn.clone();

        button.set_sensitive(false);

        glib::spawn_future_local(async move {
            status.set_text("Importing SMS...");
            let state_lock = state.lock().await;

            if let Some(map_client) = &state_lock.map_client {
                let mut imported_count = 0;
                let mut error_messages = Vec::new();

                // Import inbox messages
                status.set_text("Importing inbox...");
                if let Some(pool) = &state_lock.db_pool {
                    match import_inbox_messages(map_client, pool).await {
                        Ok(count) => imported_count += count,
                        Err(e) => error_messages.push(e),
                    }

                    // Import sent messages
                    status.set_text("Importing sent...");
                    imported_count += import_sent_messages(map_client, pool).await;
                }

                drop(state_lock);

                // Refresh conversation list
                refresh_conversations(state.clone(), ui_state_clone).await;

                if error_messages.is_empty() {
                    status.set_text(&format!("Imported {} messages", imported_count));
                } else {
                    status.set_text("Import failed");
                    show_error_dialog_with_copy(&window, "Import Errors", &error_messages.join("\n"));
                }
            } else {
                status.set_text("Not connected");
            }

            button.set_sensitive(true);
        });
    });

    // ========== SEND BUTTON ==========
    let app_state_send = app_state.clone();
    let ui_state_send = ui_state.clone();
    let status_send = status_label.clone();
    let window_send = window.clone();

    let send_handler = {
        let app_state = app_state_send.clone();
        let ui_state = ui_state_send.clone();
        let status = status_send.clone();
        let window = window_send.clone();

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

                            // If this was a new conversation, switch to it
                            let normalized = normalize_e164(&recipient)
                                .unwrap_or_else(|_| recipient.clone());

                            if is_new_conversation {
                                state_lock.current_conversation = Some(normalized.clone());

                                let ui = ui_state_clone.borrow();
                                ui.recipient_entry.set_sensitive(false);
                            }

                            drop(state_lock);
                            refresh_conversations(app_state_clone.clone(), ui_state_clone.clone())
                                .await;

                            // Select the conversation row in the list
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

    // ========== RESET DATABASE BUTTON ==========
    let app_state_reset = app_state.clone();
    let ui_state_reset = ui_state.clone();
    let status_reset = status_label.clone();
    let window_reset = window.clone();

    reset_button.connect_clicked(move |_| {
        let state = app_state_reset.clone();
        let ui_state_clone = ui_state_reset.clone();
        let status = status_reset.clone();
        let window = window_reset.clone();

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
        let ui_state_inner = ui_state_clone.clone();
        let status_inner = status.clone();

        dialog.connect_response(None, move |_, response| {
            if response == "reset" {
                let state = state_clone.clone();
                let ui_state = ui_state_inner.clone();
                let status = status_inner.clone();

                glib::spawn_future_local(async move {
                    let state_lock = state.lock().await;

                    if let Some(pool) = &state_lock.db_pool {
                        status.set_text("Resetting database...");

                        let _ = sqlx::query("DELETE FROM sms_messages")
                            .execute(pool)
                            .await;
                        let _ = sqlx::query("DELETE FROM phone_numbers")
                            .execute(pool)
                            .await;
                        let _ = sqlx::query("DELETE FROM contacts").execute(pool).await;

                        {
                            let ui = ui_state.borrow();
                            while let Some(child) = ui.conversation_list.first_child() {
                                ui.conversation_list.remove(&child);
                            }
                            while let Some(child) = ui.message_list.first_child() {
                                ui.message_list.remove(&child);
                            }
                            ui.recipient_entry.set_text("");
                            ui.message_entry.set_text("");
                        }

                        status.set_text("Database reset complete");
                    }
                });
            }
        });

        dialog.present(Some(&window));
    });

    window.present();
}
