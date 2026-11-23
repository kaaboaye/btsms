//! Message reactions sent as plain SMS.
//!
//! iPhones send Tapbacks (and iOS 18 emoji reactions) to SMS recipients as ordinary texts such
//! as `Liked “see you at 5”` or `Reacted 😂 to “see you at 5”`, and removing one sends
//! `Removed a like from “…”`. Google Messages uses the same wording when it falls back to SMS.
//! This module recognises those texts and attaches them to the message they quote, the way
//! Google Messages shows them, without changing what is stored.

use crate::db::{Message, MessageDirection};

/// A reaction parsed from a message body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reaction {
    pub emoji: String,
    pub target: ReactionTarget,
    /// The sender took the reaction back.
    pub removed: bool,
    /// The text started with the reactor's name (`Jan liked “…”`). Such texts are only
    /// treated as reactions when the quoted message is found, so that an ordinary sentence
    /// like `I liked “Dune”` stays a message.
    pub named: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReactionTarget {
    /// The text of the message reacted to, as quoted (may end with "…" if shortened).
    Quote(String),
    /// A picture or other attachment, e.g. "an image".
    Attachment(String),
}

/// Classic Tapbacks: verb prefix as sent in the SMS, and the emoji it stands for.
const TAPBACKS: &[(&str, &str)] = &[
    ("Loved ", "❤️"),
    ("Liked ", "👍"),
    ("Disliked ", "👎"),
    ("Laughed at ", "😂"),
    ("Emphasized ", "‼️"),
    ("Emphasised ", "‼️"),
    ("Questioned ", "❓"),
];

/// `Removed <noun> from …`. Longer nouns first ("an exclamation mark" before "an exclamation").
const REMOVED_TAPBACKS: &[(&str, &str)] = &[
    ("an exclamation mark", "‼️"),
    ("an exclamation", "‼️"),
    ("an emphasis", "‼️"),
    ("a question mark", "❓"),
    ("a heart", "❤️"),
    ("a like", "👍"),
    ("a dislike", "👎"),
    ("a laugh", "😂"),
];

const ATTACHMENTS: &[&str] = &[
    "an image",
    "a photo",
    "a video",
    "a movie",
    "an attachment",
    "an audio message",
    "a sticker",
    "a GIF",
    "a link",
];

/// Opening and closing quote pairs.
const QUOTES: &[(char, char)] = &[('“', '”'), ('„', '”'), ('"', '"')];

/// Invisible characters some senders put around the quote. Not U+200D (ZWJ), which is part
/// of emoji sequences.
fn is_invisible(c: char) -> bool {
    matches!(c, '\u{200B}' | '\u{2060}' | '\u{FEFF}')
}

/// Verbs as they follow a name: `Jan laughed at “…”`. The iPhone sends this form in some MAP
/// sessions (with an empty `Sender`), instead of `Laughed at “…”`.
const NAMED_VERBS: &[(&str, &str)] = &[
    (" loved ", "Loved "),
    (" liked ", "Liked "),
    (" disliked ", "Disliked "),
    (" laughed at ", "Laughed at "),
    (" emphasized ", "Emphasized "),
    (" emphasised ", "Emphasised "),
    (" questioned ", "Questioned "),
    (" reacted ", "Reacted "),
    (" removed ", "Removed "),
];

/// Parse a message body that is a reaction.
pub fn parse_reaction(body: &str) -> Option<Reaction> {
    let cleaned: String = body.chars().filter(|&c| !is_invisible(c)).collect();
    let body = cleaned.trim();
    parse_unnamed(body).or_else(|| parse_named(body))
}

fn parse_named(body: &str) -> Option<Reaction> {
    NAMED_VERBS.iter().find_map(|(verb, capitalized)| {
        let at = body.find(verb)?;
        let name = &body[..at];
        let plausible_name = !name.trim().is_empty()
            && name.chars().count() <= 40
            && name.split_whitespace().count() <= 4
            && !name
                .chars()
                .any(|c| QUOTES.iter().any(|(o, e)| c == *o || c == *e));
        if !plausible_name {
            return None;
        }
        let reaction = parse_unnamed(&format!("{capitalized}{}", &body[at + verb.len()..]))?;
        Some(Reaction {
            named: true,
            ..reaction
        })
    })
}

fn parse_unnamed(body: &str) -> Option<Reaction> {
    for (prefix, emoji) in TAPBACKS {
        if let Some(rest) = body.strip_prefix(prefix) {
            return Some(Reaction {
                emoji: (*emoji).to_string(),
                target: parse_target(rest)?,
                removed: false,
                named: false,
            });
        }
    }
    if let Some(rest) = body.strip_prefix("Reacted ") {
        let (emoji, target) = rest.split_once(" to ")?;
        return Some(Reaction {
            emoji: emoji_token(emoji)?,
            target: parse_target(target)?,
            removed: false,
            named: false,
        });
    }
    if let Some(rest) = body.strip_prefix("Removed ") {
        for (noun, emoji) in REMOVED_TAPBACKS {
            if let Some(target) = rest
                .strip_prefix(noun)
                .and_then(|r| r.strip_prefix(" from "))
            {
                return Some(Reaction {
                    emoji: (*emoji).to_string(),
                    target: parse_target(target)?,
                    removed: true,
                    named: false,
                });
            }
        }
        let (emoji, target) = rest.split_once(" from ")?;
        let emoji = emoji
            .strip_prefix("a ")
            .or_else(|| emoji.strip_prefix("an "))
            .unwrap_or(emoji);
        return Some(Reaction {
            emoji: emoji_token(emoji)?,
            target: parse_target(target)?,
            removed: true,
            named: false,
        });
    }
    None
}

fn parse_target(s: &str) -> Option<ReactionTarget> {
    for (open, close) in QUOTES {
        if let Some(inner) = s.strip_prefix(*open).and_then(|r| r.strip_suffix(*close)) {
            return (!inner.trim().is_empty()).then(|| ReactionTarget::Quote(inner.to_string()));
        }
    }
    ATTACHMENTS
        .iter()
        .find(|a| **a == s)
        .map(|a| ReactionTarget::Attachment((*a).to_string()))
}

/// Accept a single emoji (possibly a ZWJ sequence), not a word.
fn emoji_token(s: &str) -> Option<String> {
    let ok = !s.is_empty()
        && s.chars().count() <= 12
        && !s
            .chars()
            .any(|c| c.is_whitespace() || c.is_alphanumeric() || c.is_ascii())
        && s.chars().any(|c| c as u32 >= 0x2000);
    ok.then(|| s.to_string())
}

impl Reaction {
    /// Short human-readable form, e.g. `Reacted 😂 to “see you”`.
    pub fn describe(&self) -> String {
        let target = match &self.target {
            ReactionTarget::Quote(q) => format!("“{}”", shorten(q, 60)),
            ReactionTarget::Attachment(a) => a.clone(),
        };
        if self.removed {
            format!("Removed {} from {target}", self.emoji)
        } else {
            format!("Reacted {} to {target}", self.emoji)
        }
    }
}

/// Text to show for a message body in previews and notifications. The named form
/// (`Jan liked “…”`) is left as it is: it reads well and might be an ordinary sentence.
pub fn display_text(body: &str) -> String {
    parse_reaction(body)
        .filter(|r| !r.named)
        .map(|r| r.describe())
        .unwrap_or_else(|| body.to_string())
}

fn shorten(s: &str, max_chars: usize) -> String {
    let s = collapse_whitespace(s);
    if s.chars().count() <= max_chars {
        return s;
    }
    let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    out.truncate(out.trim_end().len());
    out.push('…');
    out
}

fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace()
        .map(|w| w.chars().filter(|&c| !is_invisible(c)).collect::<String>())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Does a quote from a reaction refer to `body`? Whitespace differences are ignored, and a
/// quote ending in an ellipsis matches the start of the text.
pub fn quote_matches(quote: &str, body: &str) -> bool {
    let quote = collapse_whitespace(quote);
    let body = collapse_whitespace(body);
    if quote.is_empty() {
        return false;
    }
    if quote == body {
        return true;
    }
    let prefix = quote
        .strip_suffix('…')
        .or_else(|| quote.strip_suffix("..."))
        .map(str::trim_end);
    prefix.is_some_and(|p| p.chars().count() >= 3 && body.starts_with(p))
}

/// One reaction shown on a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReactionMark {
    pub emoji: String,
    /// Who reacted: `Incoming` is the other party, `Outgoing` is the user.
    pub from: MessageDirection,
}

/// A row of a conversation once reactions are attached to their messages.
#[derive(Debug, Clone, PartialEq)]
pub enum ThreadItem<'a> {
    Message {
        message: &'a Message,
        reactions: Vec<ReactionMark>,
    },
    /// A message that is not stored, rebuilt from the quote in a reaction to it. Typically a
    /// text the user sent from the iPhone itself, which MAP never lists. Shown where the
    /// first reaction to it arrived.
    Quoted {
        /// The reaction the text was taken from.
        reaction_message: &'a Message,
        text: String,
        direction: MessageDirection,
        reactions: Vec<ReactionMark>,
    },
    /// A reaction that cannot be attached to anything: to an attachment, or a removal of a
    /// reaction to a message that is not loaded.
    Unmatched {
        message: &'a Message,
        reaction: Reaction,
    },
}

impl ThreadItem<'_> {
    fn quoted_by(&self, quote: &str) -> bool {
        match self {
            Self::Message { message, .. } => quote_matches(quote, &message.body),
            Self::Quoted { text, .. } => quote_matches(quote, text),
            Self::Unmatched { .. } => false,
        }
    }

    fn reactions_mut(&mut self) -> Option<&mut Vec<ReactionMark>> {
        match self {
            Self::Message { reactions, .. } | Self::Quoted { reactions, .. } => Some(reactions),
            Self::Unmatched { .. } => None,
        }
    }
}

fn opposite(direction: MessageDirection) -> MessageDirection {
    match direction {
        MessageDirection::Incoming => MessageDirection::Outgoing,
        MessageDirection::Outgoing => MessageDirection::Incoming,
    }
}

/// The same emoji with and without the emoji variation selector compare equal.
fn same_emoji(a: &str, b: &str) -> bool {
    a.chars()
        .filter(|&c| c != '\u{FE0F}')
        .eq(b.chars().filter(|&c| c != '\u{FE0F}'))
}

/// Attach reactions in `messages` (display order, oldest first) to the newest earlier message
/// they quote. Each person has one reaction per message, so a new one replaces theirs, as
/// changing a Tapback on the iPhone does. A reaction to a message that is not loaded brings
/// the quoted text back as a [`ThreadItem::Quoted`] bubble on the other person's side
/// (people react to each other's messages).
pub fn build_thread(messages: &[Message]) -> Vec<ThreadItem<'_>> {
    let mut items: Vec<ThreadItem<'_>> = Vec::with_capacity(messages.len());
    for m in messages {
        let Some(reaction) = parse_reaction(&m.body) else {
            items.push(ThreadItem::Message {
                message: m,
                reactions: Vec::new(),
            });
            continue;
        };
        let target = match &reaction.target {
            ReactionTarget::Quote(q) => items.iter().rposition(|item| item.quoted_by(q)),
            ReactionTarget::Attachment(_) => None,
        };
        if reaction.named && target.is_none() {
            items.push(ThreadItem::Message {
                message: m,
                reactions: Vec::new(),
            });
            continue;
        }
        let mark = ReactionMark {
            emoji: reaction.emoji.clone(),
            from: m.direction,
        };
        match (
            target.and_then(|i| items[i].reactions_mut()),
            &reaction.target,
        ) {
            (Some(reactions), _) => {
                if reaction.removed {
                    reactions
                        .retain(|r| !(r.from == m.direction && same_emoji(&r.emoji, &mark.emoji)));
                } else {
                    reactions.retain(|r| r.from != m.direction);
                    reactions.push(mark);
                }
            }
            (None, ReactionTarget::Quote(text)) if !reaction.removed => {
                items.push(ThreadItem::Quoted {
                    reaction_message: m,
                    text: text.clone(),
                    direction: opposite(m.direction),
                    reactions: vec![mark],
                });
            }
            (None, _) => items.push(ThreadItem::Unmatched {
                message: m,
                reaction,
            }),
        }
    }
    items
}

/// Text of the reaction chip under a bubble: the distinct emoji, then the count if more than
/// one person reacted. Empty when there are no reactions.
pub fn chip_text(reactions: &[ReactionMark]) -> String {
    let mut distinct: Vec<&str> = Vec::new();
    for r in reactions {
        if !distinct.iter().any(|e| same_emoji(e, &r.emoji)) {
            distinct.push(&r.emoji);
        }
    }
    let mut text = distinct.concat();
    if reactions.len() > 1 {
        text.push_str(&format!(" {}", reactions.len()));
    }
    text
}

/// Tooltip for the chip, e.g. "Them: 😂\nYou: 👍".
pub fn chip_tooltip(reactions: &[ReactionMark]) -> String {
    reactions
        .iter()
        .map(|r| match r.from {
            MessageDirection::Incoming => format!("Them: {}", r.emoji),
            MessageDirection::Outgoing => format!("You: {}", r.emoji),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quote(emoji: &str, q: &str, removed: bool) -> Option<Reaction> {
        Some(Reaction {
            emoji: emoji.into(),
            target: ReactionTarget::Quote(q.into()),
            removed,
            named: false,
        })
    }

    fn message(id: i64, body: &str, direction: MessageDirection) -> Message {
        Message {
            id,
            peer: "+48512345678".into(),
            sender_number: None,
            sender_name: None,
            recipient_number: None,
            body: body.into(),
            received_at: format!("2026-11-23T13:{:02}:00Z", id % 60),
            direction,
            read_status: true,
        }
    }

    use MessageDirection::{Incoming, Outgoing};

    #[test]
    fn classic_tapbacks() {
        assert_eq!(
            parse_reaction("Liked “see you at 5”"),
            quote("👍", "see you at 5", false)
        );
        assert_eq!(parse_reaction("Loved “ok”"), quote("❤️", "ok", false));
        assert_eq!(parse_reaction("Disliked “ok”"), quote("👎", "ok", false));
        assert_eq!(
            parse_reaction("Laughed at “a joke xd”"),
            quote("😂", "a joke xd", false)
        );
        assert_eq!(parse_reaction("Emphasized “ok”"), quote("‼️", "ok", false));
        assert_eq!(parse_reaction("Questioned “ok”"), quote("❓", "ok", false));
    }

    #[test]
    fn emoji_reactions_and_quote_styles() {
        assert_eq!(
            parse_reaction("Reacted 🙏 to “thanks to you”"),
            quote("🙏", "thanks to you", false)
        );
        // ZWJ sequence, straight quotes, Polish quotes, zero-width spaces around the quote
        assert_eq!(
            parse_reaction("Reacted 🤦‍♂️ to \"no\""),
            quote("🤦‍♂️", "no", false)
        );
        assert_eq!(parse_reaction("Liked „tak”"), quote("👍", "tak", false));
        assert_eq!(
            parse_reaction("Liked \u{200B}“tak”\u{200B}"),
            quote("👍", "tak", false)
        );
    }

    #[test]
    fn removals() {
        assert_eq!(
            parse_reaction("Removed a like from “ok”"),
            quote("👍", "ok", true)
        );
        assert_eq!(
            parse_reaction("Removed a heart from “ok”"),
            quote("❤️", "ok", true)
        );
        assert_eq!(
            parse_reaction("Removed an exclamation mark from “ok”"),
            quote("‼️", "ok", true)
        );
        assert_eq!(
            parse_reaction("Removed a question mark from “ok”"),
            quote("❓", "ok", true)
        );
        assert_eq!(
            parse_reaction("Removed 😂 from “take it from me”"),
            quote("😂", "take it from me", true)
        );
        assert_eq!(
            parse_reaction("Removed a 😂 from “ok”"),
            quote("😂", "ok", true)
        );
    }

    #[test]
    fn attachments() {
        assert_eq!(
            parse_reaction("Liked an image"),
            Some(Reaction {
                emoji: "👍".into(),
                target: ReactionTarget::Attachment("an image".into()),
                removed: false,
                named: false,
            })
        );
        assert_eq!(
            display_text("Loved a photo"),
            "Reacted ❤️ to a photo".to_string()
        );
    }

    #[test]
    fn reactions_prefixed_with_the_reactors_name() {
        let named = |emoji: &str, q: &str, removed: bool| {
            Some(Reaction {
                named: true,
                ..quote(emoji, q, removed).unwrap()
            })
        };
        assert_eq!(
            parse_reaction("Jan laughed at “więc pekao odrzuciło zwrot xd”"),
            named("😂", "więc pekao odrzuciło zwrot xd", false)
        );
        assert_eq!(
            parse_reaction("Jan Kowalski liked “ok”"),
            named("👍", "ok", false)
        );
        assert_eq!(
            parse_reaction("Jan reacted 🙏 to “ok”"),
            named("🙏", "ok", false)
        );
        assert_eq!(
            parse_reaction("Jan removed a heart from “ok”"),
            named("❤️", "ok", true)
        );
        // A sentence, not a name, before the verb
        assert_eq!(
            parse_reaction("Yesterday at the cinema my friend Jan liked “Dune”"),
            None
        );
        assert_eq!(parse_reaction("Jan liked it"), None);
    }

    #[test]
    fn named_form_counts_only_when_the_quoted_message_is_there() {
        let msgs = [
            message(1, "ok", Outgoing),
            message(2, "Jan laughed at “ok”", Incoming),
            message(3, "I liked “Dune”", Incoming),
        ];
        let thread = build_thread(&msgs);
        assert_eq!(thread.len(), 2);
        let ThreadItem::Message { reactions, .. } = &thread[0] else {
            panic!()
        };
        assert_eq!(chip_text(reactions), "😂");
        assert!(
            matches!(thread[1], ThreadItem::Message { message, .. } if message.id == 3),
            "{:?}",
            thread[1]
        );
    }

    #[test]
    fn ordinary_texts_are_not_reactions() {
        for body in [
            "Liked it a lot",
            "Liked “unterminated",
            "Liked “”",
            "Reacted badly to “news”",
            "Reacted 😂 “no target word”",
            "Removed the files from “disk”",
            "Removed from “list”",
            "I Liked “this”",
            "Loved",
            "",
        ] {
            assert_eq!(parse_reaction(body), None, "{body}");
        }
    }

    #[test]
    fn describe_shortens_long_quotes() {
        assert_eq!(
            display_text("Laughed at “więc pekao odrzuciło zwrot xd”"),
            "Reacted 😂 to “więc pekao odrzuciło zwrot xd”"
        );
        assert_eq!(
            display_text("Removed a like from “x  y”"),
            "Removed 👍 from “x y”"
        );
        let long = format!("Liked “{}”", "a".repeat(100));
        let shown = display_text(&long);
        assert!(shown.ends_with("…”"), "{shown}");
        assert_eq!(
            shown.chars().count(),
            "Reacted 👍 to “”".chars().count() + 60
        );
        assert_eq!(display_text("plain text"), "plain text");
        assert_eq!(display_text("I liked “Dune”"), "I liked “Dune”");
    }

    #[test]
    fn quotes_match_with_whitespace_and_ellipsis() {
        assert!(quote_matches("see you", "see you"));
        assert!(quote_matches("see you", "  see\nyou "));
        assert!(quote_matches("see you at…", "see you at 5"));
        assert!(quote_matches("see you at...", "see you at 5"));
        assert!(!quote_matches("see you", "see you at 5"));
        assert!(!quote_matches("…", "anything"));
        assert!(!quote_matches("other…", "see you"));
    }

    #[test]
    fn reactions_attach_to_the_newest_quoted_message() {
        let msgs = [
            message(1, "ok", Outgoing),
            message(2, "hi", Incoming),
            message(3, "ok", Outgoing),
            message(4, "Liked “ok”", Incoming),
            message(5, "Laughed at “hi”", Outgoing),
        ];
        let thread = build_thread(&msgs);
        let shown: Vec<(i64, Vec<(&str, MessageDirection)>)> = thread
            .iter()
            .map(|item| match item {
                ThreadItem::Message { message, reactions } => (
                    message.id,
                    reactions
                        .iter()
                        .map(|r| (r.emoji.as_str(), r.from))
                        .collect(),
                ),
                other => panic!("not a stored message: {other:?}"),
            })
            .collect();
        assert_eq!(
            shown,
            vec![
                (1, vec![]),
                (2, vec![("😂", Outgoing)]),
                (3, vec![("👍", Incoming)]),
            ]
        );
    }

    #[test]
    fn new_reaction_replaces_and_removal_deletes() {
        let msgs = [
            message(1, "ok", Outgoing),
            message(2, "Liked “ok”", Incoming),
            message(3, "Loved “ok”", Incoming),
            message(4, "Reacted 👍 to “ok”", Outgoing),
        ];
        let ThreadItem::Message { reactions, .. } = &build_thread(&msgs)[0] else {
            panic!()
        };
        assert_eq!(chip_text(reactions), "❤️👍 2");
        assert_eq!(chip_tooltip(reactions), "Them: ❤️\nYou: 👍");

        let msgs = [
            message(1, "ok", Outgoing),
            message(2, "Loved “ok”", Incoming),
            // removal text without the variation selector still removes ❤️
            message(3, "Removed ❤ from “ok”", Incoming),
        ];
        let thread = build_thread(&msgs);
        assert_eq!(thread.len(), 1);
        let ThreadItem::Message { reactions, .. } = &thread[0] else {
            panic!()
        };
        assert!(reactions.is_empty());
        assert_eq!(chip_text(reactions), "");
    }

    #[test]
    fn missing_target_is_rebuilt_from_the_quote() {
        let msgs = [
            message(1, "hi", Incoming),
            message(2, "Loved “sent from the phone itself”", Incoming),
            message(3, "Laughed at “sent from the phone itself”", Incoming),
            message(4, "Reacted 👍 to “sent from the phone itself”", Outgoing),
            message(5, "Liked “a text of theirs we don't have”", Outgoing),
        ];
        let thread = build_thread(&msgs);
        assert_eq!(thread.len(), 3);
        let ThreadItem::Quoted {
            reaction_message,
            text,
            direction,
            reactions,
        } = &thread[1]
        else {
            panic!("{:?}", thread[1])
        };
        assert_eq!(reaction_message.id, 2);
        assert_eq!(text, "sent from the phone itself");
        assert_eq!(*direction, Outgoing, "they reacted, so it was my message");
        assert_eq!(chip_text(reactions), "😂👍 2");
        let ThreadItem::Quoted { direction, .. } = &thread[2] else {
            panic!("{:?}", thread[2])
        };
        assert_eq!(*direction, Incoming);
    }

    #[test]
    fn reactions_that_cannot_be_attached_stay_visible() {
        let msgs = [
            message(1, "Removed a heart from “an older message”", Incoming),
            message(2, "Liked an image", Incoming),
            // reactions never attach to other reactions
            message(
                3,
                "Liked “Removed a heart from “an older message””",
                Incoming,
            ),
        ];
        let thread = build_thread(&msgs);
        assert_eq!(thread.len(), 3);
        assert!(matches!(thread[0], ThreadItem::Unmatched { .. }));
        assert!(matches!(thread[1], ThreadItem::Unmatched { .. }));
        assert!(matches!(thread[2], ThreadItem::Quoted { .. }));
    }

    #[test]
    fn chip_counts_people_not_emoji() {
        let same = [
            ReactionMark {
                emoji: "❤️".into(),
                from: Incoming,
            },
            ReactionMark {
                emoji: "❤".into(),
                from: Outgoing,
            },
        ];
        assert_eq!(chip_text(&same), "❤️ 2");
        assert_eq!(chip_text(&same[..1]), "❤️");
    }
}
