# デプロイ手順

## ベアメタルサーバー

| 項目 | 値 |
|------|-----|
| サーバー | `ubuntu@158.69.26.12` |
| OS | Ubuntu 22.04 (glibc 2.35) |
| バイナリ配置 | `/opt/nodeflare/bin/` |
| 環境変数 | `/opt/nodeflare/env/` |

## SSH接続

```bash
ssh ubuntu@158.69.26.12
```

## バイナリのビルド

WSL2はglibc 2.39、サーバーはglibc 2.35のため、**muslターゲットでスタティックビルドする必要がある**。

```bash
CARGO_TARGET_DIR=/tmp/nodeflare-build \
  cargo build --release \
  --target x86_64-unknown-linux-musl \
  -p mcp-proxy -p mcp-builder
```

ビルド成果物:
```
/tmp/nodeflare-build/x86_64-unknown-linux-musl/release/mcp-proxy
/tmp/nodeflare-build/x86_64-unknown-linux-musl/release/mcp-builder
```

## デプロイ手順

### 1. サーバーに転送

```bash
scp /tmp/nodeflare-build/x86_64-unknown-linux-musl/release/mcp-proxy \
    /tmp/nodeflare-build/x86_64-unknown-linux-musl/release/mcp-builder \
    ubuntu@158.69.26.12:/tmp/
```

### 2. インストール＆再起動

```bash
ssh ubuntu@158.69.26.12 "
  # バックアップ
  sudo cp /opt/nodeflare/bin/mcp-proxy    /opt/nodeflare/bin/mcp-proxy.bak
  sudo cp /opt/nodeflare/bin/mcp-builder  /opt/nodeflare/bin/mcp-builder.bak

  # 配置
  sudo mv /tmp/mcp-proxy   /opt/nodeflare/bin/mcp-proxy
  sudo mv /tmp/mcp-builder /opt/nodeflare/bin/mcp-builder
  sudo chmod +x /opt/nodeflare/bin/mcp-proxy /opt/nodeflare/bin/mcp-builder

  # 再起動
  sudo systemctl restart nodeflare-proxy nodeflare-builder

  # 確認
  sleep 2
  sudo systemctl status nodeflare-proxy nodeflare-builder --no-pager
"
```

### proxyのみデプロイする場合

```bash
scp /tmp/nodeflare-build/x86_64-unknown-linux-musl/release/mcp-proxy \
    ubuntu@158.69.26.12:/tmp/

ssh ubuntu@158.69.26.12 "
  sudo cp /opt/nodeflare/bin/mcp-proxy /opt/nodeflare/bin/mcp-proxy.bak
  sudo mv /tmp/mcp-proxy /opt/nodeflare/bin/mcp-proxy
  sudo chmod +x /opt/nodeflare/bin/mcp-proxy
  sudo systemctl restart nodeflare-proxy
  sleep 2
  sudo systemctl status nodeflare-proxy --no-pager
"
```

## ログ確認

```bash
# リアルタイムログ
ssh ubuntu@158.69.26.12 "sudo journalctl -u nodeflare-proxy -f"
ssh ubuntu@158.69.26.12 "sudo journalctl -u nodeflare-builder -f"

# 直近100行
ssh ubuntu@158.69.26.12 "sudo journalctl -u nodeflare-proxy -n 100 --no-pager"
```

## ロールバック

```bash
ssh ubuntu@158.69.26.12 "
  sudo cp /opt/nodeflare/bin/mcp-proxy.bak   /opt/nodeflare/bin/mcp-proxy
  sudo cp /opt/nodeflare/bin/mcp-builder.bak /opt/nodeflare/bin/mcp-builder
  sudo systemctl restart nodeflare-proxy nodeflare-builder
"
```

## GitHubへのpush

WSL環境からはGitHub認証が必要:

```bash
# 初回のみ（ブラウザ認証フローが開く）
gh auth login

# push
git push origin <branch-name>
```
