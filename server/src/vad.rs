use ndarray::{Array1, Array2, Array3};
use ort::session::Session;
use ort::value::Tensor;
use std::path::Path;

const SAMPLE_RATE: u32 = 16000;
/// Silero VAD processes 512 new samples per chunk, but prepends 64 context
/// samples from the previous chunk, making the actual input 576 samples.
const CHUNK_SIZE: usize = 512;
const CONTEXT_SIZE: usize = 64;
const SPEECH_THRESHOLD: f32 = 0.5;
const MIN_SILENCE_MS: usize = 700;
const SPEECH_PAD_MS: usize = 300;

/// Silero VAD v5 ONNX model wrapper.
/// Inputs: input (1, 576), sr (1,) i64, state (2, 1, 128)
/// Outputs: output (1, 1), stateN (2, 1, 128)
struct SileroVad {
    session: Session,
    state: Array3<f32>,
    context: Vec<f32>,
}

impl SileroVad {
    fn new(model_path: &Path) -> anyhow::Result<Self> {
        let session = Session::builder()?
            .with_intra_threads(1)?
            .commit_from_file(model_path)?;

        Ok(Self {
            session,
            state: Array3::zeros((2, 1, 128)),
            context: vec![0.0f32; CONTEXT_SIZE],
        })
    }

    fn process_chunk(&mut self, audio: &[f32]) -> anyhow::Result<f32> {
        // Prepend context from previous chunk
        let mut input_data = Vec::with_capacity(CONTEXT_SIZE + audio.len());
        input_data.extend_from_slice(&self.context);
        input_data.extend_from_slice(audio);

        // Save last CONTEXT_SIZE samples as context for next chunk
        let new_context_start = input_data.len().saturating_sub(CONTEXT_SIZE);
        self.context = input_data[new_context_start..].to_vec();

        let input = Array2::from_shape_vec((1, input_data.len()), input_data)?;
        let sr = Array1::from_vec(vec![SAMPLE_RATE as i64]);

        let inputs = vec![
            ("input", Tensor::from_array(input)?.into_dyn()),
            ("sr", Tensor::from_array(sr)?.into_dyn()),
            ("state", Tensor::from_array(self.state.clone())?.into_dyn()),
        ];

        let outputs = self.session.run(inputs)?;

        let prob = outputs[0].try_extract_tensor::<f32>()?;
        let probability = prob[[0, 0]];

        let new_state = outputs[1].try_extract_tensor::<f32>()?;
        let state_data: Vec<f32> = new_state.iter().cloned().collect();
        self.state = Array3::from_shape_vec((2, 1, 128), state_data)?;

        Ok(probability)
    }

    fn reset(&mut self) {
        self.state = Array3::zeros((2, 1, 128));
        self.context = vec![0.0f32; CONTEXT_SIZE];
    }
}

pub struct VoiceActivityDetector {
    vad: SileroVad,
    is_speaking: bool,
    silence_samples: usize,
    speech_buffer: Vec<i16>,
    chunk_buffer: Vec<f32>,
    min_silence_samples: usize,
    speech_pad_samples: usize,
    chunks_processed: usize,
}

impl VoiceActivityDetector {
    pub fn new(models_dir: &Path) -> anyhow::Result<Self> {
        let model_path = models_dir.join("silero_vad.onnx");
        let vad = SileroVad::new(&model_path)?;

        tracing::info!("VAD: initialized, chunk_size={CHUNK_SIZE}+{CONTEXT_SIZE}ctx, threshold={SPEECH_THRESHOLD}, min_silence={MIN_SILENCE_MS}ms, pad={SPEECH_PAD_MS}ms");
        Ok(Self {
            vad,
            is_speaking: false,
            silence_samples: 0,
            speech_buffer: Vec::new(),
            chunk_buffer: Vec::new(),
            min_silence_samples: (MIN_SILENCE_MS * SAMPLE_RATE as usize) / 1000,
            speech_pad_samples: (SPEECH_PAD_MS * SAMPLE_RATE as usize) / 1000,
            chunks_processed: 0,
        })
    }

    pub fn process(&mut self, samples: &[i16]) -> anyhow::Result<Option<Vec<i16>>> {
        for &s in samples {
            self.chunk_buffer.push(s as f32 / 32768.0);
        }

        while self.chunk_buffer.len() >= CHUNK_SIZE {
            let chunk: Vec<f32> = self.chunk_buffer.drain(..CHUNK_SIZE).collect();
            let probability = self.vad.process_chunk(&chunk)?;

            let i16_chunk: Vec<i16> = chunk.iter().map(|&f| (f * 32768.0) as i16).collect();

            self.chunks_processed += 1;

            // Log every chunk at debug, but also log at info every 32 chunks (~1 second)
            tracing::debug!("VAD prob: {probability:.3}, speaking: {}, buf: {} samples", self.is_speaking, self.speech_buffer.len());
            if self.chunks_processed % 32 == 0 {
                tracing::info!("VAD: chunk #{}, prob={probability:.3}, speaking={}, speech_buf={} samples, silence={}",
                    self.chunks_processed, self.is_speaking, self.speech_buffer.len(), self.silence_samples);
            }

            if probability >= SPEECH_THRESHOLD {
                if !self.is_speaking {
                    tracing::info!("VAD: speech START detected (prob={probability:.3})");
                }
                self.is_speaking = true;
                self.silence_samples = 0;
                self.speech_buffer.extend_from_slice(&i16_chunk);
            } else if self.is_speaking {
                self.silence_samples += CHUNK_SIZE;
                self.speech_buffer.extend_from_slice(&i16_chunk);

                if self.silence_samples >= self.min_silence_samples {
                    let trim_len = self.silence_samples.saturating_sub(self.speech_pad_samples);
                    let final_len = self.speech_buffer.len().saturating_sub(trim_len);
                    self.speech_buffer.truncate(final_len);

                    let utterance = std::mem::take(&mut self.speech_buffer);
                    let duration_ms = utterance.len() as f64 / 16.0;
                    tracing::info!("VAD: speech END — utterance: {} samples ({:.0}ms)", utterance.len(), duration_ms);
                    self.is_speaking = false;
                    self.silence_samples = 0;
                    self.vad.reset();
                    return Ok(Some(utterance));
                }
            }
        }

        Ok(None)
    }

    pub fn reset(&mut self) {
        tracing::info!("VAD: reset (processed {} chunks total)", self.chunks_processed);
        self.is_speaking = false;
        self.silence_samples = 0;
        self.speech_buffer.clear();
        self.chunk_buffer.clear();
        self.chunks_processed = 0;
        self.vad.reset();
    }
}
