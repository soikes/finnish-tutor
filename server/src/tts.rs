use piper_rs::synth::PiperSpeechSynthesizer;
use piper_rs::from_config_path;
use std::path::Path;

pub struct TextToSpeech {
    english_synth: PiperSpeechSynthesizer,
    finnish_synth: PiperSpeechSynthesizer,
    sample_rate: u32,
}

/// A segment of text tagged with its language.
#[derive(Debug)]
enum Segment {
    English(String),
    Finnish(String),
}

impl TextToSpeech {
    pub fn new(models_dir: &Path) -> anyhow::Result<Self> {
        let en_config_path = models_dir.join("en_US-libritts-high.onnx.json");
        let fi_config_path = models_dir.join("fi_FI-harri-medium.onnx.json");

        let en_model = from_config_path(&en_config_path)
            .map_err(|e| anyhow::anyhow!("Failed to load English TTS model: {e}"))?;
        let fi_model = from_config_path(&fi_config_path)
            .map_err(|e| anyhow::anyhow!("Failed to load Finnish TTS model: {e}"))?;

        let sample_rate = en_model
            .audio_output_info()
            .map_err(|e| anyhow::anyhow!("Failed to get audio info: {e}"))?
            .sample_rate as u32;

        let english_synth = PiperSpeechSynthesizer::new(en_model)
            .map_err(|e| anyhow::anyhow!("Failed to create English synthesizer: {e}"))?;
        let finnish_synth = PiperSpeechSynthesizer::new(fi_model)
            .map_err(|e| anyhow::anyhow!("Failed to create Finnish synthesizer: {e}"))?;

        Ok(Self {
            english_synth,
            finnish_synth,
            sample_rate,
        })
    }

    /// Synthesize text that may contain `[fi]...[/fi]` tags, emitting PCM
    /// samples per segment via the callback so callers can stream audio.
    pub fn synthesize_streaming(
        &self,
        text: &str,
        mut on_segment: impl FnMut(Vec<i16>),
    ) -> anyhow::Result<()> {
        let segments = parse_language_segments(text);
        tracing::info!("TTS: synthesizing {} segments from text ({} chars)", segments.len(), text.len());
        tracing::debug!("TTS: segments: {:?}", segments);

        for segment in segments {
            let (synth, text) = match &segment {
                Segment::English(t) => (&self.english_synth, t.as_str()),
                Segment::Finnish(t) => (&self.finnish_synth, t.as_str()),
            };

            let trimmed = text.trim();
            if trimmed.is_empty() {
                tracing::debug!("TTS: skipping empty segment");
                continue;
            }

            let lang = match &segment {
                Segment::English(_) => "en",
                Segment::Finnish(_) => "fi",
            };
            tracing::info!("TTS: synthesizing [{lang}] segment: {:?}", trimmed);

            let stream = synth
                .synthesize_parallel(trimmed.to_string(), None)
                .map_err(|e| anyhow::anyhow!("TTS synthesis error: {e}"))?;

            let mut segment_samples: Vec<i16> = Vec::new();
            for result in stream {
                let audio = result.map_err(|e| anyhow::anyhow!("TTS audio error: {e}"))?;
                segment_samples.extend_from_slice(&audio.samples.to_i16_vec());
            }
            tracing::debug!("TTS: [{lang}] segment produced {} samples", segment_samples.len());
            on_segment(segment_samples);
        }

        Ok(())
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}

/// Parse text with `[fi]...[/fi]` tags into language-tagged segments.
fn parse_language_segments(text: &str) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut remaining = text;

    while let Some(start_idx) = remaining.find("[fi]") {
        let english_part = &remaining[..start_idx];
        if !english_part.trim().is_empty() {
            segments.push(Segment::English(english_part.to_string()));
        }

        let after_tag = &remaining[start_idx + 4..];
        if let Some(end_idx) = after_tag.find("[/fi]") {
            let finnish_part = &after_tag[..end_idx];
            if !finnish_part.trim().is_empty() {
                segments.push(Segment::Finnish(finnish_part.to_string()));
            }
            remaining = &after_tag[end_idx + 5..];
        } else {
            segments.push(Segment::Finnish(after_tag.to_string()));
            remaining = "";
            break;
        }
    }

    if !remaining.trim().is_empty() {
        segments.push(Segment::English(remaining.to_string()));
    }

    segments
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_segments() {
        let text = "Hello! Say [fi]Hei[/fi] to greet someone. You can also say [fi]Hyvää päivää[/fi] for good day.";
        let segments = parse_language_segments(text);

        assert!(matches!(&segments[0], Segment::English(t) if t.contains("Hello")));
        assert!(matches!(&segments[1], Segment::Finnish(t) if t == "Hei"));
        assert!(matches!(&segments[2], Segment::English(t) if t.contains("greet")));
        assert!(matches!(&segments[3], Segment::Finnish(t) if t.contains("Hyvää")));
        assert!(matches!(&segments[4], Segment::English(t) if t.contains("good day")));
    }

    #[test]
    fn test_no_finnish_tags() {
        let text = "This is just English text.";
        let segments = parse_language_segments(text);
        assert_eq!(segments.len(), 1);
        assert!(matches!(&segments[0], Segment::English(_)));
    }
}
