use btsms::db::Conversation;
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Label, ListBox, ListBoxRow, Orientation};

pub fn add_conversation_row(list_box: &ListBox, conversation: &Conversation) {
    let row = ListBoxRow::new();
    row.set_widget_name(&conversation.phone_number);

    let row_box = GtkBox::new(Orientation::Vertical, 4);
    row_box.set_margin_start(12);
    row_box.set_margin_end(12);
    row_box.set_margin_top(8);
    row_box.set_margin_bottom(8);

    let header_box = GtkBox::new(Orientation::Horizontal, 8);

    let name = conversation
        .display_name
        .as_deref()
        .unwrap_or(&conversation.phone_number);
    let name_label = Label::new(Some(name));
    name_label.set_halign(gtk4::Align::Start);
    name_label.set_hexpand(true);
    name_label.add_css_class("heading");
    name_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);

    let time_str = format_relative_time(&conversation.last_message_time);
    let time_label = Label::new(Some(&time_str));
    time_label.add_css_class("dim-label");
    time_label.add_css_class("caption");

    header_box.append(&name_label);
    header_box.append(&time_label);

    let preview = truncate_message(&conversation.last_message, 50);
    let preview_label = Label::new(Some(&preview));
    preview_label.set_halign(gtk4::Align::Start);
    preview_label.add_css_class("dim-label");
    preview_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);

    row_box.append(&header_box);
    row_box.append(&preview_label);

    if conversation.unread_count > 0 {
        name_label.add_css_class("bold");
        preview_label.remove_css_class("dim-label");
    }

    row.set_child(Some(&row_box));
    list_box.append(&row);
}

pub fn truncate_message(message: &str, max_len: usize) -> String {
    let cleaned = message.replace('\n', " ");
    if cleaned.len() > max_len {
        format!("{}...", &cleaned[..max_len])
    } else {
        cleaned
    }
}

pub fn format_relative_time(timestamp: &str) -> String {
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
        timestamp.to_string()
    }
}

/// Parse MAP timestamp format (e.g., "20240115T143022" or "20240115T143022+0100") to RFC3339
pub fn parse_map_timestamp(timestamp: &str) -> String {
    if timestamp.len() >= 15 {
        let year = &timestamp[0..4];
        let month = &timestamp[4..6];
        let day = &timestamp[6..8];
        let hour = &timestamp[9..11];
        let minute = &timestamp[11..13];
        let second = &timestamp[13..15];

        let tz = if timestamp.len() > 15 {
            let tz_part = &timestamp[15..];
            if tz_part.starts_with('+') || tz_part.starts_with('-') {
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

        format!(
            "{}-{}-{}T{}:{}:{}{}",
            year, month, day, hour, minute, second, tz
        )
    } else if timestamp.is_empty() {
        chrono::Utc::now().to_rfc3339()
    } else {
        timestamp.to_string()
    }
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
        let msg =
            "This is a very long message that should be truncated because it exceeds the maximum length";
        let result = truncate_message(msg, 20);
        assert!(result.ends_with("..."));
        assert!(result.len() <= 23);
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
        assert!(result.contains('T'));
        assert!(result.contains('-'));
    }

    #[test]
    fn test_parse_map_timestamp_invalid() {
        let result = parse_map_timestamp("invalid");
        assert_eq!(result, "invalid");
    }
}
