//! In-process event bus.
//!
//! The system is event-driven, but at this scale a distributed broker would be
//! pure cost. A Tokio broadcast channel gives fan-out to any number of
//! in-process subscribers; durable events will later be written to Postgres by a
//! subscriber rather than by replacing this bus. See docs/DECISIONS.md ADR-0004.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use tokio::sync::broadcast;
use uuid::Uuid;

/// Events the system publishes. Only variants the skeleton can actually emit are
/// present; the rest arrive with the milestone that produces them.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DomainEvent {
    ServerStarted { version: String },
    ConversationOpened { conversation_id: Uuid },
    ConversationClosed { conversation_id: Uuid },
}

/// A published event plus the metadata every subscriber needs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEnvelope {
    pub id: Uuid,
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
    pub event: DomainEvent,
}

/// Fan-out publisher. Cloning is cheap and shares the same channel.
#[derive(Debug, Clone)]
pub struct EventBus {
    tx: broadcast::Sender<EventEnvelope>,
}

impl EventBus {
    pub fn new(capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(capacity);
        Self { tx }
    }

    /// Publishes an event. Returns the number of live subscribers that received
    /// it; zero is normal and not an error.
    pub fn publish(&self, event: DomainEvent) -> usize {
        let envelope = EventEnvelope {
            id: Uuid::new_v4(),
            at: OffsetDateTime::now_utc(),
            event,
        };
        tracing::debug!(event = ?envelope.event, "domain event published");
        self.tx.send(envelope).unwrap_or(0)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<EventEnvelope> {
        self.tx.subscribe()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new(256)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn subscriber_receives_published_event() {
        let bus = EventBus::default();
        let mut rx = bus.subscribe();

        let delivered = bus.publish(DomainEvent::ServerStarted {
            version: "test".into(),
        });
        assert_eq!(delivered, 1);

        let envelope = rx.recv().await.expect("event delivered");
        assert!(matches!(envelope.event, DomainEvent::ServerStarted { .. }));
    }

    #[tokio::test]
    async fn publishing_without_subscribers_is_not_an_error() {
        let bus = EventBus::default();
        assert_eq!(
            bus.publish(DomainEvent::ServerStarted {
                version: "t".into()
            }),
            0
        );
    }
}
