//! Lenient vCard 2.1 / 3.0 parser for PBAP phonebooks and bMessage envelopes.
//!
//! Handles line folding, `QUOTED-PRINTABLE` (with soft line breaks), parameters in both the
//! 2.1 (`TEL;CELL:`) and 3.0 (`TEL;TYPE=CELL:`) styles, property groups (`item1.TEL:`) and
//! value escapes (`\,` `\;` `\n`).

/// One `NAME;PARAMS:value` line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Property {
    pub name: String,
    pub params: Vec<(String, String)>,
    pub value: String,
}

impl Property {
    /// All type values (`TYPE=CELL,VOICE` and bare 2.1 `CELL`), upper-cased.
    pub fn types(&self) -> Vec<String> {
        self.params
            .iter()
            .filter(|(k, _)| k == "TYPE" || k.is_empty())
            .flat_map(|(_, v)| v.split(','))
            .map(|t| t.trim().to_ascii_uppercase())
            .filter(|t| !t.is_empty())
            .collect()
    }

    fn param(&self, key: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(k, v)| k == key || (k.is_empty() && v.eq_ignore_ascii_case(key)))
            .map(|(_, v)| v.as_str())
    }

    fn is_quoted_printable(&self) -> bool {
        self.param("ENCODING")
            .is_some_and(|v| v.eq_ignore_ascii_case("QUOTED-PRINTABLE"))
            || self.param("QUOTED-PRINTABLE").is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VCardPhone {
    pub number: String,
    /// CELL, WORK, HOME, FAX, MAIN or OTHER
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VCardEmail {
    pub address: String,
    pub kind: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VCard {
    pub formatted_name: Option<String>,
    pub family_name: Option<String>,
    pub given_name: Option<String>,
    pub organization: Option<String>,
    pub uid: Option<String>,
    pub phones: Vec<VCardPhone>,
    pub emails: Vec<VCardEmail>,
}

impl VCard {
    /// Best human-readable name: FN, then "Given Family", then organisation.
    pub fn display_name(&self) -> Option<String> {
        if let Some(name) = self.formatted_name.as_deref().map(str::trim)
            && !name.is_empty()
        {
            return Some(name.to_string());
        }
        let joined = [self.given_name.as_deref(), self.family_name.as_deref()]
            .into_iter()
            .flatten()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        if !joined.is_empty() {
            return Some(joined);
        }
        self.organization
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    }

    /// First phone number, if any.
    pub fn first_phone(&self) -> Option<&str> {
        self.phones.first().map(|p| p.number.as_str())
    }

    fn apply(&mut self, prop: &Property) {
        match prop.name.as_str() {
            "FN" => self.formatted_name = Some(unescape(&prop.value)),
            "N" => {
                let parts = split_unescaped(&prop.value, ';');
                self.family_name = parts.first().map(|s| unescape(s)).filter(|s| !s.is_empty());
                self.given_name = parts.get(1).map(|s| unescape(s)).filter(|s| !s.is_empty());
            }
            "ORG" => {
                let parts = split_unescaped(&prop.value, ';');
                self.organization = parts.first().map(|s| unescape(s)).filter(|s| !s.is_empty());
            }
            "UID" => self.uid = Some(prop.value.trim().to_string()).filter(|s| !s.is_empty()),
            "TEL" => {
                let number = prop
                    .value
                    .trim()
                    .trim_start_matches("tel:")
                    .trim()
                    .to_string();
                if !number.is_empty() {
                    self.phones.push(VCardPhone {
                        number,
                        kind: phone_kind(&prop.types()),
                    });
                }
            }
            "EMAIL" => {
                let address = prop.value.trim().to_string();
                if !address.is_empty() {
                    self.emails.push(VCardEmail {
                        address,
                        kind: email_kind(&prop.types()),
                    });
                }
            }
            _ => {}
        }
    }

    /// Build a vCard from already unfolded property lines (without BEGIN/END).
    pub fn from_lines<'a>(lines: impl IntoIterator<Item = &'a str>) -> Self {
        let mut card = VCard::default();
        for line in lines {
            if let Some(prop) = parse_property(line) {
                card.apply(&prop);
            }
        }
        card
    }
}

fn phone_kind(types: &[String]) -> String {
    for kind in ["CELL", "WORK", "HOME", "FAX", "MAIN"] {
        if types.iter().any(|t| t == kind) {
            return kind.to_string();
        }
    }
    if types.iter().any(|t| t == "IPHONE" || t == "MOBILE") {
        return "CELL".to_string();
    }
    "OTHER".to_string()
}

fn email_kind(types: &[String]) -> String {
    for kind in ["WORK", "HOME"] {
        if types.iter().any(|t| t == kind) {
            return kind.to_string();
        }
    }
    "OTHER".to_string()
}

/// Join folded lines (RFC 2425 folding and quoted-printable soft breaks).
pub fn unfold(data: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut qp_continues = false;
    for raw in data.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if qp_continues {
            if let Some(last) = out.last_mut() {
                last.pop(); // the soft-break '='
                last.push_str(line);
            }
        } else if (line.starts_with(' ') || line.starts_with('\t')) && !out.is_empty() {
            if let Some(last) = out.last_mut() {
                last.push_str(&line[1..]);
            }
        } else {
            out.push(line.to_string());
        }
        let current = out.last().map(String::as_str).unwrap_or("");
        qp_continues = current.ends_with('=') && header_is_quoted_printable(current);
    }
    out
}

fn header_is_quoted_printable(line: &str) -> bool {
    split_header(line)
        .map(|(header, _)| header.to_ascii_uppercase().contains("QUOTED-PRINTABLE"))
        .unwrap_or(false)
}

/// Split `NAME;PARAMS:value` at the first colon that is not inside a quoted parameter.
fn split_header(line: &str) -> Option<(&str, &str)> {
    let mut in_quotes = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_quotes = !in_quotes,
            ':' if !in_quotes => return Some((&line[..i], &line[i + 1..])),
            _ => {}
        }
    }
    None
}

/// Parse one unfolded content line.
pub fn parse_property(line: &str) -> Option<Property> {
    let (header, raw_value) = split_header(line)?;
    let mut parts = header.split(';');
    let name_part = parts.next()?.trim();
    // Strip the group prefix ("item1.TEL" -> "TEL")
    let name = name_part
        .rsplit('.')
        .next()
        .unwrap_or(name_part)
        .to_ascii_uppercase();
    if name.is_empty() {
        return None;
    }
    let params = parts
        .filter(|p| !p.is_empty())
        .map(|p| match p.split_once('=') {
            Some((k, v)) => (
                k.trim().to_ascii_uppercase(),
                v.trim().trim_matches('"').to_string(),
            ),
            None => (String::new(), p.trim().to_string()),
        })
        .collect();
    let mut prop = Property {
        name,
        params,
        value: raw_value.to_string(),
    };
    if prop.is_quoted_printable() {
        prop.value = decode_quoted_printable(&prop.value);
    }
    Some(prop)
}

/// Decode quoted-printable text, assuming UTF-8 bytes.
pub fn decode_quoted_printable(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'='
            && let Some(hex) = s.get(i + 1..i + 3)
            && hex.bytes().all(|b| b.is_ascii_hexdigit())
            && let Ok(b) = u8::from_str_radix(hex, 16)
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Undo vCard 3.0 value escaping.
pub fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n' | 'N') => out.push('\n'),
                Some(other) => out.push(other),
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out.trim().to_string()
}

/// Split on `sep` unless it is escaped with a backslash.
fn split_unescaped(s: &str, sep: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut escaped = false;
    for c in s.chars() {
        if escaped {
            current.push('\\');
            current.push(c);
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == sep {
            parts.push(std::mem::take(&mut current));
        } else {
            current.push(c);
        }
    }
    if escaped {
        current.push('\\');
    }
    parts.push(current);
    parts
}

/// Parse every `BEGIN:VCARD` … `END:VCARD` block in a phonebook dump.
pub fn parse_vcards(data: &str) -> Vec<VCard> {
    let mut cards = Vec::new();
    let mut current: Option<Vec<String>> = None;
    for line in unfold(data) {
        let upper = line.trim().to_ascii_uppercase();
        if upper == "BEGIN:VCARD" {
            current = Some(Vec::new());
        } else if upper == "END:VCARD" {
            if let Some(lines) = current.take() {
                cards.push(VCard::from_lines(lines.iter().map(String::as_str)));
            }
        } else if let Some(lines) = current.as_mut() {
            lines.push(line);
        }
    }
    cards
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic_v30_cards() {
        let data = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:John Doe\r\nN:Doe;John;;;\r\n\
                    TEL;TYPE=CELL:+15551234567\r\nEND:VCARD\r\n\
                    BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Jane Smith\r\nTEL:+15559876543\r\nEND:VCARD\r\n";
        let cards = parse_vcards(data);
        assert_eq!(cards.len(), 2);
        assert_eq!(cards[0].display_name().as_deref(), Some("John Doe"));
        assert_eq!(cards[0].family_name.as_deref(), Some("Doe"));
        assert_eq!(cards[0].given_name.as_deref(), Some("John"));
        assert_eq!(cards[0].phones[0].number, "+15551234567");
        assert_eq!(cards[0].phones[0].kind, "CELL");
        assert_eq!(cards[1].phones[0].kind, "OTHER");
    }

    #[test]
    fn parses_ios_style_params_and_groups() {
        let data = "BEGIN:VCARD\nVERSION:3.0\nN;CHARSET=UTF-8:Kowalski;Jan;;;\n\
                    FN;CHARSET=UTF-8:Jan Kowalski\nitem1.TEL;type=pref;type=CELL:512 345 678\n\
                    TEL;type=WORK;type=VOICE:+48 22 123 45 67\n\
                    item2.EMAIL;type=INTERNET;type=HOME:jan@example.com\nEND:VCARD\n";
        let card = &parse_vcards(data)[0];
        assert_eq!(card.display_name().as_deref(), Some("Jan Kowalski"));
        assert_eq!(card.phones.len(), 2);
        assert_eq!(card.phones[0].number, "512 345 678");
        assert_eq!(card.phones[0].kind, "CELL");
        assert_eq!(card.phones[1].kind, "WORK");
        assert_eq!(card.emails[0].address, "jan@example.com");
        assert_eq!(card.emails[0].kind, "HOME");
    }

    #[test]
    fn parses_v21_bare_types_and_quoted_printable() {
        let data = "BEGIN:VCARD\r\nVERSION:2.1\r\n\
                    N;CHARSET=UTF-8;ENCODING=QUOTED-PRINTABLE:=C5=81ukasz;=C5=BBaneta\r\n\
                    FN;ENCODING=QUOTED-PRINTABLE;CHARSET=UTF-8:=C5=BBaneta =C5=81uka=\r\nsz\r\n\
                    TEL;CELL;VOICE:512345678\r\nEND:VCARD\r\n";
        let card = &parse_vcards(data)[0];
        assert_eq!(card.family_name.as_deref(), Some("Łukasz"));
        assert_eq!(card.given_name.as_deref(), Some("Żaneta"));
        assert_eq!(card.formatted_name.as_deref(), Some("Żaneta Łukasz"));
        assert_eq!(card.phones[0].kind, "CELL");
    }

    #[test]
    fn unfolds_continuation_lines() {
        let data = "BEGIN:VCARD\r\nFN:Very Long\r\n  Name\r\nEND:VCARD\r\n";
        assert_eq!(
            parse_vcards(data)[0].formatted_name.as_deref(),
            Some("Very Long Name")
        );
    }

    #[test]
    fn unescapes_values() {
        let data = "BEGIN:VCARD\nFN:Smith\\, John\nN:Smith\\;Jr;John\nEND:VCARD\n";
        let card = &parse_vcards(data)[0];
        assert_eq!(card.formatted_name.as_deref(), Some("Smith, John"));
        assert_eq!(card.family_name.as_deref(), Some("Smith;Jr"));
        assert_eq!(card.given_name.as_deref(), Some("John"));
    }

    #[test]
    fn display_name_fallbacks() {
        let card = VCard {
            given_name: Some("Jan".into()),
            family_name: Some("Kowalski".into()),
            ..Default::default()
        };
        assert_eq!(card.display_name().as_deref(), Some("Jan Kowalski"));
        let org = VCard {
            organization: Some("ACME".into()),
            ..Default::default()
        };
        assert_eq!(org.display_name().as_deref(), Some("ACME"));
        assert_eq!(VCard::default().display_name(), None);
    }

    #[test]
    fn empty_and_garbage_input() {
        assert!(parse_vcards("").is_empty());
        assert!(parse_vcards("not a vcard at all").is_empty());
        // Unterminated card is dropped
        assert!(parse_vcards("BEGIN:VCARD\nFN:X\n").is_empty());
    }

    #[test]
    fn tel_uri_values() {
        let data =
            "BEGIN:VCARD\nVERSION:4.0\nTEL;VALUE=uri;TYPE=cell:tel:+48512345678\nEND:VCARD\n";
        assert_eq!(parse_vcards(data)[0].phones[0].number, "+48512345678");
    }

    #[test]
    fn quoted_printable_decoding() {
        assert_eq!(decode_quoted_printable("=C5=81=C3=B3d=C5=BA"), "Łódź");
        assert_eq!(decode_quoted_printable("plain"), "plain");
        assert_eq!(decode_quoted_printable("bad=ZZ"), "bad=ZZ");
        assert_eq!(decode_quoted_printable("end="), "end=");
    }

    #[test]
    fn property_parsing() {
        let p = parse_property("item1.TEL;TYPE=\"CELL,VOICE\":+1 555").unwrap();
        assert_eq!(p.name, "TEL");
        assert_eq!(p.types(), vec!["CELL", "VOICE"]);
        assert_eq!(p.value, "+1 555");
        assert!(parse_property("no colon here").is_none());
    }
}
