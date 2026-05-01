use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use tokio::sync::mpsc;

use crate::pipeline::{Pipeline, PipelineEvent};

pub async fn handle_connection(socket: WebSocket, pipeline: Arc<Pipeline>) {
    let (mut sender, mut receiver) = socket.split();

    tracing::info!("WebSocket client connected");

    // Channel for streaming pipeline events to the client in real-time
    let (tx, mut rx) = mpsc::channel::<PipelineEvent>(32);

    // Spawn a task to forward pipeline events to the WebSocket
    let send_task = tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            let send_result = match event {
                PipelineEvent::Transcription(text) => {
                    tracing::info!("WS: sending transcription to client: {:?}", text);
                    let json = serde_json::json!({"type": "transcription", "text": text});
                    sender.send(Message::Text(json.to_string().into())).await
                }
                PipelineEvent::Response(text) => {
                    tracing::info!("WS: sending response to client: {:?}", text);
                    let json = serde_json::json!({"type": "response", "text": text});
                    sender.send(Message::Text(json.to_string().into())).await
                }
                PipelineEvent::AudioStart => {
                    tracing::info!("WS: sending audio_start");
                    let json = serde_json::json!({"type": "audio_start"});
                    sender.send(Message::Text(json.to_string().into())).await
                }
                PipelineEvent::Audio(samples) => {
                    let bytes: Vec<u8> = samples
                        .iter()
                        .flat_map(|s| s.to_le_bytes())
                        .collect();
                    tracing::info!("WS: sending {} bytes of TTS audio", bytes.len());
                    sender.send(Message::Binary(bytes.into())).await
                }
                PipelineEvent::AudioEnd => {
                    tracing::info!("WS: sending audio_end");
                    let json = serde_json::json!({"type": "audio_end"});
                    sender.send(Message::Text(json.to_string().into())).await
                }
                PipelineEvent::Interrupt => {
                    tracing::info!("WS: sending interrupt");
                    let json = serde_json::json!({"type": "interrupt"});
                    sender.send(Message::Text(json.to_string().into())).await
                }
                PipelineEvent::Error(msg) => {
                    tracing::error!("Pipeline error: {msg}");
                    let json = serde_json::json!({"type": "error", "message": msg});
                    sender.send(Message::Text(json.to_string().into())).await
                }
            };

            if send_result.is_err() {
                tracing::warn!("Failed to send WebSocket message");
                break;
            }
        }
    });

    while let Some(msg) = receiver.next().await {
        let msg = match msg {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!("WebSocket receive error: {e}");
                break;
            }
        };

        match msg {
            Message::Binary(data) => {
                let sample_count = data.len() / 2;
                tracing::debug!("WS: received {} bytes ({} samples)", data.len(), sample_count);
                // Decode PCM 16-bit LE samples
                let samples: Vec<i16> = data
                    .chunks_exact(2)
                    .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
                    .collect();

                pipeline.process_audio(&samples, &tx).await;
            }
            Message::Text(text) => {
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&text) {
                    match parsed["type"].as_str() {
                        Some("start") => {
                            tracing::info!("Session started");
                            pipeline.reset().await;
                        }
                        Some("stop") => {
                            tracing::info!("Session stopped");
                            pipeline.reset().await;
                        }
                        Some("cancel") => {
                            tracing::info!("Cancel response requested");
                            pipeline.cancel_response().await;
                        }
                        _ => {}
                    }
                }
            }
            Message::Close(_) => {
                tracing::info!("WebSocket client disconnected");
                break;
            }
            _ => {}
        }
    }

    drop(tx);
    let _ = send_task.await;
}
