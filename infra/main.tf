terraform {
  required_providers {
    hcloud = {
      source  = "hetznercloud/hcloud"
      version = "~> 1.45"
    }
  }
}

provider "hcloud" {
  token = var.hcloud_token
}

resource "hcloud_ssh_key" "default" {
  name       = "finnish-tutor"
  public_key = file(var.ssh_public_key_path)
}

resource "hcloud_firewall" "finnish_tutor" {
  name = "finnish-tutor"

  rule {
    direction   = "in"
    protocol    = "tcp"
    port        = "22"
    source_ips  = ["0.0.0.0/0", "::/0"]
  }

  rule {
    direction   = "in"
    protocol    = "udp"
    port        = "51820"
    source_ips  = ["0.0.0.0/0", "::/0"]
    description = "WireGuard VPN (admin only)"
  }

  rule {
    direction   = "in"
    protocol    = "tcp"
    port        = "80"
    source_ips  = ["0.0.0.0/0", "::/0"]
    description = "Caddy ACME HTTP challenge + redirect"
  }

  rule {
    direction   = "in"
    protocol    = "tcp"
    port        = "443"
    source_ips  = ["0.0.0.0/0", "::/0"]
    description = "Caddy HTTPS/WSS"
  }
}

resource "hcloud_server" "finnish_tutor" {
  name        = "finnish-tutor"
  server_type = "cpx11"
  location    = "hil"
  image       = "ubuntu-24.04"
  ssh_keys    = [hcloud_ssh_key.default.id]

  firewall_ids = [hcloud_firewall.finnish_tutor.id]

  user_data = <<-EOF
    #!/bin/bash
    set -e
    apt-get update
    apt-get install -y wireguard

    # Generate WireGuard server keys
    mkdir -p /etc/wireguard
    wg genkey | tee /etc/wireguard/server_private.key | wg pubkey > /etc/wireguard/server_public.key
    chmod 600 /etc/wireguard/server_private.key

    SERVER_PRIVKEY=$(cat /etc/wireguard/server_private.key)

    # Create WireGuard config (client peer added by deploy.sh)
    cat > /etc/wireguard/wg0.conf << WGCONF
    [Interface]
    Address = 10.0.0.1/24
    ListenPort = 51820
    PrivateKey = $SERVER_PRIVKEY
    WGCONF

    chmod 600 /etc/wireguard/wg0.conf
    systemctl enable wg-quick@wg0
    systemctl start wg-quick@wg0

    # Create app directory
    mkdir -p /opt/finnish-tutor/models

    # Create systemd service
    cat > /etc/systemd/system/finnish-tutor.service << 'UNIT'
    [Unit]
    Description=Finnish Language Tutor Voice Server
    After=network.target wg-quick@wg0.service

    [Service]
    Type=simple
    WorkingDirectory=/opt/finnish-tutor
    EnvironmentFile=/opt/finnish-tutor/.env
    ExecStart=/opt/finnish-tutor/finnish-tutor
    Restart=always
    RestartSec=5

    [Install]
    WantedBy=multi-user.target
    UNIT

    systemctl daemon-reload
    systemctl enable finnish-tutor
  EOF
}
