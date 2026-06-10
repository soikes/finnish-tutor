#!/usr/bin/env bash
set -euo pipefail

# Reports remaining Groq rate limits for the models the app uses, by making
# one minimal probe request per endpoint and reading the x-ratelimit-* headers.
# Groq has no usage API for non-enterprise accounts, so this is the closest
# thing to "how close is my family to the limits". Costs ~20 tokens + 0.1s of
# audio per run.
#
# Usage: ./groq-usage.sh   (reads GROQ_API_KEY from env, .env, or server/.env)

PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CHAT_MODEL="llama-3.3-70b-versatile"
WHISPER_MODEL="whisper-large-v3-turbo"

if [ -z "${GROQ_API_KEY:-}" ]; then
    for env_file in "$PROJECT_ROOT/.env" "$PROJECT_ROOT/server/.env"; do
        if [ -f "$env_file" ]; then
            GROQ_API_KEY=$(grep -E '^GROQ_API_KEY=' "$env_file" | head -1 | cut -d= -f2-)
            [ -n "$GROQ_API_KEY" ] && break
        fi
    done
fi
if [ -z "${GROQ_API_KEY:-}" ]; then
    echo "error: GROQ_API_KEY not set and not found in .env" >&2
    exit 1
fi

header() { { grep -i "^$2:" "$1" || true; } | head -1 | cut -d: -f2- | tr -d ' \r'; }

# Print "used X of Y (Z%), resets in T" for each limit/remaining header pair
report() {
    local hdrs=$1 kind label
    for kind in requests tokens audio-seconds; do
        local limit remaining reset
        limit=$(header "$hdrs" "x-ratelimit-limit-$kind")
        remaining=$(header "$hdrs" "x-ratelimit-remaining-$kind")
        reset=$(header "$hdrs" "x-ratelimit-reset-$kind")
        [ -n "$limit" ] && [ -n "$remaining" ] || continue
        local used=$((limit - remaining))
        local pct=$((used * 100 / limit))
        case $kind in
            requests)      label="Requests     " ;;
            tokens)        label="Tokens       " ;;
            audio-seconds) label="Audio seconds" ;;
        esac
        printf '  %s  %s used of %s (%s%%)' "$label" "$used" "$limit" "$pct"
        [ -n "$reset" ] && printf ', resets in %s' "$reset"
        printf '\n'
    done
    return 0
}

tmpdir=$(mktemp -d)
trap 'rm -rf "$tmpdir"' EXIT

echo "=== $CHAT_MODEL (chat) ==="
status=$(curl -s -o "$tmpdir/chat_body" -D "$tmpdir/chat_hdrs" -w '%{http_code}' \
    https://api.groq.com/openai/v1/chat/completions \
    -H "Authorization: Bearer $GROQ_API_KEY" \
    -H "Content-Type: application/json" \
    -d "{\"model\": \"$CHAT_MODEL\", \"messages\": [{\"role\": \"user\", \"content\": \"hi\"}], \"max_completion_tokens\": 1}")
if [ "$status" = "200" ]; then
    report "$tmpdir/chat_hdrs"
elif [ "$status" = "429" ]; then
    echo "  RATE LIMITED (429) — limit already hit"
    report "$tmpdir/chat_hdrs"
else
    echo "  probe failed with HTTP $status:"
    head -c 300 "$tmpdir/chat_body"; echo
fi

echo
echo "=== $WHISPER_MODEL (STT) ==="
# 0.1s of silence: 44-byte WAV header + 1600 zero bytes (8kHz 16-bit mono)
python3 - "$tmpdir/silence.wav" <<'PY'
import struct, sys
data = b"\x00" * 1600
with open(sys.argv[1], "wb") as f:
    f.write(b"RIFF" + struct.pack("<I", 36 + len(data)) + b"WAVEfmt ")
    f.write(struct.pack("<IHHIIHH", 16, 1, 1, 8000, 16000, 2, 16))
    f.write(b"data" + struct.pack("<I", len(data)) + data)
PY
status=$(curl -s -o "$tmpdir/stt_body" -D "$tmpdir/stt_hdrs" -w '%{http_code}' \
    https://api.groq.com/openai/v1/audio/transcriptions \
    -H "Authorization: Bearer $GROQ_API_KEY" \
    -F "model=$WHISPER_MODEL" \
    -F "file=@$tmpdir/silence.wav")
if [ "$status" = "200" ]; then
    report "$tmpdir/stt_hdrs"
elif [ "$status" = "429" ]; then
    echo "  RATE LIMITED (429) — limit already hit"
    report "$tmpdir/stt_hdrs"
else
    echo "  probe failed with HTTP $status:"
    head -c 300 "$tmpdir/stt_body"; echo
fi
