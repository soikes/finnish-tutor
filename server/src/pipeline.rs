use crate::llm::{ChatClient, Message};
use crate::stt::SpeechToText;
use crate::tts::TextToSpeech;
use crate::vad::VoiceActivityDetector;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

/// Per-connection voice pipeline state.
pub struct Pipeline {
    vad: Mutex<VoiceActivityDetector>,
    stt: Arc<SpeechToText>,
    llm: Arc<ChatClient>,
    tts: Arc<TextToSpeech>,
    history: Mutex<Vec<Message>>,
}

/// Events emitted by the pipeline for the WebSocket handler to send to the client.
pub enum PipelineEvent {
    Transcription(String),
    Response(String),
    AudioStart,
    Audio(Vec<i16>),
    AudioEnd,
    Error(String),
}

impl Pipeline {
    pub fn new(
        vad: VoiceActivityDetector,
        stt: Arc<SpeechToText>,
        llm: Arc<ChatClient>,
        tts: Arc<TextToSpeech>,
    ) -> Self {
        Self {
            vad: Mutex::new(vad),
            stt,
            llm,
            tts,
            history: Mutex::new(Vec::new()),
        }
    }

    /// Process incoming audio samples. Sends pipeline events to the channel
    /// as they happen so the client gets transcription immediately.
    pub async fn process_audio(&self, samples: &[i16], tx: &mpsc::Sender<PipelineEvent>) {
        // Log audio level to verify we're getting real audio
        if !samples.is_empty() {
            let rms = (samples.iter().map(|&s| (s as f64) * (s as f64)).sum::<f64>() / samples.len() as f64).sqrt();
            let max = samples.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
            tracing::debug!("Pipeline: {} samples, RMS={:.0}, max={max}", samples.len(), rms);
        }

        // Run VAD to detect complete utterances
        let utterance = {
            let mut vad = self.vad.lock().await;
            match vad.process(samples) {
                Ok(result) => result,
                Err(e) => {
                    let _ = tx.send(PipelineEvent::Error(format!("VAD error: {e}"))).await;
                    return;
                }
            }
        };

        let utterance = match utterance {
            Some(u) if u.len() > 1600 => {
                let duration_ms = u.len() as f64 / 16.0;
                tracing::info!("Pipeline: VAD detected utterance: {} samples ({:.0}ms)", u.len(), duration_ms);
                u
            }
            Some(u) => {
                tracing::debug!("Pipeline: VAD utterance too short ({} samples, need >1600), discarding", u.len());
                return;
            }
            _ => return,
        };

        // STT: transcribe the utterance
        let stt_start = std::time::Instant::now();
        let transcription = match self.stt.transcribe(&utterance).await {
            Ok(text) => text,
            Err(e) => {
                let _ = tx.send(PipelineEvent::Error(format!("STT error: {e}"))).await;
                return;
            }
        };

        tracing::info!("Pipeline: STT took {:?}", stt_start.elapsed());

        if transcription.trim().is_empty() {
            tracing::info!("Pipeline: STT returned empty transcription, skipping");
            return;
        }

        tracing::info!("Pipeline: transcription: {:?}", transcription);
        let _ = tx.send(PipelineEvent::Transcription(transcription.clone())).await;

        // LLM: get response
        let llm_start = std::time::Instant::now();
        let history = self.history.lock().await.clone();
        let response = match self.llm.chat(&history, &transcription).await {
            Ok(resp) => resp,
            Err(e) => {
                let _ = tx.send(PipelineEvent::Error(format!("LLM error: {e}"))).await;
                return;
            }
        };

        tracing::info!("Pipeline: LLM took {:?}, response: {:?}", llm_start.elapsed(), response);
        let _ = tx.send(PipelineEvent::Response(response.clone())).await;

        // Update conversation history
        {
            let mut history = self.history.lock().await;
            history.push(Message {
                role: "user".to_string(),
                content: transcription,
            });
            history.push(Message {
                role: "assistant".to_string(),
                content: response.clone(),
            });

            // Keep history bounded (last 20 messages)
            if history.len() > 20 {
                let drain_count = history.len() - 20;
                history.drain(..drain_count);
            }
        }

        // TTS: synthesize response audio
        let tts_start = std::time::Instant::now();
        let _ = tx.send(PipelineEvent::AudioStart).await;
        match self.tts.synthesize(&response) {
            Ok(audio) => {
                tracing::info!("Pipeline: TTS took {:?}, produced {} samples", tts_start.elapsed(), audio.len());
                let _ = tx.send(PipelineEvent::Audio(audio)).await;
            }
            Err(e) => {
                let _ = tx.send(PipelineEvent::Error(format!("TTS error: {e}"))).await;
            }
        }
        let _ = tx.send(PipelineEvent::AudioEnd).await;
    }

    pub async fn reset(&self) {
        self.vad.lock().await.reset();
        self.history.lock().await.clear();
    }
}
