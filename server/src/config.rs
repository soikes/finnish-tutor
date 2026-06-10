use std::env;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    pub groq_api_key: String,
    pub prompt_path: PathBuf,
    pub models_dir: PathBuf,
    pub listen_addr: String,
    pub auth_token: String,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            groq_api_key: env::var("GROQ_API_KEY").expect("GROQ_API_KEY must be set"),
            prompt_path: env::var("PROMPT_PATH")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("../prompt.md")),
            models_dir: env::var("MODELS_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("../models")),
            // Loopback by default; Caddy terminates TLS and proxies to us
            listen_addr: env::var("LISTEN_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".to_string()),
            auth_token: env::var("AUTH_TOKEN").expect("AUTH_TOKEN must be set"),
        }
    }
}
