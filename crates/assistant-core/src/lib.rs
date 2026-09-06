//! Domain seams for the assistant.
//!
//! This crate holds the vocabulary the rest of the system agrees on. It must
//! never depend on a concrete integration (Gmail, Google Calendar) or a concrete
//! model provider. Those depend on it, not the other way round.

pub mod event;

pub use event::{DomainEvent, EventBus, EventEnvelope};
