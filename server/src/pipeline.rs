use crate::llm::{ChatClient, Message};
use crate::stt::SpeechToText;
use crate::tts::TextToSpeech;
use crate::vad::VoiceActivityDetector;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinHandle;

struct InFlightResponse {
    handle: JoinHandle<()>,
    cancelled: Arc<AtomicBool>,
}

/// Per-connection voice pipeline state.
pub struct Pipeline {
    vad: Mutex<VoiceActivityDetector>,
    stt: Arc<SpeechToText>,
    llm: Arc<ChatClient>,
    tts: Arc<TextToSpeech>,
    history: Mutex<Vec<Message>>,
    /// In-flight response task and its cancellation flag, if any.
    response: Mutex<Option<InFlightResponse>>,
}

/// Events emitted by the pipeline for the WebSocket handler to send to the client.
pub enum PipelineEvent {
    Transcription(String),
    Response(String),
    AudioStart,
    Audio(Vec<i16>),
    AudioEnd,
    Interrupt,
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
            response: Mutex::new(None),
        }
    }

    /// Process incoming audio through VAD only. If an utterance is detected,
    /// cancel any in-flight response and spawn a new response task.
    pub async fn process_audio(
        self: &Arc<Self>,
        samples: &[i16],
        tx: &mpsc::Sender<PipelineEvent>,
    ) {
        // Run VAD
        let utterance = {
            let mut vad = self.vad.lock().await;
            match vad.process(samples) {
                Ok(result) => result,
                Err(e) => {
                    let _ = tx
                        .send(PipelineEvent::Error(format!("VAD error: {e}")))
                        .await;
                    return;
                }
            }
        };

        let utterance = match utterance {
            Some(u) if u.len() > 1600 => {
                let duration_ms = u.len() as f64 / 16.0;
                tracing::info!(
                    "Pipeline: VAD detected utterance: {} samples ({:.0}ms)",
                    u.len(),
                    duration_ms
                );
                u
            }
            Some(u) => {
                tracing::debug!(
                    "Pipeline: VAD utterance too short ({} samples), discarding",
                    u.len()
                );
                return;
            }
            _ => return,
        };

        // Cancel any in-flight response
        {
            let mut slot = self.response.lock().await;
            if let Some(prev) = slot.take() {
                tracing::info!("Pipeline: interrupting in-flight response");
                prev.cancelled.store(true, Ordering::SeqCst);
                prev.handle.abort();
                let _ = tx.send(PipelineEvent::Interrupt).await;
            }
        }

        // Spawn response generation as a background task
        let cancelled = Arc::new(AtomicBool::new(false));
        let pipeline = Arc::clone(self);
        let tx = tx.clone();
        let task_cancelled = Arc::clone(&cancelled);
        let handle = tokio::spawn(async move {
            pipeline
                .generate_response(utterance, &tx, task_cancelled)
                .await;
        });

        *self.response.lock().await = Some(InFlightResponse { handle, cancelled });
    }

    /// Run STT → LLM → TTS for a detected utterance. Runs in a spawned task
    /// so it can be cancelled if the user interrupts. The `cancelled` flag is
    /// checked inside the TTS callback so blocking synthesis stops emitting
    /// audio chunks the moment cancellation happens — `JoinHandle::abort()`
    /// alone doesn't reach into spawn_blocking.
    async fn generate_response(
        &self,
        utterance: Vec<i16>,
        tx: &mpsc::Sender<PipelineEvent>,
        cancelled: Arc<AtomicBool>,
    ) {
        // STT
        let stt_start = std::time::Instant::now();
        let transcription = match self.stt.transcribe(&utterance).await {
            Ok(text) => text,
            Err(e) => {
                let _ = tx
                    .send(PipelineEvent::Error(format!("STT error: {e}")))
                    .await;
                return;
            }
        };

        tracing::info!("Pipeline: STT took {:?}", stt_start.elapsed());

        if transcription.trim().is_empty() {
            tracing::info!("Pipeline: STT returned empty transcription, skipping");
            return;
        }

        tracing::info!("Pipeline: transcription: {:?}", transcription);
        let _ = tx
            .send(PipelineEvent::Transcription(transcription.clone()))
            .await;

        // LLM
        let llm_start = std::time::Instant::now();
        let history = self.history.lock().await.clone();
        let response = match self.llm.chat(&history, &transcription).await {
            Ok(resp) => resp,
            Err(e) => {
                let _ = tx
                    .send(PipelineEvent::Error(format!("LLM error: {e}")))
                    .await;
                return;
            }
        };

        tracing::info!(
            "Pipeline: LLM took {:?}, response: {:?}",
            llm_start.elapsed(),
            response
        );
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

            if history.len() > 20 {
                let drain_count = history.len() - 20;
                history.drain(..drain_count);
            }
        }

        // TTS — stream audio per segment so the client can start playing
        // before the whole response is synthesized.
        let tts_start = std::time::Instant::now();
        let _ = tx.send(PipelineEvent::AudioStart).await;
        let tts = Arc::clone(&self.tts);
        let response_clone = response.clone();
        let tx_blocking = tx.clone();
        let tts_cancelled = Arc::clone(&cancelled);
        let tts_result = tokio::task::spawn_blocking(move || {
            tts.synthesize_streaming(&response_clone, |samples| {
                if tts_cancelled.load(Ordering::SeqCst) {
                    return;
                }
                let _ = tx_blocking.blocking_send(PipelineEvent::Audio(samples));
            })
        })
        .await;
        match tts_result {
            Ok(Ok(())) => {
                tracing::info!("Pipeline: TTS took {:?}", tts_start.elapsed());
            }
            Ok(Err(e)) => {
                let _ = tx
                    .send(PipelineEvent::Error(format!("TTS error: {e}")))
                    .await;
            }
            Err(e) => {
                let _ = tx
                    .send(PipelineEvent::Error(format!("TTS task error: {e}")))
                    .await;
            }
        }
        if !cancelled.load(Ordering::SeqCst) {
            let _ = tx.send(PipelineEvent::AudioEnd).await;
        }

        // Clear ourselves from the response slot
        *self.response.lock().await = None;
    }

    pub async fn reset(&self) {
        if let Some(prev) = self.response.lock().await.take() {
            tracing::info!("Pipeline: resetting in-flight response");
            prev.cancelled.store(true, Ordering::SeqCst);
            prev.handle.abort();
        }
        self.vad.lock().await.reset();
        self.history.lock().await.clear();
    }

    /// Abort any in-flight response without clearing history or VAD state.
    pub async fn cancel_response(&self) {
        if let Some(prev) = self.response.lock().await.take() {
            tracing::info!("Pipeline: cancelling in-flight response (user-requested)");
            prev.cancelled.store(true, Ordering::SeqCst);
            prev.handle.abort();
        }
    }
}
