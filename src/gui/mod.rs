use gtk4::prelude::*;
use gtk4::{
    glib, ApplicationWindow, Box as GtkBox, Button, Entry, Label, ListBox,
    ListBoxRow, Orientation, ScrolledWindow, SelectionMode, Paned,
};
use libadwaita::prelude::*;
use libadwaita::{self as adw, HeaderBar};
use std::sync::Arc;
use std::cell::RefCell;
use std::rc::Rc;
use tokio::sync::Mutex;
use btsms::bluetooth::{MapClient, PbapClient, AncsClient, DeviceManager};
use btsms::contacts::ContactManager;
use btsms::db::{self, Conversation};

struct AppState {
    map_client: Option<MapClient>,
    contact_manager: Option<ContactManager>,
    db_pool: Option<sqlx::SqlitePool>,
    device_address: Option<String>,
    current_conversation: Option<String>,
}

impl AppState {
    fn new() -> Self {
        Self {
            map_client: None,
            contact_manager: None,
            db_pool: None,
            device_address: None,
            current_conversation: None,
        }
    }
}

/// Shared UI state that can be accessed from callbacks
struct UiState {
    conversation_list: ListBox,
    message_list: ListBox,
    recipient_entry: Entry,
    message_entry: Entry,
    message_scroll: ScrolledWindow,
}

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

    // Header bar
    let header = HeaderBar::new();
    header.set_title_widget(Some(&Label::new(Some("Messages"))));

    let status_label = Label::new(Some("Disconnected"));
    status_label.add_css_class("dim-label");
    header.pack_end(&status_label);

    let connect_button = Button::with_label("Connect");
    connect_button.add_css_class("suggested-action");
    header.pack_start(&connect_button);

    let sync_button = Button::with_label("Sync Contacts");
    sync_button.set_sensitive(false);
    header.pack_start(&sync_button);

    let import_button = Button::with_label("Import SMS");
    import_button.set_sensitive(false);
    header.pack_start(&import_button);

    main_box.append(&header);

    // Main content: Paned layout with sidebar and chat view
    let paned = Paned::new(Orientation::Horizontal);
    paned.set_position(280);
    paned.set_shrink_start_child(false);
    paned.set_shrink_end_child(false);

    // ========== LEFT SIDEBAR: Conversation List ==========
    let sidebar_box = GtkBox::new(Orientation::Vertical, 0);
    sidebar_box.set_width_request(250);

    // "New Message" button at top of sidebar
    let new_message_btn = Button::with_label("New Message");
    new_message_btn.set_margin_start(8);
    new_message_btn.set_margin_end(8);
    new_message_btn.set_margin_top(8);
    new_message_btn.set_margin_bottom(8);
    sidebar_box.append(&new_message_btn);

    // Conversation list
    let conversation_scroll = ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vexpand(true)
        .build();

    let conversation_list = ListBox::new();
    conversation_list.set_selection_mode(SelectionMode::Single);
    conversation_list.add_css_class("navigation-sidebar");

    conversation_scroll.set_child(Some(&conversation_list));
    sidebar_box.append(&conversation_scroll);

    paned.set_start_child(Some(&sidebar_box));

    // ========== RIGHT SIDE: Chat View ==========
    let chat_box = GtkBox::new(Orientation::Vertical, 0);

    // Recipient bar at top
    let recipient_bar = GtkBox::new(Orientation::Horizontal, 8);
    recipient_bar.set_margin_start(12);
    recipient_bar.set_margin_end(12);
    recipient_bar.set_margin_top(8);
    recipient_bar.set_margin_bottom(8);

    let to_label = Label::new(Some("To:"));
    to_label.add_css_class("dim-label");

    let recipient_entry = Entry::builder()
        .placeholder_text("Phone number or contact name")
        .hexpand(true)
        .build();

    recipient_bar.append(&to_label);
    recipient_bar.append(&recipient_entry);
    chat_box.append(&recipient_bar);

    // Separator
    let separator = gtk4::Separator::new(Orientation::Horizontal);
    chat_box.append(&separator);

    // Message list (scrollable)
    let message_scroll = ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vexpand(true)
        .build();

    let message_list = ListBox::new();
    message_list.set_selection_mode(SelectionMode::None);
    message_list.add_css_class("boxed-list");

    message_scroll.set_child(Some(&message_list));
    chat_box.append(&message_scroll);

    // Compose bar at bottom
    let compose_bar = GtkBox::new(Orientation::Horizontal, 8);
    compose_bar.set_margin_start(12);
    compose_bar.set_margin_end(12);
    compose_bar.set_margin_top(8);
    compose_bar.set_margin_bottom(12);

    let message_entry = Entry::builder()
        .placeholder_text("Message")
        .hexpand(true)
        .build();
    message_entry.add_css_class("message-input");

    let send_button = Button::with_label("Send");
    send_button.add_css_class("suggested-action");
    send_button.set_sensitive(false);

    compose_bar.append(&message_entry);
    compose_bar.append(&send_button);
    chat_box.append(&compose_bar);

    paned.set_end_child(Some(&chat_box));
    main_box.append(&paned);

    window.set_child(Some(&main_box));

    // ========== SHARED STATE ==========
    let app_state = Arc::new(Mutex::new(AppState::new()));

    let ui_state = Rc::new(RefCell::new(UiState {
        conversation_list: conversation_list.clone(),
        message_list: message_list.clone(),
        recipient_entry: recipient_entry.clone(),
        message_entry: message_entry.clone(),
        message_scroll: message_scroll.clone(),
    }));

    // ========== DATABASE INITIALIZATION ==========
    let app_state_init = app_state.clone();
    let status_init = status_label.clone();
    let ui_state_init = ui_state.clone();
    let window_init = window.clone();

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
                let mut state = app_state_init.lock().await;
                state.db_pool = Some(pool.clone());
                state.contact_manager = Some(contact_manager);

                // Check if obexd service is available
                match check_obexd_service().await {
                    Ok(true) => {
                        status_init.set_text("Ready");
                    }
                    Ok(false) | Err(_) => {
                        status_init.set_text("obexd not running");
                    }
                }

                // Load conversations into sidebar
                load_conversations(pool, ui_state_init).await;
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
        // Clear selection in conversation list
        ui.conversation_list.unselect_all();
        // Clear recipient and messages
        ui.recipient_entry.set_text("");
        ui.recipient_entry.set_sensitive(true);
        ui.recipient_entry.grab_focus();
        // Clear message list
        while let Some(child) = ui.message_list.first_child() {
            ui.message_list.remove(&child);
        }
        // Clear current conversation
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

            // Try to get the display name from the row's first label (contact name)
            let display_name = row.child()
                .and_then(|child| child.first_child()) // row_box
                .and_then(|child| child.first_child()) // header_box
                .and_then(|child| child.first_child()) // name_label
                .and_then(|widget| widget.downcast::<Label>().ok())
                .map(|label| label.text().to_string())
                .unwrap_or_else(|| phone.clone());

            let ui = ui_state_select.borrow();
            // Show display name in recipient field, but we'll use phone number for sending
            ui.recipient_entry.set_text(&display_name);
            ui.recipient_entry.set_sensitive(false);

            // Load messages for this conversation
            let app_state_clone = app_state_select.clone();
            let ui_state_clone = ui_state_select.clone();
            let phone_clone = phone.clone();

            glib::spawn_future_local(async move {
                let mut state = app_state_clone.lock().await;
                state.current_conversation = Some(phone_clone.clone());

                if let Some(pool) = &state.db_pool {
                    // Mark conversation as read
                    let _ = db::mark_conversation_read(pool, &phone_clone).await;

                    // Load messages
                    if let Ok(messages) = db::get_messages_for_conversation(pool, &phone_clone, 100).await {
                        let ui = ui_state_clone.borrow();
                        // Clear existing messages
                        while let Some(child) = ui.message_list.first_child() {
                            ui.message_list.remove(&child);
                        }

                        // Add messages (in chronological order)
                        for msg in messages {
                            let is_outgoing = msg.direction == db::MessageDirection::Outgoing;
                            add_message_bubble(&ui.message_list, &msg.body, is_outgoing, &msg.received_at);
                        }

                        // Scroll to bottom
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

    connect_button.connect_clicked(move |btn| {
        let state = app_state_connect.clone();
        let status = status_connect.clone();
        let sync_btn = sync_btn_connect.clone();
        let send_btn = send_btn_connect.clone();
        let import_btn = import_btn_connect.clone();
        let ui_state_clone = ui_state_connect.clone();
        let button = btn.clone();
        let window = window_connect.clone();

        button.set_sensitive(false);
        status.set_text("Connecting...");

        glib::spawn_future_local(async move {
            let selection_result = select_paired_device(&window).await;

            match selection_result {
                PhoneSelectionResult::Selected(device) => {
                    let addr = device.address;
                    let device_name = device.name;
                    let mut state_lock = state.lock().await;

                    status.set_text("Connecting to MAP...");
                    let mut map_client = MapClient::new(addr.clone());
                    match map_client.connect().await {
                        Ok(_) => {
                            eprintln!("MAP connection successful");
                            state_lock.map_client = Some(map_client);
                            state_lock.device_address = Some(addr.clone());

                            sync_btn.set_sensitive(true);
                            send_btn.set_sensitive(true);
                            import_btn.set_sensitive(true);

                            // Start ANCS listener and auto-refresh
                            status.set_text("Connecting to ANCS...");
                            start_ancs_listener(state.clone(), ui_state_clone.clone(), status.clone()).await;
                            start_refresh_timer(state.clone(), ui_state_clone.clone());

                            status.set_text(&format!("Connected to {}", device_name));
                            button.set_label("Disconnect");
                            button.set_sensitive(true);
                        }
                        Err(e) => {
                            let error_msg = format!("{}", e);
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
                                            show_error_dialog_with_copy(&window, "Sync Error", &format!("{}", e));
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
                match map_client.list_inbox_messages().await {
                    Ok(messages) => {
                        for msg in &messages {
                            if let Some(pool) = &state_lock.db_pool {
                                // Try to get full message content
                                let body = match map_client.get_message_content(&msg.handle).await {
                                    Ok(content) => content,
                                    Err(_) => msg.subject.clone(),
                                };

                                if !body.is_empty() {
                                    let message_uid = format!("map_{}_{}", msg.handle, msg.timestamp);
                                    let timestamp = parse_map_timestamp(&msg.timestamp);

                                    // Insert if not exists
                                    let result = sqlx::query(
                                        "INSERT OR IGNORE INTO sms_messages
                                         (message_uid, device_source, sender_number, sender_normalized, message_body,
                                          direction, received_at, message_type, read_status)
                                         VALUES (?, 'phone', ?, ?, ?, 'INCOMING', ?, 'SMS', ?)"
                                    )
                                    .bind(&message_uid)
                                    .bind(&msg.sender)
                                    .bind(&msg.sender)
                                    .bind(&body)
                                    .bind(&timestamp)
                                    .bind(msg.read)
                                    .execute(pool)
                                    .await;

                                    if result.is_ok() {
                                        imported_count += 1;
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        error_messages.push(format!("Inbox: {}", e));
                    }
                }

                // Import sent messages
                status.set_text("Importing sent...");
                match map_client.list_sent_messages().await {
                    Ok(messages) => {
                        for msg in &messages {
                            if let Some(pool) = &state_lock.db_pool {
                                let body = match map_client.get_message_content(&msg.handle).await {
                                    Ok(content) => content,
                                    Err(_) => msg.subject.clone(),
                                };

                                if !body.is_empty() {
                                    let message_uid = format!("map_{}_{}", msg.handle, msg.timestamp);
                                    let timestamp = parse_map_timestamp(&msg.timestamp);
                                    let recipient = msg.recipient.as_deref().unwrap_or("");

                                    let result = sqlx::query(
                                        "INSERT OR IGNORE INTO sms_messages
                                         (message_uid, device_source, sender_normalized, recipient_number, recipient_normalized,
                                          message_body, direction, received_at, message_type, read_status)
                                         VALUES (?, 'phone', 'me', ?, ?, ?, 'OUTGOING', ?, 'SMS', 1)"
                                    )
                                    .bind(&message_uid)
                                    .bind(recipient)
                                    .bind(recipient)
                                    .bind(&body)
                                    .bind(&timestamp)
                                    .execute(pool)
                                    .await;

                                    if result.is_ok() {
                                        imported_count += 1;
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        error_messages.push(format!("Sent: {}", e));
                    }
                }

                drop(state_lock);

                // Refresh conversation list
                refresh_conversations(state.clone(), ui_state_clone).await;

                if error_messages.is_empty() {
                    status.set_text(&format!("Imported {} messages", imported_count));
                } else {
                    status.set_text(&format!("Imported {} (with errors)", imported_count));
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
                let state_lock = app_state_clone.lock().await;

                // Use current_conversation phone number if available, otherwise use recipient_entry text
                let recipient = state_lock.current_conversation.clone()
                    .unwrap_or(recipient_text);

                if let Some(map_client) = &state_lock.map_client {
                    status_clone.set_text("Sending...");

                    match map_client.send_sms(&recipient, &message).await {
                        Ok(_) => {
                            status_clone.set_text("Sent");

                            // Add message to chat
                            {
                                let ui = ui_state_clone.borrow();
                                add_message_bubble(&ui.message_list, &message, true, &chrono::Local::now().format("%H:%M").to_string());
                                scroll_to_bottom(&ui.message_scroll);
                                ui.message_entry.set_text("");
                            }

                            // Save to database
                            if let Some(pool) = &state_lock.db_pool {
                                save_message_to_db(pool, &recipient, &message, "OUTGOING").await;
                            }

                            // Refresh conversation list
                            drop(state_lock);
                            refresh_conversations(app_state_clone.clone(), ui_state_clone.clone()).await;
                        }
                        Err(e) => {
                            status_clone.set_text("Send failed");
                            show_error_dialog_with_copy(&window_clone, "Send Error", &format!("{}", e));
                        }
                    }
                }
            });
        }
    };

    // Send button click
    let send_handler_click = send_handler.clone();
    send_button.connect_clicked(move |_| {
        send_handler_click();
    });

    // Enter key to send
    let send_handler_enter = send_handler;
    message_entry.connect_activate(move |_| {
        send_handler_enter();
    });

    window.present();
}

// ========== HELPER FUNCTIONS ==========

fn add_message_bubble(list_box: &ListBox, message: &str, is_outgoing: bool, time: &str) {
    let row = ListBoxRow::new();
    row.set_selectable(false);

    let outer_box = GtkBox::new(Orientation::Horizontal, 0);
    outer_box.set_margin_start(12);
    outer_box.set_margin_end(12);
    outer_box.set_margin_top(4);
    outer_box.set_margin_bottom(4);

    let bubble_box = GtkBox::new(Orientation::Vertical, 2);
    bubble_box.set_margin_start(8);
    bubble_box.set_margin_end(8);
    bubble_box.set_margin_top(6);
    bubble_box.set_margin_bottom(6);

    let message_label = Label::new(Some(message));
    message_label.set_wrap(true);
    message_label.set_xalign(0.0);
    message_label.set_max_width_chars(40);

    let time_label = Label::new(Some(time));
    time_label.add_css_class("dim-label");
    time_label.add_css_class("caption");

    bubble_box.append(&message_label);
    bubble_box.append(&time_label);

    if is_outgoing {
        // Outgoing: align right with blue background
        outer_box.set_halign(gtk4::Align::End);
        bubble_box.add_css_class("card");
        bubble_box.add_css_class("outgoing-bubble");
        time_label.set_halign(gtk4::Align::End);
    } else {
        // Incoming: align left with gray background
        outer_box.set_halign(gtk4::Align::Start);
        bubble_box.add_css_class("card");
        bubble_box.add_css_class("incoming-bubble");
        time_label.set_halign(gtk4::Align::Start);
    }

    outer_box.append(&bubble_box);
    row.set_child(Some(&outer_box));
    list_box.append(&row);
}

fn add_conversation_row(list_box: &ListBox, conversation: &Conversation) {
    let row = ListBoxRow::new();
    row.set_widget_name(&conversation.phone_number);

    let row_box = GtkBox::new(Orientation::Vertical, 4);
    row_box.set_margin_start(12);
    row_box.set_margin_end(12);
    row_box.set_margin_top(8);
    row_box.set_margin_bottom(8);

    let header_box = GtkBox::new(Orientation::Horizontal, 8);

    // Contact name or phone number
    let name = conversation.display_name.as_deref().unwrap_or(&conversation.phone_number);
    let name_label = Label::new(Some(name));
    name_label.set_halign(gtk4::Align::Start);
    name_label.set_hexpand(true);
    name_label.add_css_class("heading");
    name_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);

    // Time of last message
    let time_str = format_relative_time(&conversation.last_message_time);
    let time_label = Label::new(Some(&time_str));
    time_label.add_css_class("dim-label");
    time_label.add_css_class("caption");

    header_box.append(&name_label);
    header_box.append(&time_label);

    // Message preview
    let preview = truncate_message(&conversation.last_message, 50);
    let preview_label = Label::new(Some(&preview));
    preview_label.set_halign(gtk4::Align::Start);
    preview_label.add_css_class("dim-label");
    preview_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);

    row_box.append(&header_box);
    row_box.append(&preview_label);

    // Unread badge
    if conversation.unread_count > 0 {
        name_label.add_css_class("bold");
        preview_label.remove_css_class("dim-label");
    }

    row.set_child(Some(&row_box));
    list_box.append(&row);
}

fn truncate_message(message: &str, max_len: usize) -> String {
    let cleaned = message.replace('\n', " ");
    if cleaned.len() > max_len {
        format!("{}...", &cleaned[..max_len])
    } else {
        cleaned
    }
}

fn format_relative_time(timestamp: &str) -> String {
    // Try to parse the timestamp and format relative to now
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(timestamp) {
        let now = chrono::Local::now();
        let msg_time = dt.with_timezone(&chrono::Local);
        let duration = now.signed_duration_since(msg_time);

        if duration.num_hours() < 24 {
            msg_time.format("%H:%M").to_string()
        } else if duration.num_days() < 7 {
            msg_time.format("%a").to_string()
        } else {
            msg_time.format("%m/%d").to_string()
        }
    } else {
        // Fallback: just show the raw timestamp
        timestamp.to_string()
    }
}

/// Parse MAP timestamp format (e.g., "20240115T143022" or "20240115T143022+0100") to RFC3339
fn parse_map_timestamp(timestamp: &str) -> String {
    // MAP timestamps are typically in format: YYYYMMDDTHHmmss or YYYYMMDDTHHmmss+ZZZZ
    if timestamp.len() >= 15 {
        let year = &timestamp[0..4];
        let month = &timestamp[4..6];
        let day = &timestamp[6..8];
        let hour = &timestamp[9..11];
        let minute = &timestamp[11..13];
        let second = &timestamp[13..15];

        // Try to parse timezone if present
        let tz = if timestamp.len() > 15 {
            let tz_part = &timestamp[15..];
            if tz_part.starts_with('+') || tz_part.starts_with('-') {
                // Format: +0100 -> +01:00
                if tz_part.len() >= 5 {
                    format!("{}:{}", &tz_part[..3], &tz_part[3..5])
                } else {
                    "+00:00".to_string()
                }
            } else {
                "+00:00".to_string()
            }
        } else {
            "+00:00".to_string()
        };

        format!("{}-{}-{}T{}:{}:{}{}", year, month, day, hour, minute, second, tz)
    } else if timestamp.is_empty() {
        chrono::Utc::now().to_rfc3339()
    } else {
        // Return as-is if can't parse
        timestamp.to_string()
    }
}

fn scroll_to_bottom(scroll: &ScrolledWindow) {
    let adj = scroll.vadjustment();
    glib::idle_add_local_once(move || {
        adj.set_value(adj.upper() - adj.page_size());
    });
}

async fn load_conversations(pool: sqlx::SqlitePool, ui_state: Rc<RefCell<UiState>>) {
    match db::get_conversations(&pool).await {
        Ok(conversations) => {
            glib::idle_add_local_once(move || {
                let ui = ui_state.borrow();
                // Clear existing
                while let Some(child) = ui.conversation_list.first_child() {
                    ui.conversation_list.remove(&child);
                }
                // Add conversations
                for conv in conversations {
                    add_conversation_row(&ui.conversation_list, &conv);
                }
            });
        }
        Err(e) => {
            eprintln!("Failed to load conversations: {}", e);
        }
    }
}

async fn refresh_conversations(app_state: Arc<Mutex<AppState>>, ui_state: Rc<RefCell<UiState>>) {
    let state = app_state.lock().await;
    if let Some(pool) = &state.db_pool {
        let pool_clone = pool.clone();
        drop(state);
        load_conversations(pool_clone, ui_state).await;
    }
}

fn start_refresh_timer(app_state: Arc<Mutex<AppState>>, ui_state: Rc<RefCell<UiState>>) {
    glib::timeout_add_seconds_local(30, move || {
        let app_state_clone = app_state.clone();
        let ui_state_clone = ui_state.clone();

        glib::spawn_future_local(async move {
            refresh_conversations(app_state_clone, ui_state_clone).await;
        });

        glib::ControlFlow::Continue
    });
}

async fn save_message_to_db(pool: &sqlx::SqlitePool, recipient: &str, message: &str, direction: &str) {
    let message_uid = format!("{}_{}", chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0), recipient);
    let now = chrono::Utc::now().to_rfc3339();

    let _ = sqlx::query(
        "INSERT INTO sms_messages (message_uid, device_source, sender_normalized, recipient_normalized, message_body, direction, received_at, message_type)
         VALUES (?, 'local', 'me', ?, ?, ?, ?, 'SMS')"
    )
    .bind(&message_uid)
    .bind(recipient)
    .bind(message)
    .bind(direction)
    .bind(&now)
    .execute(pool)
    .await;
}

// ========== DEVICE SELECTION ==========

use btsms::bluetooth::BluetoothDevice;

enum PhoneSelectionResult {
    Selected(BluetoothDevice),
    NoneFound,
    Cancelled,
    Error(String),
}

async fn select_paired_device(window: &ApplicationWindow) -> PhoneSelectionResult {
    let manager = match DeviceManager::new().await {
        Ok(m) => m,
        Err(e) => return PhoneSelectionResult::Error(format!("Device manager error: {}", e)),
    };

    let phones = match manager.get_all_paired_phones().await {
        Ok(p) => p,
        Err(e) => return PhoneSelectionResult::Error(format!("Failed to get devices: {}", e)),
    };

    if phones.is_empty() {
        return PhoneSelectionResult::NoneFound;
    }

    if phones.len() == 1 {
        let device = phones.into_iter().next().unwrap();
        return connect_and_return_device(&manager, device).await;
    }

    show_phone_selection_dialog(window, phones, manager).await
}

async fn connect_and_return_device(manager: &DeviceManager, device: BluetoothDevice) -> PhoneSelectionResult {
    if !device.connected {
        if let Err(e) = manager.connect_device(&device.address).await {
            return PhoneSelectionResult::Error(format!("Failed to connect: {}", e));
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    }
    PhoneSelectionResult::Selected(device)
}

async fn show_phone_selection_dialog(
    window: &ApplicationWindow,
    phones: Vec<BluetoothDevice>,
    manager: DeviceManager,
) -> PhoneSelectionResult {
    let (tx, rx) = tokio::sync::oneshot::channel::<Option<BluetoothDevice>>();
    let tx = Rc::new(RefCell::new(Some(tx)));

    let dialog = adw::Window::builder()
        .transient_for(window)
        .modal(true)
        .default_width(400)
        .default_height(300)
        .title("Select Phone")
        .build();

    let toolbar_view = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    toolbar_view.add_top_bar(&header);

    let content_box = GtkBox::new(Orientation::Vertical, 12);
    content_box.set_margin_start(12);
    content_box.set_margin_end(12);
    content_box.set_margin_top(12);
    content_box.set_margin_bottom(12);

    let label = Label::new(Some("Select a phone to connect:"));
    label.set_halign(gtk4::Align::Start);
    content_box.append(&label);

    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vexpand(true)
        .build();

    let list_box = ListBox::new();
    list_box.set_selection_mode(SelectionMode::Single);
    list_box.add_css_class("boxed-list");

    let phones_rc = Rc::new(phones);

    for (idx, phone) in phones_rc.iter().enumerate() {
        let row = ListBoxRow::new();
        let row_box = GtkBox::new(Orientation::Vertical, 4);
        row_box.set_margin_start(12);
        row_box.set_margin_end(12);
        row_box.set_margin_top(8);
        row_box.set_margin_bottom(8);

        let name_label = Label::new(Some(&phone.name));
        name_label.set_halign(gtk4::Align::Start);
        name_label.add_css_class("heading");

        let status = if phone.connected { "Connected" } else { "Not connected" };
        let detail_label = Label::new(Some(&format!("{} - {}", phone.address, status)));
        detail_label.set_halign(gtk4::Align::Start);
        detail_label.add_css_class("dim-label");

        row_box.append(&name_label);
        row_box.append(&detail_label);
        row.set_child(Some(&row_box));
        row.set_widget_name(&idx.to_string());

        list_box.append(&row);
    }

    if let Some(first_row) = list_box.row_at_index(0) {
        list_box.select_row(Some(&first_row));
    }

    scrolled.set_child(Some(&list_box));
    content_box.append(&scrolled);

    let button_box = GtkBox::new(Orientation::Horizontal, 6);
    button_box.set_halign(gtk4::Align::End);

    let cancel_btn = Button::with_label("Cancel");
    let select_btn = Button::with_label("Connect");
    select_btn.add_css_class("suggested-action");

    button_box.append(&cancel_btn);
    button_box.append(&select_btn);
    content_box.append(&button_box);

    toolbar_view.set_content(Some(&content_box));
    dialog.set_content(Some(&toolbar_view));

    let tx_cancel = tx.clone();
    let dialog_cancel = dialog.clone();
    cancel_btn.connect_clicked(move |_| {
        if let Some(sender) = tx_cancel.borrow_mut().take() {
            let _ = sender.send(None);
        }
        dialog_cancel.close();
    });

    let tx_select = tx.clone();
    let dialog_select = dialog.clone();
    let phones_select = phones_rc.clone();
    select_btn.connect_clicked(move |_| {
        let selected = list_box.selected_row().and_then(|row| {
            row.widget_name()
                .parse::<usize>()
                .ok()
                .and_then(|idx| phones_select.get(idx).cloned())
        });

        if let Some(sender) = tx_select.borrow_mut().take() {
            let _ = sender.send(selected);
        }
        dialog_select.close();
    });

    let tx_close = tx;
    dialog.connect_close_request(move |_| {
        if let Some(sender) = tx_close.borrow_mut().take() {
            let _ = sender.send(None);
        }
        glib::Propagation::Proceed
    });

    dialog.present();

    match rx.await {
        Ok(Some(device)) => connect_and_return_device(&manager, device).await,
        Ok(None) | Err(_) => PhoneSelectionResult::Cancelled,
    }
}

// ========== ANCS LISTENER ==========

async fn start_ancs_listener(
    app_state: Arc<Mutex<AppState>>,
    ui_state: Rc<RefCell<UiState>>,
    status_label: Label,
) {
    let app_state_clone = app_state.clone();

    glib::MainContext::default().spawn_local(async move {
        let mut ancs_client = AncsClient::new();

        match ancs_client.connect().await {
            Ok(_) => {
                eprintln!("ANCS connected - listening for notifications");
                status_label.set_text("Connected (ANCS active)");

                if let Some(mut rx) = ancs_client.take_notification_receiver() {
                    while let Some(notification) = rx.recv().await {
                        if let (Some(title), Some(message)) = (&notification.title, &notification.message) {
                            let sender = title.clone();
                            let msg = message.clone();

                            eprintln!("Received SMS: {} - {}", sender, msg);

                            // Add to UI if it's the current conversation
                            {
                                let state = app_state_clone.lock().await;
                                if state.current_conversation.as_deref() == Some(&sender) {
                                    let ui = ui_state.borrow();
                                    add_message_bubble(&ui.message_list, &msg, false, &chrono::Local::now().format("%H:%M").to_string());
                                    scroll_to_bottom(&ui.message_scroll);
                                }

                                // Save to database
                                if let Some(pool) = &state.db_pool {
                                    let message_uid = format!("{}_{}", chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0), sender);
                                    let now = chrono::Utc::now().to_rfc3339();

                                    let _ = sqlx::query(
                                        "INSERT INTO sms_messages (message_uid, device_source, sender_normalized, message_body, direction, received_at, message_type)
                                         VALUES (?, 'iphone', ?, ?, 'INCOMING', ?, 'SMS')"
                                    )
                                    .bind(&message_uid)
                                    .bind(&sender)
                                    .bind(&msg)
                                    .bind(&now)
                                    .execute(pool)
                                    .await;
                                }
                            }

                            // Refresh conversation list
                            refresh_conversations(app_state_clone.clone(), ui_state.clone()).await;
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("ANCS not available (normal for Android): {}", e);
                status_label.set_text("Connected (MAP only)");
            }
        }
    });
}

// ========== ERROR DIALOGS ==========

fn show_pairing_instructions(window: &ApplicationWindow) {
    #[allow(deprecated)]
    {
        let dialog = gtk4::MessageDialog::builder()
            .transient_for(window)
            .modal(true)
            .message_type(gtk4::MessageType::Info)
            .buttons(gtk4::ButtonsType::Ok)
            .text("No Paired Phone Found")
            .secondary_text(
                "To pair your phone:\n\n\
                1. Open terminal: bluetoothctl\n\
                2. Type: scan on\n\
                3. Type: pair [MAC_ADDRESS]\n\
                4. Type: trust [MAC_ADDRESS]\n\n\
                For iPhone: Enable 'Show Notifications' in Bluetooth settings"
            )
            .build();
        dialog.present();
    }
}

fn show_error_dialog_with_copy(window: &ApplicationWindow, title: &str, message: &str) {
    let dialog = adw::Window::builder()
        .transient_for(window)
        .modal(true)
        .default_width(500)
        .default_height(300)
        .title(title)
        .build();

    let toolbar_view = adw::ToolbarView::new();
    let header = adw::HeaderBar::new();
    toolbar_view.add_top_bar(&header);

    let content_box = GtkBox::new(Orientation::Vertical, 12);
    content_box.set_margin_start(12);
    content_box.set_margin_end(12);
    content_box.set_margin_top(12);
    content_box.set_margin_bottom(12);

    let scroll = ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vexpand(true)
        .build();

    let text_view = gtk4::TextView::builder()
        .editable(false)
        .wrap_mode(gtk4::WrapMode::WordChar)
        .margin_start(12)
        .margin_end(12)
        .margin_top(12)
        .margin_bottom(12)
        .build();

    text_view.buffer().set_text(message);
    scroll.set_child(Some(&text_view));
    content_box.append(&scroll);

    let button_box = GtkBox::new(Orientation::Horizontal, 6);
    button_box.set_halign(gtk4::Align::End);

    let copy_btn = Button::with_label("Copy");
    let ok_btn = Button::with_label("OK");
    ok_btn.add_css_class("suggested-action");

    button_box.append(&copy_btn);
    button_box.append(&ok_btn);
    content_box.append(&button_box);

    toolbar_view.set_content(Some(&content_box));
    dialog.set_content(Some(&toolbar_view));

    let message_clone = message.to_string();
    copy_btn.connect_clicked(move |_| {
        if let Some(display) = gtk4::gdk::Display::default() {
            let clipboard = display.clipboard();
            clipboard.set_text(&message_clone);
        }
    });

    let dialog_clone = dialog.clone();
    ok_btn.connect_clicked(move |_| {
        dialog_clone.close();
    });

    dialog.present();
}

async fn check_obexd_service() -> Result<bool, Box<dyn std::error::Error>> {
    let connection = zbus::Connection::session().await?;
    let proxy = zbus::fdo::DBusProxy::new(&connection).await?;
    let names = proxy.list_names().await?;
    Ok(names.iter().any(|name| name.as_str() == "org.bluez.obex"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_message_short() {
        let msg = "Hello";
        assert_eq!(truncate_message(msg, 50), "Hello");
    }

    #[test]
    fn test_truncate_message_long() {
        let msg = "This is a very long message that should be truncated because it exceeds the maximum length";
        let result = truncate_message(msg, 20);
        assert!(result.ends_with("..."));
        assert!(result.len() <= 23); // 20 + "..."
    }

    #[test]
    fn test_truncate_message_newlines() {
        let msg = "Line 1\nLine 2\nLine 3";
        let result = truncate_message(msg, 50);
        assert!(!result.contains('\n'));
        assert_eq!(result, "Line 1 Line 2 Line 3");
    }

    #[test]
    fn test_format_relative_time_today() {
        let now = chrono::Utc::now();
        let timestamp = now.to_rfc3339();
        let result = format_relative_time(&timestamp);
        // Should be HH:MM format
        assert!(result.contains(':'));
    }

    #[test]
    fn test_format_relative_time_invalid() {
        let result = format_relative_time("invalid timestamp");
        assert_eq!(result, "invalid timestamp");
    }

    #[test]
    fn test_parse_map_timestamp_basic() {
        let result = parse_map_timestamp("20240115T143022");
        assert_eq!(result, "2024-01-15T14:30:22+00:00");
    }

    #[test]
    fn test_parse_map_timestamp_with_timezone() {
        let result = parse_map_timestamp("20240115T143022+0100");
        assert_eq!(result, "2024-01-15T14:30:22+01:00");
    }

    #[test]
    fn test_parse_map_timestamp_empty() {
        let result = parse_map_timestamp("");
        // Should return current time in RFC3339 format
        assert!(result.contains('T'));
        assert!(result.contains('-'));
    }

    #[test]
    fn test_parse_map_timestamp_invalid() {
        let result = parse_map_timestamp("invalid");
        assert_eq!(result, "invalid");
    }
}
