use gtk4::prelude::*;
use gtk4::{
    glib, Application, ApplicationWindow, Box as GtkBox, Button, Entry, Label, ListBox,
    ListBoxRow, Orientation, ScrolledWindow, SelectionMode,
};
use libadwaita::{self as adw, HeaderBar};
use std::sync::{Arc, Mutex};
use sqlx::SqlitePool;

pub fn build_ui(app: &Application) {
    // Create main window
    let window = ApplicationWindow::builder()
        .application(app)
        .title("Bluetooth SMS")
        .default_width(800)
        .default_height(600)
        .build();

    // Main container
    let main_box = GtkBox::new(Orientation::Vertical, 0);

    // Header bar
    let header = HeaderBar::new();
    header.set_title_widget(Some(&Label::new(Some("Bluetooth SMS"))));

    let status_label = Label::new(Some("Not connected"));
    status_label.add_css_class("dim-label");
    header.pack_end(&status_label);

    main_box.append(&header);

    // Content area with messages
    let content_box = GtkBox::new(Orientation::Vertical, 12);
    content_box.set_margin_start(12);
    content_box.set_margin_end(12);
    content_box.set_margin_top(12);
    content_box.set_margin_bottom(12);

    // Message list
    let list_label = Label::new(Some("Recent Messages"));
    list_label.set_halign(gtk4::Align::Start);
    list_label.add_css_class("title-2");
    content_box.append(&list_label);

    let scrolled = ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .min_content_height(400)
        .vexpand(true)
        .build();

    let list_box = ListBox::new();
    list_box.set_selection_mode(SelectionMode::None);
    list_box.add_css_class("boxed-list");

    scrolled.set_child(Some(&list_box));
    content_box.append(&scrolled);

    // Compose area
    let compose_box = GtkBox::new(Orientation::Horizontal, 6);

    let recipient_entry = Entry::builder()
        .placeholder_text("Phone number")
        .width_request(150)
        .build();

    let message_entry = Entry::builder()
        .placeholder_text("Type a message...")
        .hexpand(true)
        .build();

    let send_button = Button::with_label("Send");
    send_button.add_css_class("suggested-action");

    compose_box.append(&recipient_entry);
    compose_box.append(&message_entry);
    compose_box.append(&send_button);

    content_box.append(&compose_box);
    main_box.append(&content_box);

    window.set_content(Some(&main_box));

    // Initialize database and load messages
    let list_box_clone = list_box.clone();
    let status_clone = status_label.clone();

    glib::spawn_future_local(async move {
        match init_and_load_messages(list_box_clone, status_clone).await {
            Ok(_) => {}
            Err(e) => eprintln!("Error initializing database: {}", e),
        }
    });

    // Send button handler
    let list_box_send = list_box.clone();
    let recipient_clone = recipient_entry.clone();
    let message_clone = message_entry.clone();

    send_button.connect_clicked(move |_| {
        let recipient = recipient_clone.text().to_string();
        let message = message_clone.text().to_string();

        if !recipient.is_empty() && !message.is_empty() {
            let list_box_inner = list_box_send.clone();
            let message_entry_inner = message_clone.clone();

            glib::spawn_future_local(async move {
                match send_message(&recipient, &message).await {
                    Ok(_) => {
                        message_entry_inner.set_text("");
                        // Reload messages
                        let _ = load_messages(list_box_inner, None).await;
                    }
                    Err(e) => eprintln!("Error sending message: {}", e),
                }
            });
        }
    });

    // Add some sample messages for testing
    add_sample_message(&list_box, "+1 (555) 123-4567", "Hey, how are you?", "2 min ago");
    add_sample_message(&list_box, "+1-555-987-6543", "Meeting at 3pm", "1 hour ago");
    add_sample_message(&list_box, "John Doe", "Thanks for your help!", "Yesterday");

    window.present();
}

fn add_sample_message(list_box: &ListBox, sender: &str, message: &str, time: &str) {
    let row = ListBoxRow::new();
    let box_content = GtkBox::new(Orientation::Vertical, 6);
    box_content.set_margin_start(12);
    box_content.set_margin_end(12);
    box_content.set_margin_top(8);
    box_content.set_margin_bottom(8);

    let header_box = GtkBox::new(Orientation::Horizontal, 6);
    let sender_label = Label::new(Some(sender));
    sender_label.set_halign(gtk4::Align::Start);
    sender_label.add_css_class("heading");

    let time_label = Label::new(Some(time));
    time_label.set_halign(gtk4::Align::End);
    time_label.set_hexpand(true);
    time_label.add_css_class("dim-label");

    header_box.append(&sender_label);
    header_box.append(&time_label);

    let message_label = Label::new(Some(message));
    message_label.set_halign(gtk4::Align::Start);
    message_label.set_wrap(true);

    box_content.append(&header_box);
    box_content.append(&message_label);

    row.set_child(Some(&box_content));
    list_box.append(&row);
}

async fn init_and_load_messages(
    list_box: ListBox,
    status_label: Label,
) -> Result<(), Box<dyn std::error::Error>> {
    let home = std::env::var("HOME")?;
    let db_path = format!("{}/.local/share/btsms/btsms.db", home);

    let pool = btsms::db::init_database(&db_path).await?;
    status_label.set_text("Connected to database");

    load_messages(list_box, Some(pool)).await?;
    Ok(())
}

async fn load_messages(
    list_box: ListBox,
    pool: Option<SqlitePool>,
) -> Result<(), Box<dyn std::error::Error>> {
    let pool = if let Some(p) = pool {
        p
    } else {
        let home = std::env::var("HOME")?;
        let db_path = format!("{}/.local/share/btsms/btsms.db", home);
        btsms::db::init_database(&db_path).await?
    };

    let messages = btsms::db::get_recent_messages(&pool, 50).await?;

    // Clear existing children
    while let Some(child) = list_box.first_child() {
        list_box.remove(&child);
    }

    for msg in messages {
        let display_sender = msg.sender_name.unwrap_or(msg.sender_number.clone());
        add_sample_message(&list_box, &display_sender, &msg.body, &format_time(&msg.received_at));
    }

    Ok(())
}

async fn send_message(recipient: &str, message: &str) -> Result<(), Box<dyn std::error::Error>> {
    let home = std::env::var("HOME")?;
    let db_path = format!("{}/.local/share/btsms/btsms.db", home);
    let pool = btsms::db::init_database(&db_path).await?;

    btsms::db::insert_message(
        &pool,
        recipient,
        None,
        message,
        btsms::db::MessageDirection::Outgoing,
    )
    .await?;

    Ok(())
}

fn format_time(timestamp: &str) -> String {
    // Simple time formatting - could be improved
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(timestamp) {
        let now = chrono::Utc::now();
        let diff = now.signed_duration_since(dt);

        if diff.num_minutes() < 60 {
            format!("{} min ago", diff.num_minutes())
        } else if diff.num_hours() < 24 {
            format!("{} hours ago", diff.num_hours())
        } else {
            format!("{} days ago", diff.num_days())
        }
    } else {
        "Unknown".to_string()
    }
}
