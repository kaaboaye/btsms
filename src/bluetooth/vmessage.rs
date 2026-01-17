use crate::error::{BtsmsError, Result};

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedMessage {
    pub recipient: String,
    pub sender: String,
    pub body: String,
    pub message_type: String,
}

/// Creates a vMessage/BMSG format string for SMS sending via MAP
/// Uses \r\n line endings as required by the specification
///
/// According to MAP spec:
/// - Originator (first VCARD, before BENV) = the sender (us)
/// - Recipient (VCARD inside BENV) = the receiver (them)
pub fn create_vmessage(recipient: &str, _sender: &str, message: &str) -> String {
    // Calculate LENGTH which should include BEGIN:MSG and END:MSG markers plus content
    // The spec says: LENGTH counts from "B" of "BEGIN:MSG" to CRLF of last "END:MSG"
    let msg_content = format!("BEGIN:MSG\r\n{}\r\nEND:MSG\r\n", message);
    let length = msg_content.len();

    // Note: For outgoing SMS, the originator VCARD can be empty or minimal
    // The recipient goes inside BENV
    format!(
        "BEGIN:BMSG\r\n\
VERSION:1.0\r\n\
STATUS:UNREAD\r\n\
TYPE:SMS_GSM\r\n\
FOLDER:telecom/msg/outbox\r\n\
BEGIN:BENV\r\n\
BEGIN:VCARD\r\n\
VERSION:2.1\r\n\
N:{}\r\n\
TEL:{}\r\n\
END:VCARD\r\n\
BEGIN:BBODY\r\n\
CHARSET:UTF-8\r\n\
LENGTH:{}\r\n\
{}\
END:BBODY\r\n\
END:BENV\r\n\
END:BMSG\r\n",
        recipient, recipient, length, msg_content
    )
}

/// Parses a vMessage/BMSG format string
pub fn parse_vmessage(content: &str) -> Result<ParsedMessage> {
    if !content.contains("BEGIN:BMSG") || !content.contains("END:BMSG") {
        return Err(BtsmsError::InvalidFormat("Missing BMSG markers".to_string()));
    }

    let lines: Vec<&str> = content.lines().collect();

    let mut recipient = String::new();
    let mut sender = String::new();
    let mut body = String::new();
    let mut message_type = String::from("SMS_GSM");
    let mut in_first_vcard = false;
    let mut in_second_vcard = false;
    let mut in_msg = false;

    for line in lines.iter() {
        let line = line.trim();

        if line == "BEGIN:VCARD" {
            if recipient.is_empty() {
                in_first_vcard = true;
            } else {
                in_second_vcard = true;
            }
        } else if line == "END:VCARD" {
            in_first_vcard = false;
            in_second_vcard = false;
        } else if line == "BEGIN:MSG" {
            in_msg = true;
        } else if line == "END:MSG" {
            in_msg = false;
        } else if let Some(stripped) = line.strip_prefix("TYPE:") {
            message_type = stripped.to_string();
        } else if in_first_vcard && line.starts_with("TEL:") {
            recipient = line[4..].to_string();
        } else if in_second_vcard && line.starts_with("TEL:") {
            sender = line[4..].to_string();
        } else if in_msg && !line.is_empty() {
            body = line.to_string();
        }
    }

    if recipient.is_empty() || body.is_empty() {
        return Err(BtsmsError::InvalidFormat("Missing required fields".to_string()));
    }

    Ok(ParsedMessage {
        recipient,
        sender,
        body,
        message_type,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_vmessage_basic() {
        let msg = create_vmessage("+15551234567", "", "Hello World");

        assert!(msg.contains("BEGIN:BMSG"));
        assert!(msg.contains("END:BMSG"));
        assert!(msg.contains("+15551234567")); // recipient
        assert!(msg.contains("Hello World"));
        assert!(msg.contains("BEGIN:BENV"));
        assert!(msg.contains("END:BENV"));
    }

    #[test]
    fn test_create_vmessage_debug_output() {
        let msg = create_vmessage("+48794097915", "", "Hello!");
        println!("\n=== Generated bMessage ===\n{}\n=== End ===", msg);
    }

    #[test]
    fn test_create_vmessage_has_correct_line_endings() {
        let msg = create_vmessage("+15551234567", "", "Test");
        assert!(msg.contains("\r\n"));
    }

    #[test]
    fn test_create_vmessage_has_correct_length() {
        let message = "Hello";
        let msg = create_vmessage("+15551234567", "", message);

        // LENGTH includes BEGIN:MSG\r\n + message + \r\nEND:MSG\r\n
        let expected_len = format!("BEGIN:MSG\r\n{}\r\nEND:MSG\r\n", message).len();
        assert!(msg.contains(&format!("LENGTH:{}", expected_len)));
    }

    #[test]
    fn test_create_vmessage_with_utf8() {
        let msg = create_vmessage("+15551234567", "", "Hello 你好 мир");

        assert!(msg.contains("CHARSET:UTF-8"));
        assert!(msg.contains("Hello 你好 мир"));
    }

}
