use super::models::default_embedding_model_for_provider;
use openfang_runtime::embedding::EmbeddingDriver;
use openfang_types::config::KernelConfig;
use std::sync::Arc;
use tracing::{debug, info, warn};

/// Build the configured embedding driver; explicit disabled mode never constructs one.
pub fn initialize(config: &KernelConfig) -> Option<Arc<dyn EmbeddingDriver + Send + Sync>> {
    if config.memory.embedding_provider.as_deref() == Some("disabled") {
        info!("Embedding driver explicitly disabled; local text recall only");
        return None;
    }
    use openfang_runtime::embedding::create_embedding_driver;
    let configured_model = &config.memory.embedding_model;
    if let Some(ref provider) = config.memory.embedding_provider {
        // Explicit config takes priority — use the configured embedding model.
        // If the user left embedding_model at the default ("all-MiniLM-L6-v2"),
        // pick a sensible default for the chosen provider so we don't send a
        // local model name to a cloud API.
        let model = if configured_model == "all-MiniLM-L6-v2" {
            default_embedding_model_for_provider(provider)
        } else {
            configured_model.as_str()
        };
        let api_key_env = config.memory.embedding_api_key_env.as_deref().unwrap_or("");
        let custom_url = config
            .provider_urls
            .get(provider.as_str())
            .map(|s| s.as_str());
        match create_embedding_driver(provider, model, api_key_env, custom_url) {
            Ok(d) => {
                info!(provider = %provider, model = %model, "Embedding driver configured from memory config");
                Some(Arc::from(d))
            }
            Err(e) => {
                warn!(provider = %provider, error = %e, "Embedding driver init failed — falling back to text search");
                None
            }
        }
    } else {
        // Auto-detect embedding provider by checking API key env vars in
        // priority order.  First match wins.
        const API_KEY_PROVIDERS: &[(&str, &str)] = &[
            ("OPENAI_API_KEY", "openai"),
            ("GROQ_API_KEY", "groq"),
            ("MISTRAL_API_KEY", "mistral"),
            ("TOGETHER_API_KEY", "together"),
            ("FIREWORKS_API_KEY", "fireworks"),
            ("COHERE_API_KEY", "cohere"),
        ];

        let detected_from_key = API_KEY_PROVIDERS
            .iter()
            .find(|(env_var, _)| std::env::var(env_var).is_ok())
            .and_then(|(env_var, provider)| {
                let model = if configured_model == "all-MiniLM-L6-v2" {
                    default_embedding_model_for_provider(provider)
                } else {
                    configured_model.as_str()
                };
                let custom_url = config.provider_urls.get(*provider).map(|s| s.as_str());
                match create_embedding_driver(provider, model, env_var, custom_url) {
                    Ok(d) => {
                        info!(provider = %provider, model = %model, "Embedding driver auto-detected via {}", env_var);
                        Some(Arc::from(d))
                    }
                    Err(e) => {
                        warn!(provider = %provider, error = %e, "Embedding auto-detect failed for {}", provider);
                        None
                    }
                }
            });

        if detected_from_key.is_some() {
            detected_from_key
        } else {
            // No API key found — try local providers in order:
            // Ollama, vLLM, LM Studio (no key needed).
            const LOCAL_PROVIDERS: &[&str] = &["ollama", "vllm", "lmstudio"];

            let mut local_result = None;
            for provider in LOCAL_PROVIDERS {
                let model = if configured_model == "all-MiniLM-L6-v2" {
                    default_embedding_model_for_provider(provider)
                } else {
                    configured_model.as_str()
                };
                let custom_url = config.provider_urls.get(*provider).map(|s| s.as_str());
                match create_embedding_driver(provider, model, "", custom_url) {
                    Ok(d) => {
                        info!(provider = %provider, model = %model, "Embedding driver auto-detected: {} (local)", provider);
                        local_result = Some(Arc::from(d));
                        break;
                    }
                    Err(e) => {
                        debug!(provider = %provider, error = %e, "Local embedding provider {} not available", provider);
                    }
                }
            }

            if local_result.is_none() {
                warn!(
                    "No embedding provider available. Memory recall will use text search only. \
                     Configure [memory] embedding_provider in config.toml or set an API key \
                     (OPENAI_API_KEY, GROQ_API_KEY, MISTRAL_API_KEY, TOGETHER_API_KEY, \
                     FIREWORKS_API_KEY, COHERE_API_KEY)."
                );
            }

            local_result
        }
    }
}
