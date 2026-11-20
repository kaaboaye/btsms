pub mod manager;
pub mod phone_normalizer;
pub mod vcard;

pub use manager::ContactManager;
pub use phone_normalizer::{normalize_address, normalize_for_sending};
