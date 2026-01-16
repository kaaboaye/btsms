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
pub fn create_vmessage(recipient: &str, sender: &str, message: &str) -> String {
    let length = message.as_bytes().len();

    format!(
        "BEGIN:BMSG\r\n\
         VERSION:1.0\r\n\
         STATUS:READ\r\n\
         TYPE:SMS_GSM\r\n\
         FOLDER:telecom/msg/outbox\r\n\
         BEGIN:VCARD\r\n\
         VERSION:3.0\r\n\
         FN:{}\r\n\
         TEL:{}\r\n\
         END:VCARD\r\n\
         BEGIN:BENV\r\n\
         BEGIN:VCARD\r\n\
         VERSION:3.0\r\n\
         FN:{}\r\n\
         TEL:{}\r\n\
         END:VCARD\r\n\
         BEGIN:BBODY\r\n\
         CHARSET:UTF-8\r\n\
         LENGTH:{}\r\n\
         BEGIN:MSG\r\n\
         {}\r\n\
         END:MSG\r\n\
         END:BBODY\r\n\
         END:BENV\r\n\
         END:BMSG\r\n",
        recipient, recipient, sender, sender, length, message
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
        } else if line.starts_with("TYPE:") {
            message_type = line[5..].to_string();
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

/// Validates that a vMessage has correct format
pub fn validate_vmessage(content: &str) -> Result<()> {
    if !content.starts_with("BEGIN:BMSG") {
        return Err(BtsmsError::InvalidFormat("Must start with BEGIN:BMSG".to_string()));
    }

    if !content.contains("END:BMSG") {
        return Err(BtsmsError::InvalidFormat("Must end with END:BMSG".to_string()));
    }

    if !content.contains("\r\n") {
        return Err(BtsmsError::InvalidFormat("Must use \\r\\n line endings".to_string()));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_vmessage_basic() {
        let msg = create_vmessage("+15551234567", "+15559876543", "Hello World");

        assert!(msg.contains("BEGIN:BMSG"));
        assert!(msg.contains("END:BMSG"));
        assert!(msg.contains("+15551234567"));
        assert!(msg.contains("+15559876543"));
        assert!(msg.contains("Hello World"));
    }

    #[test]
    fn test_create_vmessage_has_correct_line_endings() {
        let msg = create_vmessage("+15551234567", "+15559876543", "Test");
        assert!(msg.contains("\r\n"));
    }

    #[test]
    fn test_create_vmessage_has_correct_length() {
        let message = "Hello";
        let msg = create_vmessage("+15551234567", "+15559876543", message);

        assert!(msg.contains(&format!("LENGTH:{}", message.len())));
    }

    #[test]
    fn test_create_vmessage_with_utf8() {
        let msg = create_vmessage("+15551234567", "+15559876543", "Hello 你好 мир");

        assert!(msg.contains("CHARSET:UTF-8"));
        assert!(msg.contains("Hello 你好 мир"));
    }

    #[test]
    fn test_parse_vmessage_valid() {
        let content = "BEGIN:BMSG\r\n\
                       VERSION:1.0\r\n\
                       TYPE:SMS_GSM\r\n\
                       BEGIN:VCARD\r\n\
                       TEL:+15551234567\r\n\
                       END:VCARD\r\n\
                       BEGIN:BENV\r\n\
                       BEGIN:VCARD\r\n\
                       TEL:+15559876543\r\n\
                       END:VCARD\r\n\
                       BEGIN:BBODY\r\n\
                       BEGIN:MSG\r\n\
                       Hello World\r\n\
                       END:MSG\r\n\
                       END:BBODY\r\n\
                       END:BENV\r\n\
                       END:BMSG\r\n";

        let parsed = parse_vmessage(content).unwrap();
        assert_eq!(parsed.recipient, "+15551234567");
        assert_eq!(parsed.sender, "+15559876543");
        assert_eq!(parsed.body, "Hello World");
        assert_eq!(parsed.message_type, "SMS_GSM");
    }

    #[test]
    fn test_parse_vmessage_missing_begin() {
        let content = "TYPE:SMS_GSM\r\nEND:BMSG\r\n";
        assert!(parse_vmessage(content).is_err());
    }

    #[test]
    fn test_parse_vmessage_missing_end() {
        let content = "BEGIN:BMSG\r\nTYPE:SMS_GSM\r\n";
        assert!(parse_vmessage(content).is_err());
    }

    #[test]
    fn test_parse_vmessage_missing_required_fields() {
        let content = "BEGIN:BMSG\r\nEND:BMSG\r\n";
        assert!(parse_vmessage(content).is_err());
    }

    #[test]
    fn test_validate_vmessage_valid() {
        let msg = create_vmessage("+15551234567", "+15559876543", "Test");
        assert!(validate_vmessage(&msg).is_ok());
    }

    #[test]
    fn test_validate_vmessage_missing_begin() {
        let content = "TYPE:SMS_GSM\r\nEND:BMSG\r\n";
        assert!(validate_vmessage(content).is_err());
    }

    #[test]
    fn test_validate_vmessage_missing_end() {
        let content = "BEGIN:BMSG\r\nTYPE:SMS_GSM\r\n";
        assert!(validate_vmessage(content).is_err());
    }

    #[test]
    fn test_validate_vmessage_wrong_line_endings() {
        let content = "BEGIN:BMSG\nEND:BMSG\n";
        assert!(validate_vmessage(content).is_err());
    }

    #[test]
    fn test_create_and_parse_roundtrip() {
        let original_recipient = "+15551234567";
        let original_sender = "+15559876543";
        let original_body = "Test message";

        let vmsg = create_vmessage(original_recipient, original_sender, original_body);
        let parsed = parse_vmessage(&vmsg).unwrap();

        assert_eq!(parsed.recipient, original_recipient);
        assert_eq!(parsed.sender, original_sender);
        assert_eq!(parsed.body, original_body);
    }

    #[test]
    fn test_parse_vmessage_with_special_chars() {
        let content = "BEGIN:BMSG\r\n\
                       TYPE:SMS_GSM\r\n\
                       BEGIN:VCARD\r\n\
                       TEL:+15551234567\r\n\
                       END:VCARD\r\n\
                       BEGIN:BENV\r\n\
                       BEGIN:VCARD\r\n\
                       TEL:+15559876543\r\n\
                       END:VCARD\r\n\
                       BEGIN:BBODY\r\n\
                       BEGIN:MSG\r\n\
                       Hello! @#$%^&*()\r\n\
                       END:MSG\r\n\
                       END:BBODY\r\n\
                       END:BENV\r\n\
                       END:BMSG\r\n";

        let parsed = parse_vmessage(content).unwrap();
        assert_eq!(parsed.body, "Hello! @#$%^&*()");
    }
}
