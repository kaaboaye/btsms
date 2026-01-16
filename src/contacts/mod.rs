pub mod phone_normalizer;
pub mod manager;

pub use phone_normalizer::{normalize_e164, is_valid_e164};
pub use manager::{ContactManager, Contact, PhoneNumber};
