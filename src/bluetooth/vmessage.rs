//! bMessage (MAP 1.x, section 3.1.3) encoding and decoding.

use crate::contacts::vcard::{self, VCard};
use crate::error::{BtsmsError, Result};

/// A decoded bMessage.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BMessage {
    pub status: Option<String>,
    pub message_type: Option<String>,
    pub folder: Option<String>,
    /// vCards before the envelope: who wrote the message.
    pub originators: Vec<VCard>,
    /// vCards inside the envelope: who it was sent to.
    pub recipients: Vec<VCard>,
    pub charset: Option<String>,
    /// Message text with `\n` line endings.
    pub body: String,
}

impl BMessage {
    pub fn originator_phone(&self) -> Option<&str> {
        self.originators.iter().find_map(VCard::first_phone)
    }

    pub fn recipient_phone(&self) -> Option<&str> {
        self.recipients.iter().find_map(VCard::first_phone)
    }
}

/// Build the bMessage pushed to the phone's outbox to send an SMS.
///
/// The recipient goes in the envelope; the originator vCard is optional and omitted (the
/// phone fills in its own number). Line breaks in the text are sent as CRLF and `LENGTH`
/// counts the bytes from `BEGIN:MSG` up to and including the CRLF after `END:MSG`.
pub fn create_bmessage(recipient: &str, text: &str) -> String {
    let text = text
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', "\r\n");
    let msg_content = format!("BEGIN:MSG\r\n{text}\r\nEND:MSG\r\n");
    let length = msg_content.len();
    format!(
        "BEGIN:BMSG\r\n\
         VERSION:1.0\r\n\
         STATUS:UNREAD\r\n\
         TYPE:SMS_GSM\r\n\
         FOLDER:telecom/msg/outbox\r\n\
         BEGIN:BENV\r\n\
         BEGIN:VCARD\r\n\
         VERSION:2.1\r\n\
         N:{recipient}\r\n\
         TEL:{recipient}\r\n\
         END:VCARD\r\n\
         BEGIN:BBODY\r\n\
         CHARSET:UTF-8\r\n\
         LENGTH:{length}\r\n\
         {msg_content}\
         END:BBODY\r\n\
         END:BENV\r\n\
         END:BMSG\r\n"
    )
}

/// Decode a bMessage as returned by `org.bluez.obex.Message1.Get`.
pub fn parse_bmessage(content: &str) -> Result<BMessage> {
    let mut msg = BMessage::default();
    let mut seen_begin = false;
    let mut seen_end = false;
    let mut benv_depth = 0usize;
    let mut vcard_lines: Option<Vec<String>> = None;
    let mut in_bbody = false;
    let mut body_parts: Vec<String> = Vec::new();
    let mut current_part: Option<Vec<String>> = None;

    for raw in content.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);

        // Message text is taken verbatim.
        if let Some(part) = current_part.as_mut() {
            if line.trim() == "END:MSG" {
                body_parts.push(part.join("\n"));
                current_part = None;
            } else {
                part.push(line.to_string());
            }
            continue;
        }

        let trimmed = line.trim();
        let upper = trimmed.to_ascii_uppercase();

        if let Some(lines) = vcard_lines.as_mut() {
            if upper == "END:VCARD" {
                let joined = lines.join("\n");
                let unfolded = vcard::unfold(&joined);
                let card = VCard::from_lines(unfolded.iter().map(String::as_str));
                if benv_depth == 0 {
                    msg.originators.push(card);
                } else {
                    msg.recipients.push(card);
                }
                vcard_lines = None;
            } else {
                lines.push(line.to_string());
            }
            continue;
        }

        match upper.as_str() {
            "BEGIN:BMSG" => seen_begin = true,
            "END:BMSG" => seen_end = true,
            "BEGIN:VCARD" => vcard_lines = Some(Vec::new()),
            "BEGIN:BENV" => benv_depth += 1,
            "END:BENV" => benv_depth = benv_depth.saturating_sub(1),
            "BEGIN:BBODY" => in_bbody = true,
            "END:BBODY" => in_bbody = false,
            "BEGIN:MSG" if in_bbody => current_part = Some(Vec::new()),
            _ => {
                if let Some((key, value)) = trimmed.split_once(':') {
                    let value = value.trim().to_string();
                    match key.to_ascii_uppercase().as_str() {
                        "STATUS" if benv_depth == 0 => msg.status = Some(value),
                        "TYPE" if benv_depth == 0 => msg.message_type = Some(value),
                        "FOLDER" if benv_depth == 0 => msg.folder = Some(value),
                        "CHARSET" if in_bbody => msg.charset = Some(value),
                        _ => {}
                    }
                }
            }
        }
    }

    if !seen_begin || !seen_end {
        return Err(BtsmsError::InvalidFormat(
            "missing BEGIN:BMSG/END:BMSG".to_string(),
        ));
    }
    if let Some(part) = current_part {
        // Unterminated MSG block: keep what we got rather than losing the text.
        body_parts.push(part.join("\n"));
    }
    msg.body = body_parts.join("\n");
    Ok(msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shape of what an iPhone returns for an inbox message.
    const IOS_INBOX: &str = "BEGIN:BMSG\r\nVERSION:1.0\r\nSTATUS:READ\r\nTYPE:SMS_GSM\r\n\
        FOLDER:telecom/msg/inbox\r\nNOTIFICATION:1\r\nBEGIN:VCARD\r\nVERSION:2.1\r\n\
        FN;CHARSET=UTF-8:Jan Kowalski\r\nN;CHARSET=UTF-8:Kowalski;Jan\r\nTEL:+48512345678\r\n\
        END:VCARD\r\nBEGIN:BENV\r\nBEGIN:VCARD\r\nVERSION:2.1\r\nN;CHARSET=UTF-8:\r\n\
        TEL;CHARSET=UTF-8:+48600000000\r\nEND:VCARD\r\nBEGIN:BBODY\r\nCHARSET:UTF-8\r\n\
        LANGUAGE:UNKNOWN\r\nLENGTH:52\r\nBEGIN:MSG\r\nCześć!\r\nDruga linia\r\nEND:MSG\r\n\
        END:BBODY\r\nEND:BENV\r\nEND:BMSG\r\n";

    #[test]
    fn parses_ios_inbox_message() {
        let msg = parse_bmessage(IOS_INBOX).unwrap();
        assert_eq!(msg.status.as_deref(), Some("READ"));
        assert_eq!(msg.message_type.as_deref(), Some("SMS_GSM"));
        assert_eq!(msg.folder.as_deref(), Some("telecom/msg/inbox"));
        assert_eq!(msg.originator_phone(), Some("+48512345678"));
        assert_eq!(
            msg.originators[0].display_name().as_deref(),
            Some("Jan Kowalski")
        );
        assert_eq!(msg.recipient_phone(), Some("+48600000000"));
        assert_eq!(msg.charset.as_deref(), Some("UTF-8"));
        assert_eq!(msg.body, "Cześć!\nDruga linia");
    }

    #[test]
    fn body_keeps_whitespace_and_vcard_like_lines() {
        let content = "BEGIN:BMSG\nBEGIN:BENV\nBEGIN:BBODY\nBEGIN:MSG\n  indented\nTEL:not a header\n\nEND:MSG\nEND:BBODY\nEND:BENV\nEND:BMSG\n";
        let msg = parse_bmessage(content).unwrap();
        assert_eq!(msg.body, "  indented\nTEL:not a header\n");
        assert!(msg.recipients.is_empty());
    }

    #[test]
    fn rejects_non_bmessage() {
        assert!(parse_bmessage("").is_err());
        assert!(parse_bmessage("hello").is_err());
        assert!(parse_bmessage("BEGIN:BMSG\nVERSION:1.0\n").is_err());
    }

    #[test]
    fn empty_body_is_allowed() {
        let msg = parse_bmessage("BEGIN:BMSG\nEND:BMSG\n").unwrap();
        assert_eq!(msg.body, "");
    }

    #[test]
    fn create_has_recipient_crlf_and_utf8() {
        let msg = create_bmessage("+48512345678", "Hello 你好 мир");
        assert!(msg.starts_with("BEGIN:BMSG\r\nVERSION:1.0\r\n"));
        assert!(msg.ends_with("END:BMSG\r\n"));
        assert!(msg.contains("TEL:+48512345678\r\n"));
        assert!(msg.contains("CHARSET:UTF-8\r\n"));
        assert!(msg.contains("Hello 你好 мир"));
        assert!(!msg.replace("\r\n", "").contains('\n'));
    }

    #[test]
    fn create_length_counts_bytes_of_msg_block() {
        let text = "Zażółć\ngęślą";
        let msg = create_bmessage("+48512345678", text);
        let expected = "BEGIN:MSG\r\nZażółć\r\ngęślą\r\nEND:MSG\r\n".len();
        assert!(msg.contains(&format!("LENGTH:{expected}\r\n")));
    }

    #[test]
    fn create_normalizes_line_endings() {
        let msg = create_bmessage("1", "a\r\nb\rc\nd");
        assert!(msg.contains("BEGIN:MSG\r\na\r\nb\r\nc\r\nd\r\nEND:MSG\r\n"));
    }

    #[test]
    fn create_then_parse_roundtrip() {
        let text = "Line one\nLine two";
        let parsed = parse_bmessage(&create_bmessage("+48512345678", text)).unwrap();
        assert_eq!(parsed.body, text);
        assert_eq!(parsed.recipient_phone(), Some("+48512345678"));
        assert!(parsed.originators.is_empty());
        assert_eq!(parsed.folder.as_deref(), Some("telecom/msg/outbox"));
    }
}
