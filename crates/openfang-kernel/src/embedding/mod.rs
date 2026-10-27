//! Embedding initialization policy; independent from the chat-model provider chain.
mod initialize;
mod models;

pub use initialize::initialize;
pub use models::default_embedding_model_for_provider;
