//! Deterministic Voice Session State Machine.

use thiserror::Error;

// The wire definition is the single definition. This machine validates the very
// states the client is told about, rather than a parallel copy of them.
pub use assistant_protocol::VoiceState;

#[derive(Debug, Error)]
pub enum StateMachineError {
    #[error("Invalid state transition from {from:?} to {to:?}")]
    InvalidTransition { from: VoiceState, to: VoiceState },
}

#[derive(Debug, Clone)]
pub struct VoiceStateMachine {
    current_state: VoiceState,
}

impl Default for VoiceStateMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl VoiceStateMachine {
    pub fn new() -> Self {
        Self {
            current_state: VoiceState::Idle,
        }
    }

    pub fn state(&self) -> VoiceState {
        self.current_state
    }

    pub fn transition_to(&mut self, target: VoiceState) -> Result<VoiceState, StateMachineError> {
        let allowed = match (self.current_state, target) {
            // Same state is always allowed (idempotent)
            (from, to) if from == to => true,

            // Error and Interrupted can be reached from any active state
            (_, VoiceState::Error) => true,
            (_, VoiceState::Interrupted) => true,

            // Return to Idle from Interrupted or Error
            (VoiceState::Interrupted, VoiceState::Idle) => true,
            (VoiceState::Error, VoiceState::Idle) => true,

            // Normal turn progression
            (VoiceState::Idle, VoiceState::Listening) => true,
            (VoiceState::Listening, VoiceState::Transcribing) => true,
            (VoiceState::Transcribing, VoiceState::Thinking) => true,
            (VoiceState::Thinking, VoiceState::Speaking) => true,
            (VoiceState::Speaking, VoiceState::Idle) => true,

            // Direct cancel/reset to Idle from active states
            (VoiceState::Listening, VoiceState::Idle) => true,
            (VoiceState::Transcribing, VoiceState::Idle) => true,
            (VoiceState::Thinking, VoiceState::Idle) => true,

            // User starts speaking while thinking or transcribing
            (VoiceState::Thinking, VoiceState::Listening) => true,
            (VoiceState::Speaking, VoiceState::Listening) => true,

            _ => false,
        };

        if allowed {
            self.current_state = target;
            Ok(self.current_state)
        } else {
            Err(StateMachineError::InvalidTransition {
                from: self.current_state,
                to: target,
            })
        }
    }

    pub fn reset(&mut self) {
        self.current_state = VoiceState::Idle;
    }
}
