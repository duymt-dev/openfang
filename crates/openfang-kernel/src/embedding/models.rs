/// Pick a sensible default embedding model for a given provider when the user
/// configured an explicit `embedding_provider` but left `embedding_model` at the
/// default value (which is a local model name that cloud APIs wouldn't recognise).
pub fn default_embedding_model_for_provider(provider: &str) -> &'static str {
    match provider {
        "openai" => "text-embedding-3-small",
        "groq" => "nomic-embed-text",
        "mistral" => "mistral-embed",
        "together" => "togethercomputer/m2-bert-80M-8k-retrieval",
        "fireworks" => "nomic-ai/nomic-embed-text-v1.5",
        "cohere" => "embed-english-v3.0",
        // Local providers use nomic-embed-text as a good default
        "ollama" | "vllm" | "lmstudio" => "nomic-embed-text",
        // Other OpenAI-compatible APIs typically support the OpenAI model names
        _ => "text-embedding-3-small",
    }
}
