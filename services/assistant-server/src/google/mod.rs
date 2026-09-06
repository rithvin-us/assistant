//! Google ecosystem integrations and personal schedule foundation.

pub mod client;
pub mod free_time;

pub use client::GoogleClient;
pub use free_time::{calculate_free_slots, find_free_slots};
