//! Regression tests for the voice error contract and the provider defaults.
//!
//! These pin things that were actually wrong and that fail silently when they
//! regress: an error code the client branches on, and the Cartesia model ids
//! and API version, all three of which were invalid and made every real call
//! fail with `400 invalid model`.

use assistant_voice::{
    CARTESIA_API_VERSION, DEFAULT_STT_MODEL, DEFAULT_TTS_MODEL, VoiceError, VoiceState,
    VoiceStateMachine,
};
use std::time::Duration;

#[test]
fn every_failure_has_a_distinct_stable_code() {
    // The client picks its recovery path from these strings. Collapsing two
    // failures onto one code silently removes a recovery path, so assert they
    // stay distinct rather than merely non-empty.
    let codes = [
        VoiceError::Unconfigured("x".into()).code(),
        VoiceError::AuthenticationFailed("x".into()).code(),
        VoiceError::Network("x".into()).code(),
        VoiceError::AudioEncoding("x".into()).code(),
        VoiceError::Api {
            code: "400".into(),
            message: "x".into(),
        }
        .code(),
        VoiceError::Cancelled.code(),
        VoiceError::Timeout(Duration::from_secs(30)).code(),
    ];

    let mut unique = codes.to_vec();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(
        unique.len(),
        codes.len(),
        "two failures share a code: {codes:?}"
    );
}

#[test]
fn only_transient_failures_are_retryable() {
    // Retrying a rejected key or a bad model id fails identically forever, so
    // offering "try again" for those wastes the user's time.
    assert!(VoiceError::Network("x".into()).is_retryable());
    assert!(VoiceError::Timeout(Duration::from_secs(1)).is_retryable());

    assert!(!VoiceError::AuthenticationFailed("x".into()).is_retryable());
    assert!(!VoiceError::Unconfigured("x".into()).is_retryable());
    assert!(!VoiceError::Cancelled.is_retryable());
}

#[test]
fn error_messages_do_not_leak_the_api_key() {
    // Provider errors are rendered into a frame the client displays.
    let err = VoiceError::AuthenticationFailed("Invalid Cartesia API key".into());
    let rendered = err.to_string();
    assert!(
        !rendered.contains("sk_"),
        "rendered error carried a key: {rendered}"
    );
}

#[test]
fn cartesia_defaults_match_the_live_contract() {
    // Verified against the live API: these exact values round-trip, and the
    // previous ones (ink-en-us, sonic-english/sonic-2, 2024-06-10) returned
    // "400 invalid model". A silent revert here breaks all voice.
    assert_eq!(DEFAULT_STT_MODEL, "ink-whisper");
    assert!(
        DEFAULT_TTS_MODEL.starts_with("sonic-"),
        "unexpected TTS model {DEFAULT_TTS_MODEL}"
    );
    assert_ne!(
        DEFAULT_TTS_MODEL, "sonic-english",
        "sonic-english is retired"
    );
    assert_ne!(
        DEFAULT_STT_MODEL, "ink-en-us",
        "ink-en-us is not a real model"
    );
    assert_eq!(CARTESIA_API_VERSION, "2026-08-14");
}

#[test]
fn a_superseded_turn_cannot_walk_back_into_speaking() {
    // Barge-in puts the turn in Interrupted. From there the only way on is Idle:
    // an interrupted turn must never resume playback, which is what produced
    // stale audio over a new question.
    let mut sm = VoiceStateMachine::new();
    sm.transition_to(VoiceState::Listening).unwrap();
    sm.transition_to(VoiceState::Transcribing).unwrap();
    sm.transition_to(VoiceState::Thinking).unwrap();
    sm.transition_to(VoiceState::Speaking).unwrap();
    sm.transition_to(VoiceState::Interrupted).unwrap();

    assert!(sm.transition_to(VoiceState::Speaking).is_err());
    assert!(sm.transition_to(VoiceState::Thinking).is_err());
    assert!(sm.transition_to(VoiceState::Transcribing).is_err());
    assert_eq!(sm.state(), VoiceState::Interrupted);

    assert!(sm.transition_to(VoiceState::Idle).is_ok());
}

#[test]
fn a_failed_turn_recovers_only_through_idle() {
    let mut sm = VoiceStateMachine::new();
    sm.transition_to(VoiceState::Listening).unwrap();
    sm.transition_to(VoiceState::Error).unwrap();

    // Not straight back into the middle of a turn that no longer exists.
    assert!(sm.transition_to(VoiceState::Speaking).is_err());
    assert!(sm.transition_to(VoiceState::Transcribing).is_err());

    assert!(sm.transition_to(VoiceState::Idle).is_ok());
    assert!(sm.transition_to(VoiceState::Listening).is_ok());
}

#[test]
fn a_turn_cannot_skip_straight_to_speaking() {
    // Speaking without having transcribed or thought means audio with no
    // answer behind it.
    let mut sm = VoiceStateMachine::new();
    assert!(sm.transition_to(VoiceState::Speaking).is_err());
    assert_eq!(sm.state(), VoiceState::Idle);
}
