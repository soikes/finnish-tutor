#!/usr/bin/env bash
set -euo pipefail

MODELS_DIR="$(cd "$(dirname "$0")" && pwd)"

echo "Downloading models to $MODELS_DIR..."

# Silero VAD v5
echo "=> Silero VAD v5"
curl -L -o "$MODELS_DIR/silero_vad.onnx" \
  "https://github.com/snakers4/silero-vad/raw/master/src/silero_vad/data/silero_vad.onnx"

# Piper English voice (en_US-lessac-medium)
echo "=> Piper English voice (lessac-medium)"
curl -L -o "$MODELS_DIR/en_US-lessac-medium.onnx" \
  "https://huggingface.co/rhasspy/piper-voices/resolve/v1.0.0/en/en_US/lessac/medium/en_US-lessac-medium.onnx"
curl -L -o "$MODELS_DIR/en_US-lessac-medium.onnx.json" \
  "https://huggingface.co/rhasspy/piper-voices/resolve/v1.0.0/en/en_US/lessac/medium/en_US-lessac-medium.onnx.json"

# Piper Finnish voice (fi_FI-harri-medium)
echo "=> Piper Finnish voice (harri-medium)"
curl -L -o "$MODELS_DIR/fi_FI-harri-medium.onnx" \
  "https://huggingface.co/rhasspy/piper-voices/resolve/v1.0.0/fi/fi_FI/harri/medium/fi_FI-harri-medium.onnx"
curl -L -o "$MODELS_DIR/fi_FI-harri-medium.onnx.json" \
  "https://huggingface.co/rhasspy/piper-voices/resolve/v1.0.0/fi/fi_FI/harri/medium/fi_FI-harri-medium.onnx.json"

echo "Done! Models downloaded to $MODELS_DIR"
