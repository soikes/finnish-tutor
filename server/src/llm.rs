use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};

const GROQ_CHAT_URL: &str = "https://api.groq.com/openai/v1/chat/completions";
const MODEL: &str = "llama-3.3-70b-versatile";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

pub struct ChatClient {
    client: Client,
    api_key: String,
    system_prompt: String,
}

impl ChatClient {
    pub fn new(api_key: String, system_prompt: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
            system_prompt,
        }
    }

    /// Send a message with conversation history, return the full response.
    /// Streams internally for lower time-to-first-token but accumulates the full response.
    pub async fn chat(&self, history: &[Message], user_text: &str) -> anyhow::Result<String> {
        tracing::info!("LLM: sending chat with {} history messages, user: {:?}", history.len(), user_text);
        let mut messages = vec![Message {
            role: "system".to_string(),
            content: self.system_prompt.clone(),
        }];
        messages.extend_from_slice(history);
        messages.push(Message {
            role: "user".to_string(),
            content: user_text.to_string(),
        });

        let body = serde_json::json!({
            "model": MODEL,
            "messages": messages,
            "stream": true,
            "temperature": 0.7,
            "max_tokens": 512,
            // Halt the moment the model tries to roleplay a new turn for
            // either side. Catches the failure mode where it questions
            // itself and burns tokens on a hallucinated dialogue.
            "stop": ["\nUser:", "\nHuman:", "\nStudent:", "\nAssistant:"],
        });

        let resp = self
            .client
            .post(GROQ_CHAT_URL)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        tracing::debug!("LLM: Groq API responded with status {status}");

        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            tracing::error!("LLM: Groq chat API error {status}: {body}");
            anyhow::bail!("Groq chat API error {status}: {body}");
        }

        let mut full_response = String::new();
        let mut stream = resp.bytes_stream();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            let text = String::from_utf8_lossy(&chunk);

            for line in text.lines() {
                let line = line.trim();
                if let Some(data) = line.strip_prefix("data: ") {
                    if data == "[DONE]" {
                        break;
                    }
                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data) {
                        if let Some(content) = parsed["choices"][0]["delta"]["content"].as_str() {
                            full_response.push_str(content);
                        }
                    }
                }
            }
        }

        tracing::info!("LLM: response ({} chars): {:?}", full_response.len(), full_response);
        Ok(full_response)
    }
}
