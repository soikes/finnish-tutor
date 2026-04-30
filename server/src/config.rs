use std::env;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    pub groq_api_key: String,
    pub prompt_path: PathBuf,
    pub models_dir: PathBuf,
    pub listen_addr: String,
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
            // Listen on WireGuard interface by default
            listen_addr: env::var("LISTEN_ADDR").unwrap_or_else(|_| "10.0.0.1:8080".to_string()),
        }
    }
}
