#!/usr/bin/env bash
# NodeFlare bare-metal server setup script (Ubuntu 22.04 / Debian 12)
# Run as root: bash setup-baremetal.sh
set -euo pipefail

ARCH=$(dpkg --print-architecture)
NERDCTL_VERSION=2.0.4
CNI_VERSION=1.5.1
CONTAINERD_VERSION=2.0.4

echo "=== [1/6] System packages ==="
apt-get update -qq
apt-get install -y --no-install-recommends \
    ca-certificates curl wget git unzip \
    iptables iproute2 socat

echo "=== [2/6] containerd ==="
CONTAINERD_URL="https://github.com/containerd/containerd/releases/download/v${CONTAINERD_VERSION}/containerd-${CONTAINERD_VERSION}-linux-${ARCH}.tar.gz"
curl -fsSL "$CONTAINERD_URL" | tar -C /usr/local -xz

# containerd systemd unit — pin to the installed version, not the rolling main branch
curl -fsSL "https://raw.githubusercontent.com/containerd/containerd/v${CONTAINERD_VERSION}/containerd.service" \
    -o /etc/systemd/system/containerd.service
systemctl daemon-reload
systemctl enable --now containerd

echo "=== [3/6] CNI plugins ==="
mkdir -p /opt/cni/bin
CNI_URL="https://github.com/containernetworking/plugins/releases/download/v${CNI_VERSION}/cni-plugins-linux-${ARCH}-v${CNI_VERSION}.tgz"
curl -fsSL "$CNI_URL" | tar -C /opt/cni/bin -xz

echo "=== [4/6] nerdctl ==="
NERDCTL_URL="https://github.com/containerd/nerdctl/releases/download/v${NERDCTL_VERSION}/nerdctl-${NERDCTL_VERSION}-linux-${ARCH}.tar.gz"
curl -fsSL "$NERDCTL_URL" | tar -C /usr/local/bin -xz nerdctl

# Verify
nerdctl --version

echo "=== [5/6] Caddy ==="
# Use official apt repository instead of curl|bash to avoid supply-chain risk.
apt-get install -y debian-keyring debian-archive-keyring apt-transport-https
curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/gpg.key' \
    | gpg --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg
curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt' \
    | tee /etc/apt/sources.list.d/caddy-stable.list
apt-get update -qq
apt-get install -y caddy

echo "=== [6/6] nodeflare user + directories ==="
id -u nodeflare &>/dev/null || useradd -r -s /sbin/nologin -G containerd nodeflare
mkdir -p /opt/nodeflare/{bin,env} /var/run/mcp /var/lib/mcp-data
chown nodeflare:nodeflare /var/run/mcp /var/lib/mcp-data

echo ""
echo "=== Setup complete ==="
echo "Next steps:"
echo "  1. Copy binaries: cp target/release/mcp-builder /opt/nodeflare/bin/"
echo "              cp target/release/mcp-proxy  /opt/nodeflare/bin/"
echo "  2. Configure env: cp deploy/env/builder.env.example /opt/nodeflare/env/builder.env"
echo "                    cp deploy/env/proxy.env.example   /opt/nodeflare/env/proxy.env"
echo "  3. Install services: cp deploy/systemd/*.service /etc/systemd/system/"
echo "                       systemctl daemon-reload"
echo "                       systemctl enable --now nodeflare-builder nodeflare-proxy"
echo "  4. Log in to ghcr.io: nerdctl login ghcr.io -u YOUR_GITHUB_USERNAME"
