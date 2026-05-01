use reqwest::Client;

const GROQ_TRANSCRIPTION_URL: &str = "https://api.groq.com/openai/v1/audio/transcriptions";
const WHISPER_MODEL: &str = "whisper-large-v3-turbo";

pub struct SpeechToText {
    client: Client,
    api_key: String,
}

impl SpeechToText {
    pub fn new(api_key: String) -> Self {
        Self {
            client: Client::new(),
            api_key,
        }
    }

    /// Transcribe PCM audio (16-bit LE, 16kHz mono) to text via Groq Whisper.
    pub async fn transcribe(&self, pcm_samples: &[i16]) -> anyhow::Result<String> {
        let duration_ms = pcm_samples.len() as f64 / 16.0;
        tracing::info!("STT: transcribing {:.0}ms of audio ({} samples)", duration_ms, pcm_samples.len());
        let wav_bytes = encode_wav(pcm_samples, 16000);
        tracing::debug!("STT: encoded WAV is {} bytes", wav_bytes.len());

        let form = reqwest::multipart::Form::new()
            .text("model", WHISPER_MODEL)
            .text("response_format", "json")
            .text("prompt", "The speaker is practicing Finnish (suomi) and English. Hei, kiitos, hyvää päivää, anteeksi.")
            .part(
                "file",
                reqwest::multipart::Part::bytes(wav_bytes)
                    .file_name("audio.wav")
                    .mime_str("audio/wav")?,
            );

        let resp = self
            .client
            .post(GROQ_TRANSCRIPTION_URL)
            .bearer_auth(&self.api_key)
            .multipart(form)
            .send()
            .await?;

        let status = resp.status();
        tracing::debug!("STT: Groq API responded with status {status}");

        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            tracing::error!("STT: Groq Whisper API error {status}: {body}");
            anyhow::bail!("Groq Whisper API error {status}: {body}");
        }

        let body: serde_json::Value = resp.json().await?;
        let text = body["text"]
            .as_str()
            .unwrap_or("")
            .to_string();

        tracing::info!("STT: transcription result: {:?}", text);
        Ok(text)
    }
}

/// Encode PCM i16 samples as a WAV file in memory.
fn encode_wav(samples: &[i16], sample_rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let file_len = 36 + data_len;

    let mut buf = Vec::with_capacity(file_len as usize + 8);

    // RIFF header
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&file_len.to_le_bytes());
    buf.extend_from_slice(b"WAVE");

    // fmt chunk
    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes()); // chunk size
    buf.extend_from_slice(&1u16.to_le_bytes()); // PCM format
    buf.extend_from_slice(&1u16.to_le_bytes()); // mono
    buf.extend_from_slice(&sample_rate.to_le_bytes());
    buf.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
    buf.extend_from_slice(&2u16.to_le_bytes()); // block align
    buf.extend_from_slice(&16u16.to_le_bytes()); // bits per sample

    // data chunk
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_len.to_le_bytes());
    for &sample in samples {
        buf.extend_from_slice(&sample.to_le_bytes());
    }

    buf
}
