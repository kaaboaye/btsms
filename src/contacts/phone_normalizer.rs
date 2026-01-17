use crate::error::{BtsmsError, Result};

/// Normalizes a phone number to E.164 format: +[CountryCode][Number]
/// Example: "555-123-4567" -> "+15551234567"
pub fn normalize_e164(phone: &str) -> Result<String> {
    if phone.is_empty() {
        return Err(BtsmsError::Parse("Empty phone number".to_string()));
    }

    // Extract only digits and '+'
    let digits: String = phone
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '+')
        .collect();

    if digits.is_empty() {
        return Err(BtsmsError::Parse(format!("No digits found in: {}", phone)));
    }

    // If already starts with +, validate and return
    if digits.starts_with('+') {
        if digits.len() < 8 {
            return Err(BtsmsError::Parse(format!(
                "Phone number too short: {}",
                phone
            )));
        }
        return Ok(digits);
    }

    // Assume US/Canada (+1) if no country code
    let normalized = if digits.len() == 10 {
        format!("+1{}", digits)
    } else if digits.len() == 11 && digits.starts_with('1') {
        format!("+{}", digits)
    } else if digits.len() < 7 {
        return Err(BtsmsError::Parse(format!(
            "Phone number too short: {}",
            phone
        )));
    } else {
        // Unknown format, assume it's international without +
        format!("+{}", digits)
    };

    Ok(normalized)
}

/// Check if a phone number is in valid E.164 format
pub fn is_valid_e164(phone: &str) -> bool {
    if !phone.starts_with('+') {
        return false;
    }

    let digits: String = phone
        .chars()
        .skip(1)
        .filter(|c| c.is_ascii_digit())
        .collect();
    digits.len() >= 7 && digits.len() <= 15
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_us_number_with_dashes() {
        assert_eq!(normalize_e164("555-123-4567").unwrap(), "+15551234567");
    }

    #[test]
    fn test_normalize_us_number_with_parens() {
        assert_eq!(normalize_e164("(555) 123-4567").unwrap(), "+15551234567");
    }

    #[test]
    fn test_normalize_us_number_with_spaces() {
        assert_eq!(normalize_e164("555 123 4567").unwrap(), "+15551234567");
    }

    #[test]
    fn test_normalize_us_number_with_dots() {
        assert_eq!(normalize_e164("555.123.4567").unwrap(), "+15551234567");
    }

    #[test]
    fn test_normalize_us_number_with_leading_1() {
        assert_eq!(normalize_e164("1-555-123-4567").unwrap(), "+15551234567");
    }

    #[test]
    fn test_normalize_already_e164() {
        assert_eq!(normalize_e164("+15551234567").unwrap(), "+15551234567");
    }

    #[test]
    fn test_normalize_international_uk() {
        assert_eq!(normalize_e164("+44 20 7123 4567").unwrap(), "+442071234567");
    }

    #[test]
    fn test_normalize_international_germany() {
        assert_eq!(normalize_e164("+49 30 12345678").unwrap(), "+493012345678");
    }

    #[test]
    fn test_empty_string_returns_error() {
        assert!(normalize_e164("").is_err());
    }

    #[test]
    fn test_no_digits_returns_error() {
        assert!(normalize_e164("not-a-number").is_err());
    }

    #[test]
    fn test_too_short_returns_error() {
        assert!(normalize_e164("123").is_err());
    }

    #[test]
    fn test_special_chars_removed() {
        assert_eq!(normalize_e164("+1 (555) 123-4567").unwrap(), "+15551234567");
    }

    #[test]
    fn test_is_valid_e164_valid() {
        assert!(is_valid_e164("+15551234567"));
        assert!(is_valid_e164("+442071234567"));
    }

    #[test]
    fn test_is_valid_e164_invalid_no_plus() {
        assert!(!is_valid_e164("15551234567"));
    }

    #[test]
    fn test_is_valid_e164_invalid_too_short() {
        assert!(!is_valid_e164("+123"));
    }

    #[test]
    fn test_is_valid_e164_invalid_too_long() {
        assert!(!is_valid_e164("+1234567890123456"));
    }
}
