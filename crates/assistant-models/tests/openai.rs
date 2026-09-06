use assistant_models::{
    Capability, ModelProvider,
    openai::{OpenAIConfig, OpenAIModelProvider},
};

#[test]
fn api_key_is_redacted_in_debug() {
    let config = OpenAIConfig::new("sk-secret-test-key");
    let debug = format!("{config:?}");
    assert!(!debug.contains("sk-secret-test-key"));
    assert!(debug.contains("<redacted>"));
}

#[test]
fn provider_advertises_correct_capabilities() {
    let config = OpenAIConfig::new("sk-test");
    let provider = OpenAIModelProvider::new(config).unwrap();
    assert_eq!(provider.name(), "openai");
    assert!(provider.supports(Capability::Generate));
    assert!(provider.supports(Capability::Stream));
    assert!(provider.supports(Capability::ToolUse));
}

#[test]
fn rejects_empty_api_key() {
    let config = OpenAIConfig::new("   ");
    let provider = OpenAIModelProvider::new(config);
    assert!(provider.is_err());
}
