mod config;
mod llm;
mod pipeline;
mod stt;
mod tts;
mod vad;
mod ws;

use axum::{
    extract::{State, WebSocketUpgrade},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::get,
    Router,
};
use subtle::ConstantTimeEq;
use std::net::SocketAddr;
use std::sync::Arc;

use config::Config;
use llm::ChatClient;
use pipeline::Pipeline;
use stt::SpeechToText;
use tts::TextToSpeech;
use vad::VoiceActivityDetector;

struct AppState {
    config: Config,
    stt: Arc<SpeechToText>,
    llm: Arc<ChatClient>,
    tts: Arc<TextToSpeech>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "finnish_tutor=debug,info".parse().unwrap()),
        )
        .init();
    let _ = dotenvy::dotenv();

    let config = Config::from_env();
    tracing::info!("Starting finnish-tutor server on {}", config.listen_addr);

    // Load system prompt
    let system_prompt = std::fs::read_to_string(&config.prompt_path)
        .unwrap_or_else(|e| panic!("Failed to read prompt file {:?}: {e}", config.prompt_path));
    tracing::info!("Loaded system prompt from {:?}", config.prompt_path);

    // Initialize shared services
    let stt = Arc::new(SpeechToText::new(config.groq_api_key.clone()));
    let llm = Arc::new(ChatClient::new(config.groq_api_key.clone(), system_prompt));
    let tts = Arc::new(TextToSpeech::new(&config.models_dir)?);
    tracing::info!("Initialized STT, LLM, and TTS services");

    let state = Arc::new(AppState {
        config: config.clone(),
        stt,
        llm,
        tts,
    });

    let app = Router::new()
        .route("/ws", get(ws_handler))
        .route("/health", get(|| async { "ok" }))
        .with_state(state);

    let addr: SocketAddr = config.listen_addr.parse()?;
    tracing::info!("Listening on {addr}");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn ws_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    if !authorized(&headers, &state.config.auth_token) {
        tracing::warn!("Rejected WebSocket connection with missing or invalid token");
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let vad = match VoiceActivityDetector::new(&state.config.models_dir) {
        Ok(v) => v,
        Err(e) => {
            tracing::error!("Failed to create VAD: {e}");
            return axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let pipeline = Arc::new(Pipeline::new(
        vad,
        state.stt.clone(),
        state.llm.clone(),
        state.tts.clone(),
    ));

    ws.on_upgrade(move |socket| ws::handle_connection(socket, pipeline))
}

fn authorized(headers: &HeaderMap, expected_token: &str) -> bool {
    let Some(provided) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    else {
        return false;
    };
    provided.as_bytes().ct_eq(expected_token.as_bytes()).into()
}
