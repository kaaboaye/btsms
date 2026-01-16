use gtk4::prelude::*;
use gtk4::{
    glib, ApplicationWindow, Box as GtkBox, Button, Entry, Label, ListBox,
    ListBoxRow, Orientation, ScrolledWindow, SelectionMode,
};
use libadwaita::{self as adw, HeaderBar};
use std::sync::{Arc};
use tokio::sync::Mutex;
use crate::bluetooth::{MapClient, PbapClient, AncsClient, DeviceManager};
use crate::contacts::ContactManager;
use crate::db;
use sqlx::Row;

struct AppState {
    map_client: Option<MapClient>,
    pbap_client: Option<PbapClient>,
    ancs_client: Option<AncsClient>,
    contact_manager: Option<ContactManager>,
    db_pool: Option<sqlx::SqlitePool>,
    device_address: Option<String>,
}

impl AppState {
    fn new() -> Self {
        Self {
            map_client: None,
            pbap_client: None,
            ancs_client: None,
            contact_manager: None,
            db_pool: None,
            device_address: None,
        }
    }
}

pub fn build_ui(app: &adw::Application) {
    // Create main window
    let window = ApplicationWindow::builder()
        .application(app)
        .title("Bluetooth SMS")
        .default_width(900)
        .default_height(700)
        .build();

    // Main container
    let main_box = GtkBox::new(Orientation::Vertical, 0);

    // Header bar
    let header = HeaderBar::new();
    header.set_title_widget(Some(&Label::new(Some("Bluetooth SMS"))));

    let status_label = Label::new(Some("Disconnected"));
    status_label.add_css_class("dim-label");
    header.pack_end(&status_label);

    let connect_button = Button::with_label("Connect to Phone");
    connect_button.add_css_class("suggested-action");
    header.pack_start(&connect_button);

    let sync_button = Button::with_label("Sync Contacts");
    sync_button.set_sensitive(false);
    header.pack_start(&sync_button);

    main_box.append(&header);

    // Content area
    let content_box = GtkBox::new(Orientation::Vertical, 12);
    content_box.set_margin_start(12);
    content_box.set_margin_end(12);
    content_box.set_margin_top(12);
    content_box.set_margin_bottom(12);

    // Message list
    let list_label = Label::new(Some("Messages"));
    list_label.set_halign(gtk4::Align::Start);
    list_label.add_css_class("title-2");
    content_box.append(&list_label);

    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .min_content_height(450)
        .vexpand(true)
        .build();

    let list_box = ListBox::new();
    list_box.set_selection_mode(SelectionMode::None);
    list_box.add_css_class("boxed-list");

    scrolled.set_child(Some(&list_box));
    content_box.append(&scrolled);

    // Compose area
    let compose_label = Label::new(Some("Send Message"));
    compose_label.set_halign(gtk4::Align::Start);
    compose_label.add_css_class("title-3");
    content_box.append(&compose_label);

    let compose_box = GtkBox::new(Orientation::Horizontal, 6);

    let recipient_entry = Entry::builder()
        .placeholder_text("Phone number or contact name")
        .width_request(200)
        .build();

    let message_entry = Entry::builder()
        .placeholder_text("Type a message...")
        .hexpand(true)
        .build();

    let send_button = Button::with_label("Send");
    send_button.add_css_class("suggested-action");
    send_button.set_sensitive(false);

    compose_box.append(&recipient_entry);
    compose_box.append(&message_entry);
    compose_box.append(&send_button);

    content_box.append(&compose_box);
    main_box.append(&content_box);

    window.set_child(Some(&main_box));

    // Shared application state
    let app_state = Arc::new(Mutex::new(AppState::new()));

    // Initialize database
    let app_state_init = app_state.clone();
    let status_init = status_label.clone();
    let list_init = list_box.clone();

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
                        status_init.set_text("Ready - Click 'Connect to Phone' to begin");
                    }
                    Ok(false) | Err(_) => {
                        status_init.set_text("⚠️  Warning: obexd service not running - run: systemctl --user start obex");
                    }
                }

                // Load existing messages
                load_messages_from_db(pool, list_init).await;
            }
            Err(e) => {
                eprintln!("Failed to initialize database: {}", e);
            }
        }
    });

    // Connect button handler
    let app_state_connect = app_state.clone();
    let status_connect = status_label.clone();
    let sync_btn_connect = sync_button.clone();
    let send_btn_connect = send_button.clone();
    let list_connect = list_box.clone();
    let window_connect = window.clone();

    connect_button.connect_clicked(move |btn| {
        let state = app_state_connect.clone();
        let status = status_connect.clone();
        let sync_btn = sync_btn_connect.clone();
        let send_btn = send_btn_connect.clone();
        let list_box_clone = list_connect.clone();
        let button = btn.clone();
        let window = window_connect.clone();

        button.set_sensitive(false);
        status.set_text("Connecting to phone...");

        glib::spawn_future_local(async move {
            // Get device address (in real implementation, you'd scan for devices)
            // For now, we'll use a placeholder
            let device_address = get_paired_device_address().await;

            match device_address {
                Some(addr) => {
                    let mut state_lock = state.lock().await;

                    // Connect MAP client (for SMS send/receive)
                    status.set_text("Connecting to MAP (SMS)...");
                    let mut map_client = MapClient::new(addr.clone());
                    match map_client.connect().await {
                        Ok(_) => {
                            eprintln!("MAP connection successful");
                            state_lock.map_client = Some(map_client);
                            state_lock.device_address = Some(addr.clone());

                            // Enable sync and send buttons
                            sync_btn.set_sensitive(true);
                            send_btn.set_sensitive(true);

                            // Start ANCS client for notifications (iPhone only)
                            status.set_text("Connecting to ANCS (notifications)...");
                            start_ancs_listener(state.clone(), list_box_clone, status.clone()).await;

                            status.set_text(&format!("✓ Connected to {}", addr));
                            button.set_label("Disconnect");
                            button.set_sensitive(true);
                        }
                        Err(e) => {
                            let error_msg = format!("{}", e);
                            eprintln!("MAP connection failed: {}", error_msg);
                            status.set_text("Failed to connect to MAP service");

                            // Show helpful error dialog
                            let error_text = format!(
                                "Failed to connect to MAP (Message Access Profile):\n\n{}\n\n\
                                ⚠️  MOST LIKELY FIX: Start the obexd service\n\
                                Run this command in terminal:\n\
                                    systemctl --user start obex\n\n\
                                Or if that doesn't work, try:\n\
                                    /usr/lib/bluetooth/obexd &\n\n\
                                FOR IPHONE: Make sure 'Show Notifications' is enabled in:\n\
                                Settings → Bluetooth → [Computer Name] → (i) → Show Notifications\n\n\
                                FOR ANDROID: Make sure Bluetooth is enabled and the device is unlocked.\n\n\
                                Other possible issues:\n\
                                • Phone not properly paired or trusted\n\
                                • Phone locked or Bluetooth disabled",
                                error_msg
                            );

                            #[allow(deprecated)]
                            {
                                let dialog = gtk4::MessageDialog::builder()
                                    .transient_for(&window)
                                    .modal(true)
                                    .message_type(gtk4::MessageType::Error)
                                    .buttons(gtk4::ButtonsType::Ok)
                                    .text("Connection Failed")
                                    .secondary_text(&error_text)
                                    .build();
                                dialog.present();
                            }

                            button.set_sensitive(true);
                        }
                    }
                }
                None => {
                    status.set_text("No paired phone found");

                    // Show helpful dialog with pairing instructions
                    let instructions = "To use this app with your iPhone or Android phone:\n\n\
                        1. Open a terminal and run: bluetoothctl\n\
                        2. Type: scan on\n\
                        3. Wait for your phone to appear\n\
                        4. Type: pair [MAC_ADDRESS]\n\
                        5. Type: trust [MAC_ADDRESS]\n\
                        6. Type: exit\n\n\
                        FOR IPHONE USERS:\n\
                        After pairing, go to iPhone Settings → Bluetooth → \
                        tap (i) next to your computer name → \
                        enable 'Show Notifications'\n\n\
                        Then click 'Connect to Phone' again.";

                    #[allow(deprecated)]
                    {
                        let dialog = gtk4::MessageDialog::builder()
                            .transient_for(&window)
                            .modal(true)
                            .message_type(gtk4::MessageType::Info)
                            .buttons(gtk4::ButtonsType::Ok)
                            .text("No Paired Phone Found")
                            .secondary_text(instructions)
                            .build();
                        dialog.present();
                    }
                    button.set_sensitive(true);
                }
            }
        });
    });

    // Sync contacts button handler
    let app_state_sync = app_state.clone();
    let status_sync = status_label.clone();

    sync_button.connect_clicked(move |_| {
        let state = app_state_sync.clone();
        let status = status_sync.clone();

        glib::spawn_future_local(async move {
            status.set_text("Syncing contacts...");

            let mut state_lock = state.lock().await;

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
                                            status.set_text(&format!("Failed to sync contacts: {}", e));
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                status.set_text(&format!("Failed to pull contacts: {}", e));
                            }
                        }

                        let _ = pbap_client.disconnect().await;
                    }
                    Err(e) => {
                        status.set_text(&format!("Failed to connect PBAP: {}", e));
                    }
                }
            }
        });
    });

    // Send button handler
    let app_state_send = app_state.clone();
    let recipient_send = recipient_entry.clone();
    let message_send = message_entry.clone();
    let status_send = status_label.clone();
    let list_send = list_box.clone();

    send_button.connect_clicked(move |_| {
        let recipient = recipient_send.text().to_string();
        let message = message_send.text().to_string();

        if recipient.is_empty() || message.is_empty() {
            return;
        }

        let state = app_state_send.clone();
        let status = status_send.clone();
        let list_box_clone = list_send.clone();
        let msg_entry_clone = message_send.clone();

        glib::spawn_future_local(async move {
            let state_lock = state.lock().await;

            if let Some(map_client) = &state_lock.map_client {
                status.set_text("Sending message...");

                match map_client.send_sms(&recipient, &message).await {
                    Ok(_) => {
                        status.set_text("Message sent!");

                        // Add to message list
                        add_message_to_list(&list_box_clone, &recipient, &message, "You", true);

                        // Save to database
                        if let Some(pool) = &state_lock.db_pool {
                            save_message_to_db(pool, &recipient, &message, "OUTGOING").await;
                        }

                        // Clear message entry
                        msg_entry_clone.set_text("");
                    }
                    Err(e) => {
                        status.set_text(&format!("Failed to send: {}", e));
                    }
                }
            }
        });
    });

    window.present();
}

async fn get_paired_device_address() -> Option<String> {
    match DeviceManager::new().await {
        Ok(manager) => {
            match manager.get_first_paired_phone().await {
                Ok(Some(device)) => {
                    eprintln!("Found paired device: {} ({})", device.name, device.address);

                    // Ensure device is connected
                    if !device.connected {
                        eprintln!("Device not connected, attempting to connect...");
                        if let Err(e) = manager.connect_device(&device.address).await {
                            eprintln!("Failed to connect to device: {}", e);
                            return None;
                        }
                        // Give it a moment to establish connection
                        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                    }

                    Some(device.address)
                }
                Ok(None) => {
                    eprintln!("No paired phones found. Please pair your phone using bluetoothctl");
                    None
                }
                Err(e) => {
                    eprintln!("Failed to get paired devices: {}", e);
                    None
                }
            }
        }
        Err(e) => {
            eprintln!("Failed to create device manager: {}", e);
            None
        }
    }
}

async fn start_ancs_listener(
    app_state: Arc<Mutex<AppState>>,
    list_box: ListBox,
    status_label: Label,
) {
    // Clone what we need for the async task
    let app_state_clone = app_state.clone();

    // Spawn the async work on tokio runtime
    glib::MainContext::default().spawn_local(async move {
        let mut ancs_client = AncsClient::new();

        match ancs_client.connect().await {
            Ok(_) => {
                eprintln!("ANCS connection successful - listening for iPhone notifications");
                status_label.set_text("✓ ANCS connected (iPhone notifications active)");

                if let Some(mut rx) = ancs_client.take_notification_receiver() {
                    while let Some(notification) = rx.recv().await {
                        if let (Some(title), Some(message)) = (&notification.title, &notification.message) {
                            let sender = title.clone();
                            let msg = message.clone();

                            eprintln!("Received SMS notification: {} - {}", sender, msg);

                            // Add to UI (we're already on the main thread)
                            add_message_to_list(&list_box, &sender, &msg, &sender, false);

                            // Save to database
                            let state = app_state_clone.lock().await;
                            if let Some(pool) = &state.db_pool {
                                save_message_to_db(pool, &sender, &msg, "INCOMING").await;
                            }
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("Failed to start ANCS listener (this is normal for Android): {}", e);
                // Don't show error for ANCS - it's iPhone-only
                status_label.set_text("MAP connected (ANCS not available - Android or iPhone without notifications enabled)");
            }
        }
    });
}

fn add_message_to_list(list_box: &ListBox, _sender: &str, message: &str, display_name: &str, is_outgoing: bool) {
    let row = ListBoxRow::new();
    let row_box = GtkBox::new(Orientation::Vertical, 6);
    row_box.set_margin_start(12);
    row_box.set_margin_end(12);
    row_box.set_margin_top(8);
    row_box.set_margin_bottom(8);

    let header_box = GtkBox::new(Orientation::Horizontal, 12);

    let sender_label = Label::new(Some(display_name));
    sender_label.set_halign(gtk4::Align::Start);
    sender_label.add_css_class("title-4");

    let time_label = Label::new(Some(&chrono::Local::now().format("%H:%M").to_string()));
    time_label.set_halign(gtk4::Align::End);
    time_label.set_hexpand(true);
    time_label.add_css_class("dim-label");

    header_box.append(&sender_label);
    header_box.append(&time_label);

    let message_label = Label::new(Some(message));
    message_label.set_halign(gtk4::Align::Start);
    message_label.set_wrap(true);
    message_label.set_xalign(0.0);

    row_box.append(&header_box);
    row_box.append(&message_label);

    row.set_child(Some(&row_box));

    if is_outgoing {
        row.add_css_class("outgoing-message");
    }

    list_box.prepend(&row);
}

async fn load_messages_from_db(pool: sqlx::SqlitePool, list_box: ListBox) {
    match sqlx::query(
        "SELECT sender_normalized, message_body, direction, received_at
         FROM sms_messages
         ORDER BY received_at DESC
         LIMIT 50"
    )
    .fetch_all(&pool)
    .await
    {
        Ok(rows) => {
            for row in rows {
                let sender: String = row.get("sender_normalized");
                let message: String = row.get("message_body");
                let direction: String = row.get("direction");
                let is_outgoing = direction == "OUTGOING";

                let display_name = if is_outgoing { "You" } else { &sender };

                glib::idle_add_local_once({
                    let list_box = list_box.clone();
                    let sender = sender.clone();
                    let message = message.clone();
                    let display_name = display_name.to_string();

                    move || {
                        add_message_to_list(&list_box, &sender, &message, &display_name, is_outgoing);
                    }
                });
            }
        }
        Err(e) => {
            eprintln!("Failed to load messages: {}", e);
        }
    }
}

async fn save_message_to_db(pool: &sqlx::SqlitePool, sender: &str, message: &str, direction: &str) {
    let message_uid = format!("{}_{}", chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0), sender);
    let now = chrono::Utc::now().to_rfc3339();

    let _ = sqlx::query(
        "INSERT INTO sms_messages (message_uid, sender_normalized, message_body, direction, received_at)
         VALUES (?, ?, ?, ?, ?)"
    )
    .bind(&message_uid)
    .bind(sender)
    .bind(message)
    .bind(direction)
    .bind(&now)
    .execute(pool)
    .await;
}

/// Check if obexd service is available on D-Bus
async fn check_obexd_service() -> Result<bool, Box<dyn std::error::Error>> {
    let connection = zbus::Connection::session().await?;

    // Try to check if org.bluez.obex is available
    let proxy = zbus::fdo::DBusProxy::new(&connection).await?;
    let names = proxy.list_names().await?;

    Ok(names.iter().any(|name| name.as_str() == "org.bluez.obex"))
}
