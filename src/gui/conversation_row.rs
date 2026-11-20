use btsms::db::Conversation;
use chrono::{DateTime, Local};
use gtk4::prelude::*;
use gtk4::{Box as GtkBox, Label, ListBoxRow, Orientation};

pub fn build_conversation_row(conversation: &Conversation) -> ListBoxRow {
    let row = ListBoxRow::new();

    let row_box = GtkBox::new(Orientation::Vertical, 2);
    row_box.set_margin_start(6);
    row_box.set_margin_end(6);
    row_box.set_margin_top(6);
    row_box.set_margin_bottom(6);

    let header = GtkBox::new(Orientation::Horizontal, 8);
    let name_label = Label::builder()
        .label(conversation.title())
        .halign(gtk4::Align::Start)
        .hexpand(true)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .css_classes(["heading", "conversation-title"])
        .build();
    let time_label = Label::builder()
        .label(format_relative_time(&conversation.last_message_time))
        .css_classes(["dim-label", "caption"])
        .build();
    header.append(&name_label);
    header.append(&time_label);

    let preview_row = GtkBox::new(Orientation::Horizontal, 6);
    let preview_text = if conversation.is_outgoing {
        format!("You: {}", conversation.last_message)
    } else {
        conversation.last_message.clone()
    };
    let preview_label = Label::builder()
        .label(truncate_message(&preview_text, 80))
        .halign(gtk4::Align::Start)
        .hexpand(true)
        .ellipsize(gtk4::pango::EllipsizeMode::End)
        .css_classes(["dim-label", "conversation-preview"])
        .build();
    preview_row.append(&preview_label);

    if conversation.unread_count > 0 {
        row.add_css_class("conversation-unread");
        preview_label.remove_css_class("dim-label");
        let badge = Label::builder()
            .label(conversation.unread_count.to_string())
            .valign(gtk4::Align::Center)
            .css_classes(["unread-badge"])
            .build();
        preview_row.append(&badge);
    }

    row_box.append(&header);
    row_box.append(&preview_row);
    row.set_child(Some(&row_box));
    row.set_tooltip_text(Some(&conversation.peer));
    row
}

/// One-line preview: newlines become spaces, long text is cut on a character boundary.
pub fn truncate_message(message: &str, max_chars: usize) -> String {
    let cleaned = message.replace(['\n', '\r'], " ");
    if cleaned.chars().count() > max_chars {
        let truncated: String = cleaned.chars().take(max_chars).collect();
        format!("{truncated}…")
    } else {
        cleaned
    }
}

fn to_local(timestamp: &str) -> Option<DateTime<Local>> {
    DateTime::parse_from_rfc3339(timestamp)
        .ok()
        .map(|dt| dt.with_timezone(&Local))
}

/// Sidebar time: "14:05" today, weekday this week, date otherwise.
pub fn format_relative_time(timestamp: &str) -> String {
    format_relative_time_at(timestamp, Local::now())
}

pub fn format_relative_time_at(timestamp: &str, now: DateTime<Local>) -> String {
    let Some(t) = to_local(timestamp) else {
        return timestamp.to_string();
    };
    if t.date_naive() == now.date_naive() {
        t.format("%H:%M").to_string()
    } else if now.signed_duration_since(t).num_days() < 6 && t <= now {
        t.format("%a").to_string()
    } else {
        t.format("%d.%m.%y").to_string()
    }
}

/// Time under a bubble.
pub fn format_bubble_time(timestamp: &str) -> String {
    to_local(timestamp)
        .map(|t| t.format("%H:%M").to_string())
        .unwrap_or_else(|| timestamp.to_string())
}

/// Calendar day of a stored timestamp, in local time.
pub fn local_day(timestamp: &str) -> Option<chrono::NaiveDate> {
    to_local(timestamp).map(|t| t.date_naive())
}

/// Heading for a day separator in the chat.
pub fn format_day(day: chrono::NaiveDate, today: chrono::NaiveDate) -> String {
    let diff = (today - day).num_days();
    match diff {
        0 => "Today".to_string(),
        1 => "Yesterday".to_string(),
        2..=6 => day.format("%A").to_string(),
        _ => day.format("%-d %B %Y").to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, TimeZone, Utc};

    fn local(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(y, m, d, h, min, 0).unwrap()
    }

    #[test]
    fn truncate_short_and_long() {
        assert_eq!(truncate_message("Hello", 50), "Hello");
        let long = "This is a very long message that should be truncated";
        let result = truncate_message(long, 20);
        assert!(result.ends_with('…'));
        assert_eq!(result.chars().count(), 21);
    }

    #[test]
    fn truncate_newlines_and_unicode() {
        assert_eq!(
            truncate_message("Line 1\nLine 2\r\nLine 3", 50),
            "Line 1 Line 2  Line 3"
        );
        let pl = "Btw BOSSA powiedziała ze MIFIDy musza być uzupełnione";
        assert_eq!(truncate_message(pl, 50).chars().count(), 51);
        let emoji = "Hello 👋 World 🌍 Test 🎉 More";
        assert_eq!(truncate_message(emoji, 15).chars().count(), 16);
    }

    #[test]
    fn relative_time_today_week_older() {
        let now = local(2026, 11, 20, 15, 0);
        let today = local(2026, 11, 20, 9, 5).with_timezone(&Utc).to_rfc3339();
        assert_eq!(format_relative_time_at(&today, now), "09:05");
        let monday = local(2026, 11, 16, 9, 5).with_timezone(&Utc).to_rfc3339();
        assert_eq!(format_relative_time_at(&monday, now), "Mon");
        let old = local(2026, 1, 2, 9, 5).with_timezone(&Utc).to_rfc3339();
        assert_eq!(format_relative_time_at(&old, now), "02.01.26");
        assert_eq!(format_relative_time_at("invalid", now), "invalid");
    }

    #[test]
    fn bubble_time_and_day() {
        let t = local(2026, 11, 20, 14, 30).with_timezone(&Utc).to_rfc3339();
        assert_eq!(format_bubble_time(&t), "14:30");
        assert_eq!(local_day(&t), NaiveDate::from_ymd_opt(2026, 11, 20));
        assert_eq!(format_bubble_time("14:30"), "14:30");
        assert_eq!(local_day("nope"), None);
    }

    #[test]
    fn day_headings() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 20).unwrap();
        assert_eq!(format_day(today, today), "Today");
        assert_eq!(format_day(today.pred_opt().unwrap(), today), "Yesterday");
        assert_eq!(
            format_day(NaiveDate::from_ymd_opt(2026, 11, 17).unwrap(), today),
            "Tuesday"
        );
        assert_eq!(
            format_day(NaiveDate::from_ymd_opt(2025, 3, 5).unwrap(), today),
            "5 March 2025"
        );
    }
}
