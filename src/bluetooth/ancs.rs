//! ANCS (Apple Notification Center Service) protocol logic, independent of transport.
//!
//! The iPhone is the GATT server. We subscribe to Notification Source and Data Source, and
//! for every new notification ask the Control Point for its attributes. The Control Point
//! accepts one request at a time (a second write fails with `InProgress`) and Data Source
//! replies may arrive split over several GATT notifications, so requests are queued and
//! replies reassembled here.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

pub const SERVICE_UUID: &str = "7905f431-b5ce-4e99-a40f-4b1e122d00d0";
pub const NOTIFICATION_SOURCE_UUID: &str = "9fbf120d-6301-42d9-8c58-25e699a21dbd";
pub const CONTROL_POINT_UUID: &str = "69d1d8f3-45e1-49a8-9821-9bbdfdaad9d9";
pub const DATA_SOURCE_UUID: &str = "22eac6e9-24d6-4bb5-be44-b36ace7c7bfb";

/// Bundle id of the Messages app (SMS and iMessage).
pub const MESSAGES_APP_ID: &str = "com.apple.MobileSMS";

/// A reply that never completes is dropped after this long so the queue keeps moving.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

const CMD_GET_NOTIFICATION_ATTRIBUTES: u8 = 0;

const ATTR_APP_IDENTIFIER: u8 = 0;
const ATTR_TITLE: u8 = 1;
const ATTR_MESSAGE: u8 = 3;
const ATTR_DATE: u8 = 5;
const TITLE_MAX_LEN: u16 = 128;
const MESSAGE_MAX_LEN: u16 = 1024;
/// Attributes requested for every notification, in request order.
const REQUESTED_ATTRIBUTES: [u8; 4] = [ATTR_APP_IDENTIFIER, ATTR_TITLE, ATTR_MESSAGE, ATTR_DATE];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Other,
    IncomingCall,
    MissedCall,
    Voicemail,
    Social,
    Schedule,
    Email,
    News,
    HealthAndFitness,
    BusinessAndFinance,
    Location,
    Entertainment,
    Unknown(u8),
}

impl From<u8> for Category {
    fn from(v: u8) -> Self {
        match v {
            0 => Self::Other,
            1 => Self::IncomingCall,
            2 => Self::MissedCall,
            3 => Self::Voicemail,
            4 => Self::Social,
            5 => Self::Schedule,
            6 => Self::Email,
            7 => Self::News,
            8 => Self::HealthAndFitness,
            9 => Self::BusinessAndFinance,
            10 => Self::Location,
            11 => Self::Entertainment,
            other => Self::Unknown(other),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    Added,
    Modified,
    Removed,
    Unknown(u8),
}

impl From<u8> for EventKind {
    fn from(v: u8) -> Self {
        match v {
            0 => Self::Added,
            1 => Self::Modified,
            2 => Self::Removed,
            other => Self::Unknown(other),
        }
    }
}

/// A Notification Source event (8 bytes on the wire).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceEvent {
    pub kind: EventKind,
    pub flags: u8,
    pub category: Category,
    pub category_count: u8,
    pub uid: u32,
}

impl SourceEvent {
    pub const FLAG_SILENT: u8 = 1 << 0;
    pub const FLAG_IMPORTANT: u8 = 1 << 1;
    pub const FLAG_PRE_EXISTING: u8 = 1 << 2;

    pub fn parse(data: &[u8]) -> Option<Self> {
        if data.len() < 8 {
            return None;
        }
        Some(Self {
            kind: EventKind::from(data[0]),
            flags: data[1],
            category: Category::from(data[2]),
            category_count: data[3],
            uid: u32::from_le_bytes([data[4], data[5], data[6], data[7]]),
        })
    }

    /// Notifications that were already on the phone when we subscribed.
    pub fn is_pre_existing(&self) -> bool {
        self.flags & Self::FLAG_PRE_EXISTING != 0
    }
}

/// A notification with its attributes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AncsNotification {
    pub uid: u32,
    pub category: Category,
    pub app_identifier: Option<String>,
    pub title: Option<String>,
    pub message: Option<String>,
    /// `yyyyMMdd'T'HHmmSS` in the phone's local time.
    pub date: Option<String>,
}

impl AncsNotification {
    /// SMS or iMessage from the Messages app.
    pub fn is_message(&self) -> bool {
        self.app_identifier.as_deref() == Some(MESSAGES_APP_ID)
    }
}

/// Build a GetNotificationAttributes request.
pub fn build_attributes_request(uid: u32) -> Vec<u8> {
    let mut req = vec![CMD_GET_NOTIFICATION_ATTRIBUTES];
    req.extend_from_slice(&uid.to_le_bytes());
    for attr in REQUESTED_ATTRIBUTES {
        req.push(attr);
        match attr {
            ATTR_TITLE => req.extend_from_slice(&TITLE_MAX_LEN.to_le_bytes()),
            ATTR_MESSAGE => req.extend_from_slice(&MESSAGE_MAX_LEN.to_le_bytes()),
            _ => {}
        }
    }
    req
}

/// Outcome of parsing (possibly partial) Data Source bytes.
#[derive(Debug, PartialEq, Eq)]
pub enum ParseOutcome {
    NeedMore,
    Complete(AncsNotification),
    Invalid,
}

/// Parse a GetNotificationAttributes reply. Returns `NeedMore` while fragments are missing.
pub fn parse_attributes_reply(data: &[u8], category: Category) -> ParseOutcome {
    if data.len() < 5 {
        return ParseOutcome::NeedMore;
    }
    if data[0] != CMD_GET_NOTIFICATION_ATTRIBUTES {
        return ParseOutcome::Invalid;
    }
    let uid = u32::from_le_bytes([data[1], data[2], data[3], data[4]]);
    let mut n = AncsNotification {
        uid,
        category,
        app_identifier: None,
        title: None,
        message: None,
        date: None,
    };
    let mut offset = 5;
    let mut seen = 0;
    while seen < REQUESTED_ATTRIBUTES.len() {
        if offset + 3 > data.len() {
            return ParseOutcome::NeedMore;
        }
        let id = data[offset];
        let len = u16::from_le_bytes([data[offset + 1], data[offset + 2]]) as usize;
        offset += 3;
        if offset + len > data.len() {
            return ParseOutcome::NeedMore;
        }
        let value = String::from_utf8_lossy(&data[offset..offset + len]).into_owned();
        offset += len;
        seen += 1;
        let value = (!value.is_empty()).then_some(value);
        match id {
            ATTR_APP_IDENTIFIER => n.app_identifier = value,
            ATTR_TITLE => n.title = value,
            ATTR_MESSAGE => n.message = value,
            ATTR_DATE => n.date = value,
            _ => {}
        }
    }
    ParseOutcome::Complete(n)
}

#[derive(Debug)]
struct InFlight {
    uid: u32,
    category: Category,
    buf: Vec<u8>,
    sent_at: Instant,
}

/// Request queue and reply reassembly for one ANCS subscription.
#[derive(Debug, Default)]
pub struct AncsSession {
    queue: VecDeque<(u32, Category)>,
    in_flight: Option<InFlight>,
}

impl AncsSession {
    pub fn new() -> Self {
        Self::default()
    }

    /// Handle a Notification Source value. New notifications are queued for an attribute
    /// request; pre-existing ones (already on the phone at subscribe time) are skipped.
    pub fn on_notification_source(&mut self, data: &[u8]) -> Option<SourceEvent> {
        let event = SourceEvent::parse(data)?;
        if event.kind == EventKind::Added && !event.is_pre_existing() {
            if !self.queue.iter().any(|(uid, _)| *uid == event.uid) {
                self.queue.push_back((event.uid, event.category));
            }
        } else if event.kind == EventKind::Removed {
            self.queue.retain(|(uid, _)| *uid != event.uid);
        }
        Some(event)
    }

    /// The next Control Point write, if the Control Point is free.
    pub fn next_request(&mut self, now: Instant) -> Option<Vec<u8>> {
        if self.in_flight.is_some() {
            return None;
        }
        let (uid, category) = self.queue.pop_front()?;
        self.in_flight = Some(InFlight {
            uid,
            category,
            buf: Vec::new(),
            sent_at: now,
        });
        Some(build_attributes_request(uid))
    }

    /// Handle a Data Source value. Returns the notification once its reply is complete.
    pub fn on_data_source(&mut self, data: &[u8]) -> Option<AncsNotification> {
        let in_flight = self.in_flight.as_mut()?;
        in_flight.buf.extend_from_slice(data);
        match parse_attributes_reply(&in_flight.buf, in_flight.category) {
            ParseOutcome::NeedMore => None,
            ParseOutcome::Complete(n) => {
                let expected = in_flight.uid;
                self.in_flight = None;
                if n.uid == expected { Some(n) } else { None }
            }
            ParseOutcome::Invalid => {
                self.in_flight = None;
                None
            }
        }
    }

    /// The Control Point write failed: forget the request.
    pub fn request_failed(&mut self) {
        self.in_flight = None;
    }

    /// Drop a request whose reply never completed. Returns the abandoned uid.
    pub fn expire(&mut self, now: Instant) -> Option<u32> {
        let stale = self
            .in_flight
            .as_ref()
            .is_some_and(|f| now.duration_since(f.sent_at) > REQUEST_TIMEOUT);
        if stale {
            self.in_flight.take().map(|f| f.uid)
        } else {
            None
        }
    }

    pub fn pending(&self) -> usize {
        self.queue.len() + usize::from(self.in_flight.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(kind: u8, flags: u8, category: u8, uid: u32) -> Vec<u8> {
        let mut v = vec![kind, flags, category, 1];
        v.extend_from_slice(&uid.to_le_bytes());
        v
    }

    fn reply(uid: u32, attrs: &[(u8, &str)]) -> Vec<u8> {
        let mut v = vec![0];
        v.extend_from_slice(&uid.to_le_bytes());
        for (id, value) in attrs {
            v.push(*id);
            v.extend_from_slice(&(value.len() as u16).to_le_bytes());
            v.extend_from_slice(value.as_bytes());
        }
        v
    }

    fn sms_reply(uid: u32) -> Vec<u8> {
        reply(
            uid,
            &[
                (0, MESSAGES_APP_ID),
                (1, "Jan Kowalski"),
                (3, "Cześć, co tam?"),
                (5, "20261120T124531"),
            ],
        )
    }

    #[test]
    fn uuids_match_apple_spec() {
        // The previous implementation had invented UUIDs here, so ANCS never worked.
        assert_eq!(
            NOTIFICATION_SOURCE_UUID,
            "9fbf120d-6301-42d9-8c58-25e699a21dbd"
        );
        assert_eq!(DATA_SOURCE_UUID, "22eac6e9-24d6-4bb5-be44-b36ace7c7bfb");
        assert_eq!(CONTROL_POINT_UUID, "69d1d8f3-45e1-49a8-9821-9bbdfdaad9d9");
    }

    #[test]
    fn source_event_parsing() {
        let e = SourceEvent::parse(&source(0, 0b11101, 2, 0x01020304)).unwrap();
        assert_eq!(e.kind, EventKind::Added);
        assert_eq!(e.category, Category::MissedCall);
        assert_eq!(e.uid, 0x01020304);
        assert!(e.is_pre_existing());
        assert!(SourceEvent::parse(&[0, 0, 0]).is_none());
        assert_eq!(Category::from(200), Category::Unknown(200));
    }

    #[test]
    fn request_has_lengths_only_where_the_spec_wants_them() {
        let req = build_attributes_request(7);
        assert_eq!(
            req,
            vec![0, 7, 0, 0, 0, 0, 1, 128, 0, 3, 0x00, 0x04, 5],
            "AppIdentifier and Date take no max length; Title and Message do"
        );
    }

    #[test]
    fn full_flow_single_packet() {
        let mut s = AncsSession::new();
        let now = Instant::now();
        s.on_notification_source(&source(0, 0, 4, 42));
        let req = s.next_request(now).unwrap();
        assert_eq!(&req[1..5], &42u32.to_le_bytes());
        let n = s.on_data_source(&sms_reply(42)).unwrap();
        assert!(n.is_message());
        assert_eq!(n.title.as_deref(), Some("Jan Kowalski"));
        assert_eq!(n.message.as_deref(), Some("Cześć, co tam?"));
        assert_eq!(n.date.as_deref(), Some("20261120T124531"));
        assert_eq!(n.category, Category::Social);
        assert_eq!(s.pending(), 0);
    }

    #[test]
    fn fragmented_reply_is_reassembled() {
        let mut s = AncsSession::new();
        s.on_notification_source(&source(0, 0, 4, 9));
        s.next_request(Instant::now()).unwrap();
        let data = sms_reply(9);
        let (a, rest) = data.split_at(3);
        let (b, c) = rest.split_at(17);
        assert!(s.on_data_source(a).is_none());
        assert!(s.on_data_source(b).is_none());
        assert_eq!(s.on_data_source(c).unwrap().uid, 9);
    }

    #[test]
    fn only_one_request_in_flight() {
        let mut s = AncsSession::new();
        let now = Instant::now();
        s.on_notification_source(&source(0, 0, 4, 1));
        s.on_notification_source(&source(0, 0, 4, 2));
        assert!(s.next_request(now).is_some());
        assert!(s.next_request(now).is_none(), "must wait for the reply");
        s.on_data_source(&sms_reply(1)).unwrap();
        let req = s.next_request(now).unwrap();
        assert_eq!(&req[1..5], &2u32.to_le_bytes());
    }

    #[test]
    fn pre_existing_and_modified_events_are_not_requested() {
        let mut s = AncsSession::new();
        s.on_notification_source(&source(0, SourceEvent::FLAG_PRE_EXISTING, 4, 1));
        s.on_notification_source(&source(1, 0, 4, 2));
        assert!(s.next_request(Instant::now()).is_none());
    }

    #[test]
    fn removed_before_request_is_dropped() {
        let mut s = AncsSession::new();
        s.on_notification_source(&source(0, 0, 4, 5));
        s.on_notification_source(&source(2, 0, 4, 5));
        assert!(s.next_request(Instant::now()).is_none());
    }

    #[test]
    fn duplicate_added_events_are_queued_once() {
        let mut s = AncsSession::new();
        s.on_notification_source(&source(0, 0, 4, 5));
        s.on_notification_source(&source(0, 0, 4, 5));
        assert_eq!(s.pending(), 1);
    }

    #[test]
    fn stuck_request_expires() {
        let mut s = AncsSession::new();
        let t0 = Instant::now();
        s.on_notification_source(&source(0, 0, 4, 3));
        s.next_request(t0).unwrap();
        assert_eq!(s.expire(t0 + Duration::from_secs(1)), None);
        assert_eq!(
            s.expire(t0 + REQUEST_TIMEOUT + Duration::from_secs(1)),
            Some(3)
        );
        assert_eq!(s.pending(), 0);
    }

    #[test]
    fn failed_write_frees_the_control_point() {
        let mut s = AncsSession::new();
        s.on_notification_source(&source(0, 0, 4, 1));
        s.on_notification_source(&source(0, 0, 4, 2));
        s.next_request(Instant::now()).unwrap();
        s.request_failed();
        assert!(s.next_request(Instant::now()).is_some());
    }

    #[test]
    fn invalid_or_unexpected_replies() {
        let mut s = AncsSession::new();
        // No request in flight: ignore
        assert!(s.on_data_source(&sms_reply(1)).is_none());
        s.on_notification_source(&source(0, 0, 4, 1));
        s.next_request(Instant::now()).unwrap();
        // Wrong command id
        assert!(s.on_data_source(&[9, 1, 0, 0, 0]).is_none());
        assert_eq!(s.pending(), 0);
    }

    #[test]
    fn non_message_apps_are_flagged() {
        let mut s = AncsSession::new();
        s.on_notification_source(&source(0, 0, 2, 1));
        s.next_request(Instant::now()).unwrap();
        let n = s
            .on_data_source(&reply(
                1,
                &[(0, "com.apple.mobilephone"), (1, "Mama"), (3, ""), (5, "")],
            ))
            .unwrap();
        assert!(!n.is_message());
        assert_eq!(n.message, None);
    }
}
