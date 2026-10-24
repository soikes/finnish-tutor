# finnish-tutor

Real-time voice app for learning Finnish. The phone streams mic audio over WebSocket to a Rust server, which runs VAD → STT → LLM → TTS and streams speech back.

Silero VAD → Groq Whisper → Groq Llama 3.3 70B → Piper TTS (English + Finnish voices).

## Layout

- `server/` — Rust (axum + tokio) WebSocket server and voice pipeline
- `app/` — Expo React Native client
- `infra/` — Terraform (Hetzner) and deploy scripts
- `models/` — ONNX models, fetched by `models/download.sh`
- `prompt.md` — LLM system prompt, loaded at runtime

## Run locally

```bash
./models/download.sh
cp .env.example .env   # set GROQ_API_KEY
cd server && AUTH_TOKEN=devtoken cargo run
```

Listens on `127.0.0.1:8080`. `/health` is unauthenticated; `/ws` needs `Authorization: Bearer <AUTH_TOKEN>`.

The app reads `EXPO_PUBLIC_SERVER_URL` and `EXPO_PUBLIC_AUTH_TOKEN` from `app/.env`.

## Deploy

`infra/deploy.sh --first-run` provisions the box, builds on the server, sets up Caddy (TLS), and prints the app config. Later deploys: `infra/deploy.sh`.

iOS builds go through EAS: `eas build --platform ios --profile production`, then `eas submit`.
