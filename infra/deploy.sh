#!/usr/bin/env bash
set -euo pipefail

# Usage: ./deploy.sh <server-ip> [--first-run]

SERVER_IP="${1:?Usage: deploy.sh <server-ip> [--first-run]}"
FIRST_RUN="${2:-}"
PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
REMOTE_DIR="/opt/finnish-tutor"
REMOTE_SRC="/opt/finnish-tutor/src"
DOMAIN="finnish.soikke.li"

# Install build deps on server if needed
echo "=> Ensuring build dependencies..."
ssh "root@${SERVER_IP}" bash <<'DEPS'
    if ! command -v cargo &>/dev/null; then
        echo "Installing Rust..."
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
        source "$HOME/.cargo/env"
    fi
    if ! dpkg -s build-essential cmake clang pkg-config &>/dev/null 2>&1; then
        echo "Installing build tools..."
        apt-get update && apt-get install -y build-essential cmake clang pkg-config
    fi
    if ! command -v caddy &>/dev/null; then
        echo "Installing Caddy..."
        apt-get update && apt-get install -y caddy
    fi
DEPS

echo "=> Configuring Caddy for ${DOMAIN}..."
ssh "root@${SERVER_IP}" bash <<CADDY
    cat > /etc/caddy/Caddyfile <<EOF
${DOMAIN} {
    reverse_proxy localhost:8080
}
EOF
    systemctl enable --now caddy
    systemctl reload caddy
CADDY

echo "=> Uploading source..."
rsync -az --delete \
    --exclude target/ \
    "$PROJECT_ROOT/server/" "root@${SERVER_IP}:${REMOTE_SRC}/"

echo "=> Uploading prompt..."
scp "$PROJECT_ROOT/prompt.md" "root@${SERVER_IP}:${REMOTE_DIR}/prompt.md"

echo "=> Uploading models (skipping existing)..."
for f in "$PROJECT_ROOT/models/"*.onnx "$PROJECT_ROOT/models/"*.onnx.json; do
    fname=$(basename "$f")
    local_size=$(stat -f%z "$f" 2>/dev/null || stat -c%s "$f" 2>/dev/null)
    remote_size=$(ssh "root@${SERVER_IP}" "stat -c%s ${REMOTE_DIR}/models/${fname} 2>/dev/null || echo 0")
    if [ "$local_size" = "$remote_size" ]; then
        echo "   Skipping $fname (already exists, same size)"
    else
        echo "   Uploading $fname..."
        scp "$f" "root@${SERVER_IP}:${REMOTE_DIR}/models/$fname"
    fi
done

echo "=> Building on server..."
ssh "root@${SERVER_IP}" bash <<'BUILD'
    source "$HOME/.cargo/env"
    cd /opt/finnish-tutor/src
    cargo build --release
    systemctl stop finnish-tutor 2>/dev/null || true
    cp target/release/finnish-tutor /opt/finnish-tutor/finnish-tutor
    cp target/release/libonnxruntime.so /usr/local/lib/ 2>/dev/null && ldconfig || true
BUILD

if [ "$FIRST_RUN" = "--first-run" ]; then
    echo "=> First run setup..."

    # Get server public key
    SERVER_PUBKEY=$(ssh "root@${SERVER_IP}" "cat /etc/wireguard/server_public.key")
    echo "Server public key: $SERVER_PUBKEY"

    # Generate client keys locally
    CLIENT_PRIVKEY=$(wg genkey)
    CLIENT_PUBKEY=$(echo "$CLIENT_PRIVKEY" | wg pubkey)

    # Replace WireGuard config with exactly one client peer (idempotent)
    ssh "root@${SERVER_IP}" bash -c "
        SERVER_PRIVKEY=\$(cat /etc/wireguard/server_private.key)
        cat > /etc/wireguard/wg0.conf <<EOF
[Interface]
Address = 10.0.0.1/24
ListenPort = 51820
PrivateKey = \$SERVER_PRIVKEY

[Peer]
PublicKey = ${CLIENT_PUBKEY}
AllowedIPs = 10.0.0.2/32
EOF
        chmod 600 /etc/wireguard/wg0.conf
        wg syncconf wg0 <(wg-quick strip wg0)

        # Create .env (preserves GROQ_API_KEY and AUTH_TOKEN if already set)
        EXISTING_KEY=\$(grep -oP 'GROQ_API_KEY=\K.*' ${REMOTE_DIR}/.env 2>/dev/null || echo 'REPLACE_ME')
        EXISTING_TOKEN=\$(grep -oP 'AUTH_TOKEN=\K.*' ${REMOTE_DIR}/.env 2>/dev/null || openssl rand -hex 32)
        cat > ${REMOTE_DIR}/.env <<EOF
GROQ_API_KEY=\$EXISTING_KEY
AUTH_TOKEN=\$EXISTING_TOKEN
PROMPT_PATH=${REMOTE_DIR}/prompt.md
MODELS_DIR=${REMOTE_DIR}/models
LISTEN_ADDR=127.0.0.1:8080
EOF
    "

    WG_CONFIG="[Interface]
PrivateKey = ${CLIENT_PRIVKEY}
Address = 10.0.0.2/24
DNS = 1.1.1.1

[Peer]
PublicKey = ${SERVER_PUBKEY}
Endpoint = ${SERVER_IP}:51820
AllowedIPs = 10.0.0.1/32
PersistentKeepalive = 25"

    echo ""
    echo "=== Admin WireGuard Config (optional, SSH/admin only — the app no longer uses it) ==="
    echo "Scan this QR code in the WireGuard app if you want VPN access:"
    echo ""
    echo "$WG_CONFIG" | qrencode -t ansiutf8
    echo ""
    AUTH_TOKEN=$(ssh "root@${SERVER_IP}" "grep -oP 'AUTH_TOKEN=\K.*' ${REMOTE_DIR}/.env")
    echo "=== App Config (put these in app/.env and EAS env vars) ==="
    echo "EXPO_PUBLIC_SERVER_URL=wss://${DOMAIN}/ws"
    echo "EXPO_PUBLIC_AUTH_TOKEN=${AUTH_TOKEN}"
    echo ""
    echo "Reminder: DNS A record ${DOMAIN} -> ${SERVER_IP} must exist for Caddy to get a certificate."
    echo "Note: re-running --first-run regenerates WireGuard client keys (existing WG phone configs break)."
    echo ""
fi

echo "=> Restarting service..."
ssh "root@${SERVER_IP}" "systemctl restart finnish-tutor"

echo "=> Done! Checking status..."
ssh "root@${SERVER_IP}" "systemctl status finnish-tutor --no-pager"
