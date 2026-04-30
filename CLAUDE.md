# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Real-time voice-based Finnish language learning app. A phone app streams audio over WebSocket to a Rust server, which runs a voice pipeline: VAD → STT → LLM → TTS, and streams synthesized audio back.

## Architecture

- **server/** — Rust binary (axum + tokio) handling WebSocket connections and the full voice pipeline
- **app/** — Expo React Native mobile client with minimal UI (mic button + status)
- **infra/** — Terraform for Hetzner Cloud provisioning + deploy scripts
- **models/** — ONNX models for Silero VAD and Piper TTS (gitignored, downloaded via `models/download.sh`)
- **prompt.md** — LLM system prompt loaded at runtime, configurable without recompiling

## Networking

Server is only accessible over a WireGuard VPN tunnel (no public HTTP/HTTPS ports). The phone connects to WireGuard first, then reaches the server at `10.0.0.1:8080`. No TLS or auth tokens needed — the tunnel handles encryption and authentication.

## Voice Pipeline

Audio in → Silero VAD (detect speech end) → Groq Whisper STT → Groq Llama 3.3 70B → Piper TTS → Audio out

The LLM wraps Finnish text in `[fi]...[/fi]` tags. The TTS module parses these to route English text to the English Piper voice and Finnish text to the Finnish Piper voice.

## Build & Run

```bash
# Download ONNX models (required first time)
./models/download.sh

# Build server
cd server && cargo build

# Run server (needs .env with GROQ_API_KEY)
cd server && cargo run

# Local development
LISTEN_ADDR=127.0.0.1:8080 cargo run
```

## Key Dependencies

- **ort** = 2.0.0-rc.9 — ONNX Runtime (pinned to match silero-vad-rs)
- **ort-sys** = 2.0.0-rc.9 — must be pinned alongside ort to avoid TLS build errors in newer versions
- **ndarray** = 0.15 — must match silero-vad-rs (not 0.16)
- **piper-rs** — Rust bindings for Piper TTS
- **silero-vad-rs** — Rust bindings for Silero VAD
- **axum** — HTTP/WebSocket server
- **reqwest** — HTTP client for Groq API calls

## External Services

- **Groq API** (`api.groq.com/openai/v1`) — Whisper STT and Llama 3.3 70B chat completions
- **Hetzner Cloud** — CPX11 x86 server in Hillsboro, OR

## Deployment

Target: Hetzner CPX11 (x86_64). Deploy via `infra/deploy.sh` which uploads source and builds on the server (avoids cross-compilation issues with C dependencies like espeak-ng). First deploy with `--first-run` flag to install Rust/build tools, set up WireGuard keys, and print phone config.
