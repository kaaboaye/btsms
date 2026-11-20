//! Phone number normalisation.
//!
//! Numbers are normalised to E.164 (`+48123456789`) using libphonenumber metadata and a
//! default region for numbers written without a country code. The region comes from the
//! config or the locale; there is deliberately no hard-coded fallback country (the old code
//! assumed the US, which turned Polish `512…` numbers into `+512…`).
//!
//! Two entry points:
//! * [`normalize_address`] never fails. It produces the key used to group conversations and
//!   match contacts, and copes with short codes (`8080`), alphanumeric senders (`ING`) and
//!   iMessage e-mail addresses.
//! * [`normalize_for_sending`] validates user input before an SMS is pushed to the phone,
//!   without changing which number it goes to.

use crate::error::{BtsmsError, Result};
use phonenumber::country::Id;
use std::sync::RwLock;

static DEFAULT_REGION: RwLock<Option<Id>> = RwLock::new(None);

/// Shortest digit string treated as a full phone number rather than a short code.
const MIN_FULL_NUMBER_DIGITS: usize = 7;

/// Parse an ISO 3166 region code like `"PL"`.
pub fn parse_region(region: &str) -> Option<Id> {
    region.trim().to_ascii_uppercase().parse().ok()
}

/// Set the process-wide default region (call once at startup from the config).
pub fn set_default_region(region: Option<&str>) {
    let id = region.and_then(parse_region);
    if region.is_some() && id.is_none() {
        tracing::warn!(
            ?region,
            "unknown phone region, numbers without a country code stay as typed"
        );
    }
    tracing::info!(region = ?id, "default phone region");
    *DEFAULT_REGION.write().unwrap_or_else(|e| e.into_inner()) = id;
}

/// The process-wide default region.
pub fn default_region() -> Option<Id> {
    *DEFAULT_REGION.read().unwrap_or_else(|e| e.into_inner())
}

/// Normalise any sender/recipient address using the default region. Never fails.
pub fn normalize_address(raw: &str) -> String {
    normalize_address_with(raw, default_region())
}

/// Normalise any sender/recipient address. Never fails; returns a stable key.
pub fn normalize_address_with(raw: &str, region: Option<Id>) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.contains('@') {
        // iMessage e-mail handle
        return trimmed.to_lowercase();
    }
    if trimmed.chars().any(char::is_alphabetic) {
        // Alphanumeric sender id ("ING", "Allegro")
        return trimmed.to_string();
    }
    let cleaned = clean_digits(trimmed);
    let digit_count = cleaned.trim_start_matches('+').len();
    if digit_count == 0 {
        return trimmed.to_string();
    }
    if digit_count < MIN_FULL_NUMBER_DIGITS {
        // Short code: no country code applies
        return cleaned.trim_start_matches('+').to_string();
    }
    match phonenumber::parse(region, &cleaned) {
        Ok(number) => number.format().mode(phonenumber::Mode::E164).to_string(),
        Err(_) => cleaned,
    }
}

/// Validate a number typed by the user before sending and strip its formatting.
///
/// The number is deliberately **not** rewritten to E.164: the region may be guessed from the
/// locale (many people run `en_US` outside the US), and a wrong guess would send the SMS to a
/// different country. The phone applies its own network's dialling rules to national numbers,
/// so passing digits through as typed always reaches the intended recipient.
pub fn normalize_for_sending(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(BtsmsError::Parse("Empty phone number".into()));
    }
    if trimmed.chars().any(char::is_alphabetic) {
        return Err(BtsmsError::Parse(format!(
            "'{trimmed}' is not a phone number"
        )));
    }
    let plus = trimmed.starts_with('+');
    let digits: String = trimmed.chars().filter(char::is_ascii_digit).collect();
    if digits.len() < 3 {
        return Err(BtsmsError::Parse(format!(
            "Phone number too short: '{trimmed}'"
        )));
    }
    if digits.len() > 17 {
        return Err(BtsmsError::Parse(format!(
            "Phone number too long: '{trimmed}'"
        )));
    }
    Ok(if plus { format!("+{digits}") } else { digits })
}

/// Keep digits and a leading `+`; turn the international `00` prefix into `+`.
fn clean_digits(s: &str) -> String {
    let plus = s.trim_start().starts_with('+');
    let digits: String = s.chars().filter(char::is_ascii_digit).collect();
    if plus {
        format!("+{digits}")
    } else if let Some(rest) = digits.strip_prefix("00")
        && rest.len() >= MIN_FULL_NUMBER_DIGITS
    {
        format!("+{rest}")
    } else {
        digits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pl() -> Option<Id> {
        Some(Id::PL)
    }

    #[test]
    fn polish_national_number_gets_country_code() {
        assert_eq!(normalize_address_with("512 345 678", pl()), "+48512345678");
        assert_eq!(normalize_address_with("512-345-678", pl()), "+48512345678");
    }

    #[test]
    fn international_numbers_keep_their_country() {
        assert_eq!(
            normalize_address_with("+48 512 345 678", pl()),
            "+48512345678"
        );
        assert_eq!(
            normalize_address_with("+44 20 7123 4567", pl()),
            "+442071234567"
        );
        assert_eq!(
            normalize_address_with("+1 (202) 555-0143", pl()),
            "+12025550143"
        );
    }

    #[test]
    fn double_zero_prefix_is_international() {
        assert_eq!(
            normalize_address_with("0048512345678", pl()),
            "+48512345678"
        );
        assert_eq!(
            normalize_address_with("0044 20 7123 4567", None),
            "+442071234567"
        );
    }

    #[test]
    fn trunk_prefix_is_dropped() {
        // UK mobile written nationally with the leading 0
        assert_eq!(
            normalize_address_with("07700 900123", Some(Id::GB)),
            "+447700900123"
        );
    }

    #[test]
    fn no_region_no_guessing() {
        // Without a region we cannot know the country: keep the digits, don't invent one.
        assert_eq!(normalize_address_with("512345678", None), "512345678");
        assert_eq!(normalize_address_with("+48512345678", None), "+48512345678");
    }

    #[test]
    fn short_codes_stay_short() {
        assert_eq!(normalize_address_with("8080", pl()), "8080");
        assert_eq!(normalize_address_with("80 80", pl()), "8080");
    }

    #[test]
    fn alphanumeric_senders_are_kept() {
        assert_eq!(normalize_address_with("ING", pl()), "ING");
        assert_eq!(normalize_address_with("  Allegro ", pl()), "Allegro");
    }

    #[test]
    fn email_handles_are_lowercased() {
        assert_eq!(
            normalize_address_with("John.Doe@iCloud.com", pl()),
            "john.doe@icloud.com"
        );
    }

    #[test]
    fn empty_and_garbage() {
        assert_eq!(normalize_address_with("", pl()), "");
        assert_eq!(normalize_address_with("   ", pl()), "");
        assert_eq!(normalize_address_with("---", pl()), "---");
    }

    #[test]
    fn same_number_different_formats_match() {
        let a = normalize_address_with("+48 512 345 678", pl());
        let b = normalize_address_with("512345678", pl());
        let c = normalize_address_with("0048512345678", pl());
        assert_eq!(a, b);
        assert_eq!(b, c);
    }

    #[test]
    fn sending_keeps_the_number_as_typed() {
        assert_eq!(normalize_for_sending("512 345 678").unwrap(), "512345678");
        assert_eq!(
            normalize_for_sending("+48 512-345-678").unwrap(),
            "+48512345678"
        );
        assert_eq!(
            normalize_for_sending("0048 512 345 678").unwrap(),
            "0048512345678"
        );
        assert_eq!(normalize_for_sending("8080").unwrap(), "8080");
    }

    #[test]
    fn sending_never_guesses_a_country() {
        // An Indian number typed by someone with LANG=en_US must not become +1... (the
        // function takes no region at all).
        assert_eq!(normalize_for_sending("9123456789").unwrap(), "9123456789");
    }

    #[test]
    fn sending_rejects_non_numbers() {
        assert!(normalize_for_sending("").is_err());
        assert!(normalize_for_sending("John").is_err());
        assert!(normalize_for_sending("12").is_err());
        assert!(normalize_for_sending("+1234567890123456789").is_err());
    }

    #[test]
    fn region_parsing() {
        assert_eq!(parse_region("pl"), Some(Id::PL));
        assert_eq!(parse_region(" GB "), Some(Id::GB));
        assert_eq!(parse_region("XX"), None);
        assert_eq!(parse_region(""), None);
    }

    #[test]
    fn clean_digits_handles_formatting() {
        assert_eq!(clean_digits("+1 (555) 123-4567"), "+15551234567");
        assert_eq!(clean_digits("0048 123 456 789"), "+48123456789");
        assert_eq!(clean_digits("00 12"), "0012");
    }
}
