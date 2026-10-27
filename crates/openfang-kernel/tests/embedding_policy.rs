use openfang_kernel::embedding::{default_embedding_model_for_provider, initialize};
use openfang_types::config::KernelConfig;

#[test]
fn disabled_ignores_provider_overrides_and_never_constructs_a_driver() {
    let mut config = KernelConfig::default();
    config.memory.embedding_provider = Some("disabled".into());
    config.memory.embedding_api_key_env = Some("PATH".into());
    config
        .provider_urls
        .insert("disabled".into(), "http://127.0.0.1:1".into());
    assert!(initialize(&config).is_none());
}

#[test]
fn explicit_local_provider_and_model_keep_existing_behavior() {
    let mut config = KernelConfig::default();
    config.memory.embedding_provider = Some("ollama".into());
    config.memory.embedding_model = "text-embedding-3-small".into();
    let driver = initialize(&config).expect("explicit local driver remains available");
    assert_eq!(driver.dimensions(), 1536);
    // Construction only: no request to any local/remote provider is made by this test.
}

#[test]
fn extracted_default_model_mapping_preserves_upstream_choices() {
    for (provider, expected) in [
        ("openai", "text-embedding-3-small"),
        ("groq", "nomic-embed-text"),
        ("mistral", "mistral-embed"),
        ("together", "togethercomputer/m2-bert-80M-8k-retrieval"),
        ("fireworks", "nomic-ai/nomic-embed-text-v1.5"),
        ("cohere", "embed-english-v3.0"),
        ("ollama", "nomic-embed-text"),
        ("vllm", "nomic-embed-text"),
        ("lmstudio", "nomic-embed-text"),
        ("other", "text-embedding-3-small"),
    ] {
        assert_eq!(default_embedding_model_for_provider(provider), expected);
    }
}
