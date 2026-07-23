# ベアメタル移行ガイド

Fly.ioからOVHcloud/Hetznerベアメタルへの移行作業一覧。
全ソースコードを読んで洗い出した、抜け漏れのない版。

## 移行方針

| サービス | 移行先 | 理由 |
|---------|--------|------|
| Proxy | ベアメタル | MCPサーバーとUnix Socketで通信するため必須 |
| Builder | ベアメタル | `nerdctl build` をサーバー上で実行するため |
| Code Runner | ベアメタル | Proxyから `localhost` で呼ぶため |
| MCPサーバー群 | ベアメタル | これが本題 |
| **API** | **Fly.io継続** | DBに繋ぐだけでMCPサーバーと直接通信しない |
| Frontend | Vercel継続 | 変更なし |
| PostgreSQL (Neon) | 継続 | 変更なし |
| Redis (Upstash) | 継続 | 変更なし |

---

## 現在のアーキテクチャ（Fly.io）

```
[Claude/Client]
     │ HTTPS
     ▼
[mcp-cloud-proxy.fly.dev]       ← Fly.io app (nrt+iad)
     │ HTTPS (.fly.dev)
     ▼
[mcp-{uuid-nohyphens}.fly.dev]  ← 各MCPサーバー (Fly.io app per server)
     │
     └── [mcp-code-runner.internal:8080]  ← Deno sandbox (Fly.io app, 6PN IPv6)

[nodeflare-api.fly.dev]         ← REST API (Fly.io app, nrt)
[nodeflare-builder.fly.dev]     ← ビルド/デプロイワーカー (Fly.io app, nrt)
[PostgreSQL (Neon)] [Redis (Upstash)] ← 外部マネージドサービス
```

## 移行後のアーキテクチャ

```
[Claude/Client]
     │ HTTPS
     ▼
[Caddy] (TLS終端)                ← ベアメタルサーバー（Debian 12）
     │ HTTP
     ▼
[mcp-cloud-proxy]                ← systemdサービス ★移行
     │ Unix Socket (/var/run/mcp/mcp-{uuid-nohyphens}.sock)
     ▼
[MCP containers via containerd]  ← nerdctl/containerd管理 ★移行
     │
     └── [mcp-code-runner, localhost:8082]  ★移行

[nodeflare-api.fly.dev]          ← Fly.io継続（変更なし）
[nodeflare-builder]              ← systemdサービス ★移行
[PostgreSQL (Neon)] [Redis (Upstash)] ← 継続（変更なし）
[apps/web on Vercel]             ← Vercel継続（変更なし）
```

**API と ベアメタルの接続:**
- API → Redis (Upstash) → Builder という経路でデプロイジョブを渡す（今と同じ）
- API が直接ベアメタルを叩く経路はない

---

## 凡例
- 🔴 **完全書き直し** — Fly.io専用コードのため全削除・新実装
- 🟠 **大幅修正** — Fly.io前提の処理を別実装に置き換え
- 🟡 **小修正** — フォーマット変更・設定値変更・コメント修正
- 🟢 **変更なし** — そのまま使える

---

## 1. インフラ新規作成

### 1.1 OSセットアップ
→ `docs/baremetal/os-layer.md` 参照

- [ ] Debian 12 インストール（OVHcloud コントロールパネル）
- [ ] `fs.file-max`, `somaxconn`, `vm.swappiness` sysctl設定
- [ ] noatime マウントオプション
- [ ] 不要パッケージ削除

### 1.2 コンテナランタイム
→ `docs/baremetal/container-runtime-layer.md` 参照

- [ ] containerd インストール・設定
- [ ] crun（OCIランタイム）インストール
- [ ] Nydus snapshotter（lazy pull用）
- [ ] cgroups v2 有効確認
- [ ] nerdctl インストール（CLI操作用）

### 1.3 Caddy

```caddyfile
# /etc/caddy/Caddyfile
api.nodeflare.tech {
    reverse_proxy localhost:8080
}
*.nodeflare.tech {
    reverse_proxy localhost:8081
}
```

### 1.4 コンテナレジストリ

GitHub Container Registry (ghcr.io) を推奨。既存GitHub連携をそのまま使用。
自己ホスト型（`distribution/registry`）も選択可能。

---

## 2. `docker/Dockerfile.builder` 🟠

**現状:**
```dockerfile
# flyctl をインストール（Fly.io公式CLI）
RUN curl -L https://fly.io/install.sh | FLYCTL_INSTALL=/usr/local sh
```

**変更後:**
```dockerfile
# nerdctl をインストール（containerd管理CLI）
ARG NERDCTL_VERSION=1.7.6
RUN curl -L https://github.com/containerd/nerdctl/releases/download/v${NERDCTL_VERSION}/nerdctl-${NERDCTL_VERSION}-linux-amd64.tar.gz \
    | tar -xz -C /usr/local/bin nerdctl
```

**影響範囲:** `docker/Dockerfile.builder` 1ファイルのみ。

---

## 3. `crates/common/src/config.rs` 🟠

**現状 (行174-189):**
```rust
pub struct FlyioConfig {
    pub api_token: String,   // FLY_API_TOKEN
    pub org_slug: String,    // FLY_ORG_SLUG
    pub region: String,      // FLY_REGION (default: "nrt")
}
// AppConfig.flyio: FlyioConfig
```

**変更後:**
```rust
pub struct ContainerConfig {
    pub registry_base: String,    // CONTAINER_REGISTRY_BASE = "ghcr.io/org-name"
    pub registry_token: String,   // CONTAINER_REGISTRY_TOKEN = ghp_xxxx
    pub socket_dir: String,       // CONTAINER_SOCKET_DIR = "/var/run/mcp"
    pub data_dir: String,         // CONTAINER_DATA_DIR = "/var/lib/mcp-data"
}
// AppConfig.container: ContainerConfig
```

**削除する環境変数:**
```
FLY_API_TOKEN
FLY_ORG_SLUG
FLY_REGION
```

**追加する環境変数:**
```
CONTAINER_REGISTRY_BASE=ghcr.io/org-name
CONTAINER_REGISTRY_TOKEN=ghp_xxxxx
CONTAINER_SOCKET_DIR=/var/run/mcp
CONTAINER_DATA_DIR=/var/lib/mcp-data
```

---

## 4. `crates/builder/` 🔴

### 4.1 `flyctl.rs` (3,317行) — 完全削除

flyctl CLI（Fly.io公式ツール）を全面的にラップするモジュール。

| 関数 | Fly.io処理 | ベアメタル代替 |
|------|-----------|--------------|
| `build_and_deploy()` (行1,941-) | `flyctl deploy --remote-only` リモートビルド | `nerdctl build` + `nerdctl push` |
| `generate_fly_toml()` (行612-) | MCPサーバー用 fly.toml 生成 | 不要（起動パラメータに移行） |
| `destroy_app()` (行2,347-) | `flyctl apps destroy` CLI実行 | `nerdctl rm -f mcp-{id}` |
| `verify_mcp_initialize()` (行2,571-) | `https://{app}.fly.dev/mcp` HTTPプローブ | Unix Socket経由プローブ |
| `fetch_app_logs()` (行2,484-) | `flyctl logs -a {app}` CLI実行 | `nerdctl logs mcp-{id}` |
| `get_machine_id()` (行2,379-) | Fly Machines API `GET /apps/{app}/machines` | 不要（コンテナ名=socket名で一意） |
| `app_exists()` (行2,332-) | Fly API `GET /apps/{app}` | `nerdctl inspect` |
| `detect_workspace_globs()` 等 | Fly.io無関係 | **そのまま移植可能** ✅ |

**DeployResult構造体の変更 (行603-610):**
```rust
// 現状
pub struct DeployResult {
    pub endpoint_url: String,       // "https://mcp-xxx.fly.dev"
    pub machine_id: Option<String>, // "mcp-xxx:machine_id_123"
    pub image_url: String,
    pub app_name: String,
}

// 変更後
pub struct DeployResult {
    pub socket_path: String,        // "/var/run/mcp/mcp-xxx.sock"
    pub image_url: String,
    pub container_name: String,     // "mcp-xxx"
}
```

**新規実装:** `crates/builder/src/containerd.rs`

```
デプロイフロー（新）:
1. GitHub からソース取得（既存ロジック流用）
2. Dockerfile 生成（fly.toml生成部分を削除）
3. nerdctl build -t ghcr.io/org/mcp-{id}:{sha} .
4. nerdctl push ghcr.io/org/mcp-{id}:{sha}
5. nerdctl rm -f mcp-{id}  ← 既存コンテナがあれば停止
6. nerdctl run -d \
     --name mcp-{id} --net=host --snapshotter nydus \
     -v /var/run/mcp:/var/run/mcp \
     -v /var/lib/mcp-data/{id}:/data \
     -e MCP_SOCKET_PATH=/var/run/mcp/{id}.sock \
     -m {memory_mb}m \
     ghcr.io/org/mcp-{id}:{sha}
7. Unix Socket 経由でヘルスチェック (initialize probe)
8. endpoint_url = "/var/run/mcp/mcp-{uuid-nohyphens}.sock" でDB更新
```

### 4.2 `flyio.rs` (197行) — 完全削除

Fly.io Machines API 直接呼び出し（DeployJobからの起動用）。

- `POST /apps` — アプリ作成
- `POST /apps/{app}/machines` — マシン作成・起動
- `GET /apps/{app}/machines/{id}` — 起動待ちポーリング

→ `containerd.rs` に統合。

### 4.3 `docker.rs` — 削除

コメントに "Note: docker module kept for reference but not used" とある。
`containerd.rs` の参考にして削除。

### 4.4 `main.rs` 🟠

| 行 | 現状 | 変更 |
|----|------|------|
| 15-16 | `mod flyctl; mod flyio;` | `mod containerd;` |
| 279, 358, 565 | コメント中の "Fly app" 言及 | コメント修正 |
| 387 | `flyctl::destroy_app(&ctx.config, &server.fly_app_name)` | `deployer.destroy(server.id)` |
| 535 | コメント "required for Fly.io to keep machine running" | コメント修正 |
| 623 | コメント "SIGTERM (Fly.io machine stop / deploy)" | コメント修正 |
| 855-866 | `flyctl::detect_workspace_globs()`, `flyctl::subdir_is_workspace_member()` 等 | `mod detect` に移植（ロジック変更なし） |
| 982-1088 | `flyctl::build_and_deploy()` 呼び出し全体 | `deployer.build_and_deploy(&job)` |
| 1136-1150 | `deploy_result.machine_id` を `server_regions` に保存 | `deploy_result.socket_path` を保存 |
| 1567 | `flyio::deploy(&ctx.config, &job)` | `deployer.deploy(&job)` |
| 1717 | `flyctl::destroy_app(&ctx.config, &job.app_name)` | `deployer.destroy(job.server_id)` |
| 1719 | コメント "Fly.io app {} destroyed" | コメント修正 |
| 1725 | コメント "Fly app {} destroyed" | コメント修正 |

---

## 5. `crates/queue/src/lib.rs` 🟡

コメントのFly.io言及を修正。フィールド名は変えても変えなくても可。

| 箇所 | 現状 | 変更 |
|------|------|------|
| 行13-14 | `BuildJob.app_name` コメント "Persisted Fly.io app name to deploy to" | コメント修正 |
| 行58 | `server.fly_app_name.clone()` | `server.container_name.clone()` (DBカラム名変更後) |
| 行82-83 | `DeployJob.app_name` コメント "Persisted Fly.io app name" | コメント修正 |
| 行103-105 | `DestroyJob` コメント "tear down a deleted server's Fly.io app" | コメント修正 |
| 行212 | `push_destroy_job()` コメント "tear down a deleted server's Fly.io app" | コメント修正 |

---

## 6. DB スキーマ 🟠

### 6.1 `mcp_servers.fly_app_name` → `container_name`

**現状 (`crates/db/src/models/server.rs` 行42):**
```rust
pub fly_app_name: String,  // "mcp-<uuid-without-dashes>"
```

値の形式（`mcp-{uuid-nohyphens}`）はそのまま使える。カラム名だけ変更する。

```sql
-- migrations/20240156000000_rename_fly_app_name.sql
ALTER TABLE mcp_servers RENAME COLUMN fly_app_name TO container_name;
```

**影響ファイル:**
- `crates/db/src/models/server.rs`: フィールド名変更 + `new_fly_app_name()` → `new_container_name()`
- `crates/db/src/repositories/server_repo.rs`: SELECT/INSERT句のカラム名変更 (行10, 19, 139, 147, 174)
- `crates/queue/src/lib.rs`: `server.fly_app_name` → `server.container_name` (行58)
- `crates/api/src/routes/servers.rs`: `existing.fly_app_name` → `existing.container_name` (行872)
- `crates/api/src/routes/deployments.rs`: `server.fly_app_name` → `server.container_name` (行187)
- `crates/builder/src/main.rs`: `server.fly_app_name` 参照全箇所

### 6.2 `server_regions.machine_id` 🟡

**現状 (`migrations/20240113000000_add_multi_region.sql`):**
```sql
machine_id VARCHAR(255),  -- Fly.io machine ID
```

現在の格納値は `"app_name:raw_machine_id"` 形式（FlyioRuntime.encode_id()で生成）。
`console.rs` がこの値を使って `fly_runtime.status()` / `fly_runtime.exec()` を呼ぶ。

**変更後:** コンテナ名（`mcp-{uuid-nohyphens}`）を格納。カラム名はそのまま可。
コメントのみ更新推奨：

```sql
-- migrations/20240157000000_update_machine_id_comment.sql
COMMENT ON COLUMN server_regions.machine_id IS
    'Container name (e.g. mcp-<uuid-nohyphens>). Formerly stored Fly.io machine ID.';
```

### 6.3 `endpoint_url` フォーマット変更

**現状:** `https://mcp-{uuid-nohyphens}.fly.dev/mcp`
**変更後:** `/var/run/mcp/mcp-{uuid-nohyphens}.sock`

カラム型変更不要。デプロイ時に自動上書きされるため移行SQLは任意。

### 6.4 リージョンコード

**現状:** Fly.ioリージョン（`nrt`=東京, `iad`=Ashburn, `lhr`=London等）がDBに保存されている。

**変更後:** 独自リージョン名（例: `ovh-jp`, `hetzner-de`）に変更。
ただしカラム定義変更は不要（VARCHAR(10)のまま使える）。
既存レコードはベアメタル移行時に一括UPDATE推奨：

```sql
UPDATE mcp_servers SET region = 'ovh-jp' WHERE region = 'nrt';
UPDATE server_regions SET region = 'ovh-jp' WHERE region = 'nrt';
```

---

## 7. `crates/container/src/` 🟠

### 7.1 現状の構成

```
crates/container/src/
├── lib.rs      (ContainerRuntime trait + pub mod flyio + pub use flyio::*)
├── docker.rs   (Docker実装)
└── flyio.rs    (Fly.io実装: FlyioRuntime, WireGuardConfig等)
```

**`lib.rs` の `pub use flyio::*` で公開されているもの:**
```rust
pub use flyio::{
    AppMetrics, ExecResponse, FlyioRuntime, MetricDataPoint, WireGuardConfig, WireGuardPeerInfo,
};
```
これらは `wireguard.rs` などから参照されている。

### 7.2 変更後の構成

```
crates/container/src/
├── lib.rs      (ContainerRuntime trait + pub mod nerdctl + pub use nerdctl::*)
├── docker.rs   (削除または保持)
├── flyio.rs    (削除)
└── nerdctl.rs  ← 新規追加
```

### 7.3 `nerdctl.rs` 新規実装（スケルトン）

```rust
pub struct NerdctlRuntime {
    pub socket_dir: String,   // "/var/run/mcp"
    pub data_dir: String,     // "/var/lib/mcp-data"
}

#[async_trait::async_trait]
impl ContainerRuntime for NerdctlRuntime {
    async fn create(&self, name: &str, config: ContainerConfig) -> Result<Container> {
        let socket_path = format!("{}/{}.sock", self.socket_dir, name);
        Command::new("nerdctl").args([
            "run", "-d", "--name", name, "--net=host", "--snapshotter=nydus",
            "-v", &format!("{}:{}", self.socket_dir, self.socket_dir),
            "-v", &format!("{}/{}:/data", self.data_dir, name),
            "-e", &format!("MCP_SOCKET_PATH={}", socket_path),
            "-m", &format!("{}m", config.memory_mb),
            &config.image,
        ]).status().await?;
        Ok(Container {
            id: name.to_string(),
            name: name.to_string(),
            status: ContainerStatus::Creating,
            endpoint_url: Some(socket_path),
        })
    }

    async fn start(&self, id: &str) -> Result<()> {
        Command::new("nerdctl").args(["start", id]).status().await?;
        Ok(())
    }

    async fn stop(&self, id: &str) -> Result<()> {
        Command::new("nerdctl").args(["stop", id]).status().await?;
        Ok(())
    }

    async fn delete(&self, id: &str) -> Result<()> {
        Command::new("nerdctl").args(["rm", "-f", id]).status().await?;
        Ok(())
    }

    async fn status(&self, id: &str) -> Result<ContainerStatus> {
        let out = Command::new("nerdctl")
            .args(["inspect", "--format", "{{.State.Status}}", id])
            .output().await?;
        Ok(match String::from_utf8_lossy(&out.stdout).trim() {
            "running" => ContainerStatus::Running,
            "exited" | "stopped" => ContainerStatus::Stopped,
            _ => ContainerStatus::Failed,
        })
    }

    async fn logs(&self, id: &str, tail: usize) -> Result<String> {
        let out = Command::new("nerdctl")
            .args(["logs", "--tail", &tail.to_string(), id])
            .output().await?;
        Ok(String::from_utf8_lossy(&out.stderr).to_string())
    }

    async fn exec(&self, id: &str, cmd: Vec<String>, _timeout: u32) -> Result<ExecResponse> {
        let out = Command::new("nerdctl")
            .arg("exec").arg(id).args(&cmd)
            .output().await?;
        Ok(ExecResponse {
            stdout: String::from_utf8_lossy(&out.stdout).to_string(),
            stderr: String::from_utf8_lossy(&out.stderr).to_string(),
            exit_code: out.status.code().unwrap_or(-1),
        })
    }
}

/// ベアメタルでの使用量サンプリング（課金用）
impl NerdctlRuntime {
    /// 起動中コンテナのメモリ使用量一覧（MB）
    pub async fn list_started_machine_memory_mb(&self, container_prefix: &str) -> Result<Vec<u32>> {
        // nerdctl stats --no-stream --format json | filter by name prefix
        let out = Command::new("nerdctl")
            .args(["stats", "--no-stream", "--format", "{{json .}}"])
            .output().await?;
        // メモリ使用量をパースして返す
        // ...
        Ok(vec![])
    }
}
```

---

## 8. `crates/api/src/` — API は Fly.io 継続だが修正が必要

> **注意:** APIはFly.io上で動き続けるが、MCPサーバーがベアメタルに移るため
> `FlyioRuntime` を使ったMCPサーバー操作がすべて機能しなくなる。
> APIからは直接 `nerdctl` を叩けないので、コンテナ操作はBuilderのHTTP APIを経由させる。

### 8.1 Builder に内部APIエンドポイントを追加 🟠

APIからコンテナ操作をするための内部エンドポイントをBuilderに追加する。
認証は既存の `NODEFLARE_SERVER_TOKEN` を使用。

```
POST /internal/exec          ← console.rs から呼ぶ (nerdctl exec の代替)
GET  /internal/metrics/{id}  ← servers.rs metrics から呼ぶ
GET  /internal/stats         ← 課金サンプラーから呼ぶ
```

### 8.2 `state.rs` 🟡

```rust
// fly_runtime を削除
// pub fly_runtime: Option<FlyioRuntime>,

// Builder内部APIのURLを保持するだけ
pub builder_internal_url: String,  // BUILDER_INTERNAL_URL env var
```

### 8.3 `main.rs` — `start_usage_sampler_task()` 🟠

**現状:** Fly Machines APIを呼んで起動中マシンのメモリ量を取得（行338-410）。

**変更後:** Builder の `/internal/stats` エンドポイントを呼ぶ。

```rust
// 変更前: Fly Machines API
let started = fly.list_started_machine_memory_mb(&app_name).await?;

// 変更後: Builder internal API
let resp = http.get(&format!("{}/internal/stats", builder_url))
    .bearer_auth(&server_token)
    .send().await?;
```

また `app_name_from_endpoint()` 関数（行197）の `.fly.dev` URL前提のパースを、
Unixソケットパス（`/var/run/mcp/mcp-xxx.sock` → `mcp-xxx`）対応に変更。

### 8.4 `routes/console.rs` 🟠

**現状:** `fly_runtime.exec()` → Fly Machines API でコンテナ内コマンド実行。
APIはFly.io上にあるためベアメタルの `nerdctl exec` を直接叩けない。

**変更後:** Builder の `/internal/exec` エンドポイントに転送。

```rust
// 変更前
fly_runtime.exec(&machine_id, body.command, body.timeout).await?

// 変更後: Builder API経由
let resp = state.http.post(&format!("{}/internal/exec", state.builder_internal_url))
    .bearer_auth(&server_token)
    .json(&ExecRequest { container_name, command: body.command, timeout: body.timeout })
    .send().await?;
```

`machine_id` カラムの値も変わる（`"app_name:machine_id"` → コンテナ名）ので
デコードロジックを削除し、コンテナ名をそのまま使う。

### 8.5 `routes/servers.rs` 🟡

| 行 | 現状 | 変更 |
|----|------|------|
| 864, 869 | コメント "Fly teardown", "Fly.io app" 等 | コメント修正 |
| 872 | `existing.fly_app_name.clone()` | `existing.container_name.clone()` |
| 881 | `state.fly_runtime.as_ref()` でインライン削除フォールバック | 削除 or Builder API経由に変更 |
| 882 | `fly_runtime.destroy_app(&app_name)` | DestroyJobのキュー投入のみ（インラインフォールバック削除） |
| 1125-1141 | `fly_runtime.get_metrics()` / endpoint_URLパース | Builder の `/internal/metrics/{id}` 呼び出しに変更 |

### 8.6 `routes/deployments.rs` 🟡

```rust
// 行187: fly_app_name → container_name (DBカラム名変更に追従)
app_name: server.container_name,
```

### 8.7 `routes/wireguard.rs` 🔴

全てFly.io GraphQL API依存。**機能を無効化**（503を返すか、ルーター登録解除）。

### 8.8 `middleware/rate_limit.rs` 🟢 変更なし

APIはFly.io継続のため `fly-client-ip` ヘッダーは引き続き有効。変更不要。

---

## 15. `crates/proxy/src/rate_limit.rs` 🟡

**現状 (行68-94):**
```rust
/// (and can inject `fly-client-ip`/`cf-connecting-ip` headers when we are not
/// infrastructure appended, and only honour `fly-client-ip` when explicitly told
/// we sit behind Fly's edge (`PROXY_BEHIND_FLY=true`).
let behind_fly = std::env::var("PROXY_BEHIND_FLY")
    .map(|v| v == "true" || v == "1")
    .unwrap_or(false);
if behind_fly {
    if let Some(fly_ip) = headers.get("fly-client-ip").and_then(...) {
        return fly_ip.to_string();
    }
}
```

**変更後:**
- `PROXY_BEHIND_FLY` 環境変数は `PROXY_BEHIND_PROXY` に変更（または削除）
- `fly-client-ip` ヘッダーを削除し `X-Real-IP` を使用
- デフォルトは `false`（ベアメタルではCaddyが `X-Real-IP` を設定）

---

## 16. `crates/proxy/src/affinity.rs` 🔴

**現状 (293行):**
Fly.io Machines API (`https://api.machines.dev/v1/apps/{app}/machines`) を呼び出して
`fly-force-instance-id` ヘッダーを付与。セッションをマシンにピン留め。

**ベアメタルでの状況:**
- 各MCPサーバー = コンテナ1つ = 「マシン」概念なし
- Proxy自体がシングルインスタンス → セッションアフィニティ問題は発生しない
- `fly-force-instance-id` ヘッダーは送信先が解釈しない

**変更後:** 大半を削除して簡素化。

削除する実装:
- `fetch_machines()` — Fly Machines API呼び出し
- `machine_list()` — Redisキャッシュされたマシン一覧
- `pick_machine()` — ラウンドロビン選択
- `app_name_from_target()` — `.fly.dev` URLからアプリ名抽出
- `FlyMachine`, `Machine` 構造体

残す実装（将来の水平スケール用）:
- `lookup_session()` / `store_session()` — Redisでセッション管理
- `decide()` — ただし `forced_machine` は常に `None`
- `capture_session()` — セッションIDをRedisに保存

---

## 17. `crates/proxy/src/main.rs` 🟠

### 17.1 endpoint_url処理 (行500-527)

**現状:**
```rust
let target_url = format!("{}/{}{}", endpoint_url.trim_end_matches('/'), mcp_path, query);
// → "https://mcp-xxx.fly.dev/mcp?..."
```

**変更後:** Unix Socketクライアントで送信。

```rust
// endpoint_url が "/var/run/mcp/mcp-xxx.sock" の場合
let socket_path = endpoint_url;
let uri = format!("http://localhost/{}{}", mcp_path, query);
// hyperlocal::UnixClientExt でリクエスト送信
```

### 17.2 `fly-force-instance-id` ヘッダー (行1054-1058)

**現状:**
```rust
if let Some(machine) = affinity.forced_machine.as_deref() {
    if let Ok(value) = HeaderValue::from_str(machine) {
        headers.insert("fly-force-instance-id", value);
    }
}
```

**変更後:** `affinity.forced_machine` が常に `None` になるため、このブロックは実質無効化。
コードは残してもよいが削除推奨。

### 17.3 IPv6バインディング (行228-232) 🟡

```rust
// 現状コメント: "Fly private networking is IPv6-only, so proxy reaches us over 6PN"
// コメント修正のみ。コード自体 ("[::]:{port}") はベアメタルでも使える（IPv4/IPv6 dual-stack）
```

### 17.4 Cargo.toml への追加

```toml
# Unix Socket経由HTTPクライアント
hyper = { version = "1.5", features = ["client", "http1"] }
hyper-util = { version = "0.1", features = ["client", "client-legacy", "tokio"] }
hyperlocal = "0.9"
http-body-util = "0.1"
```

---

## 18. `crates/proxy/src/code_runner.rs` 🟡

**現状 (行1コメント):**
```rust
//! Client for the sandboxed code runner (a dedicated Deno-on-Firecracker Fly app).
```

**変更:** コメント修正 + `PROXY_CODE_RUNNER_URL` 変更。

```bash
# 変更前 (Fly.io内部DNS)
PROXY_CODE_RUNNER_URL=https://mcp-code-runner.internal:8080

# 変更後 (同一サーバー上のコンテナ)
PROXY_CODE_RUNNER_URL=http://localhost:8082
```

---

## 19. Fly.io TOML ファイル群の整理

現在リポジトリに存在する7ファイル:

| ファイル | 用途 | 対応 |
|---------|------|------|
| `fly.toml` (=`fly.api.toml`) | APIサーバーデプロイ | **継続利用**（API は Fly.io 残留） |
| `fly.api.toml` | APIサーバーデプロイ | **継続利用** |
| `fly.proxy.toml` | プロキシデプロイ | 廃止 → systemd化 |
| `fly.builder.toml` | ビルダーデプロイ | 廃止 → systemd化 |
| `fly-builder.toml` | ビルダーデプロイ（旧版） | 廃止 |
| `fly.code-runner.toml` | コードランナーデプロイ | 廃止 → nerdctl run |
| `fly.web.toml` | Webフロント（未使用） | 廃止（既に未使用） |

**systemdサービスファイル例（ベアメタル移行分）:**

```ini
# /etc/systemd/system/mcp-cloud-proxy.service
[Unit]
Description=NodeFlare MCP Proxy
After=network.target containerd.service

[Service]
ExecStart=/opt/nodeflare/bin/mcp-proxy
Restart=always
EnvironmentFile=/opt/nodeflare/env/proxy.env

[Install]
WantedBy=multi-user.target
```

---

## 20. フロントエンド (`apps/web/src/`) 🟡

**Fly.io言及箇所（コメント・テキスト）:**

| ファイル | 行 | 内容 | 対応 |
|---------|-----|------|------|
| `app/docs/page.tsx` | 229 | アーキテクチャ図 "pinned to one Fly machine" | テキスト修正 |
| `app/docs/page.tsx` | 242 | "GitHub repo -> ... -> Fly image -> deploy" | テキスト修正 |
| `app/docs/page.tsx` | 272 | "so Fly marks the machine unhealthy" | テキスト修正 |
| `app/legal/privacy/page.tsx` | 79 | "Fly.io" プライバシーポリシーの外部サービス列挙 | テキスト修正（Fly.io→削除またはOVHcloud） |
| `app/dashboard/servers/[id]/page.tsx` | 253 | コメント "Fly app destruction" | コメント修正 |

WireGuard機能を削除する場合: VPN設定UIページを非表示にするコンポーネント変更が必要。

---

## 21. `services/code-runner/` 🟡

**`main.ts` のFly.io依存箇所:**

| 行 | 現状 | 変更 |
|----|------|------|
| 1 (コメント) | "a dedicated Deno-on-Firecracker Fly app" | コメント修正 |
| 258 | コメント "Bind IPv6 (dual-stack): Fly private networking is IPv6-only" | コメント修正 |
| 260 | `Deno.serve({ hostname: "::" })` | ベアメタルではIPv4可。コメント修正のみ |

**デプロイ方法の変更:**
```bash
# 変更前
fly deploy -c fly.code-runner.toml

# 変更後
nerdctl build -t ghcr.io/org/mcp-code-runner:latest services/code-runner/
nerdctl push ghcr.io/org/mcp-code-runner:latest
nerdctl run -d \
  --name mcp-code-runner \
  --net=host \
  -e PORT=8080 \
  -e CODE_RUNNER_TOKEN=${CODE_RUNNER_TOKEN} \
  -e RUNNER_MAX_HEAP_MB=128 \
  ghcr.io/org/mcp-code-runner:latest
```

---

## 22. `crates/builder/assets/stdio-adapter.cjs` 🟡

**Fly.io依存箇所（コメントのみ）:**

```javascript
// 現状 (行594付近)
// "Fail (503) when the child is dead so Fly.io marks the machine unhealthy instead of silently 500-ing."
// 変更後
// "Fail (503) when the child is dead so Caddy/healthcheck detects the failure."
```

動作コードの変更なし。

---

## 23. スケールtoゼロ（新規実装）

Fly.ioの自動スリープ相当機能をベアメタルで実装する。

```
アイドル検出フロー:
  Proxyリクエスト受信
    → Redis に last_request:{server_id} = now() を保存 (TTL 10分)
  
  Scaler (5分毎のバックグラウンドタスク):
    → running状態のサーバー一覧取得
    → last_request が5分以上前 → nerdctl stop {container}
    → DBのstatus を 'stopped' に更新
  
  Proxyリクエスト受信 (stopped server):
    → socket_path が存在しない → nerdctl start {container}
    → socket_path が出現するまで最大5秒待機
    → リクエスト転送
```

データは `/var/lib/mcp-data/{container-name}/` にbind mountで永続化されるため、
コンテナをstop/rmしてもデータは失われない。

---

## 24. 移行手順（実行順序）

### Phase 1: インフラ準備（コード変更なし）
1. OVHcloud Debian 12 インストール
2. `docs/baremetal/os-layer.md` の設定適用
3. containerd + crun + Nydus インストール
4. Caddy インストール・設定（`*.nodeflare.tech` → localhost:8081）
5. ghcr.io レジストリ用 PAT 取得

### Phase 2: DBマイグレーション
1. `20240156000000_rename_fly_app_name.sql` — `fly_app_name` → `container_name`
2. `20240157000000_update_machine_id_comment.sql` — `server_regions.machine_id` コメント更新

### Phase 3: 共有クレート変更（API・Builder両方に影響）
1. `crates/common/src/config.rs` — `FlyioConfig` → `ContainerConfig`
2. `crates/db/src/models/server.rs` — `fly_app_name` → `container_name`
3. `crates/db/src/repositories/server_repo.rs` — SQLカラム名更新
4. `crates/queue/src/lib.rs` — コメント修正、`fly_app_name` 参照を `container_name` に

### Phase 4: `crates/container/` に NerdctlRuntime 追加
1. `nerdctl.rs` 新規実装（ContainerRuntime trait + list_started_machine_memory_mb）
2. `flyio.rs` 削除
3. `lib.rs` の pub use 更新

### Phase 5: Builder 書き直し ← **最大の作業**
1. `flyctl.rs`, `flyio.rs` 削除
2. `containerd.rs` 新規実装（build/deploy/destroy/verify/logs）
3. Builder に内部APIエンドポイント追加（`/internal/exec`, `/internal/metrics/{id}`, `/internal/stats`）
4. `docker/Dockerfile.builder` の flyctl → nerdctl
5. `main.rs` の参照更新

### Phase 6: Proxy 変更
1. `affinity.rs` 簡素化（Fly Machines API呼び出し削除）
2. `crates/proxy/Cargo.toml` に hyperlocal 追加
3. Unix Socketクライアント実装
4. `main.rs` の endpoint_url処理変更（HTTPS URL → Unix Socket）
5. `rate_limit.rs` の `PROXY_BEHIND_FLY` → `X-Real-IP`（Caddy経由）に変更
6. `code_runner.rs` の `PROXY_CODE_RUNNER_URL` をローカルに変更

### Phase 7: API 変更（Fly.io 上で再デプロイ）
1. `state.rs` の `fly_runtime` 削除、Builder内部APIのURL保持に変更
2. `main.rs` の `start_usage_sampler_task` → Builder `/internal/stats` 呼び出しに
3. `servers.rs` 更新（`fly_app_name` → `container_name`、メトリクス取得変更）
4. `deployments.rs` 更新（`fly_app_name` → `container_name`）
5. `console.rs` 更新（Builder `/internal/exec` 経由に変更）
6. `wireguard.rs` 無効化（503返却）
7. API を Fly.io に **再デプロイ**

### Phase 8: コードランナー移行
1. ベアメタル上でビルド・push・起動（nerdctl run）
2. `PROXY_CODE_RUNNER_URL=http://localhost:8082` 設定

### Phase 9: ベアメタルサービス起動
1. Builder を systemd で起動
2. Proxy を systemd で起動
3. Code Runner を nerdctl で起動
4. 動作確認（新規MCPサーバーをテストデプロイ）

### Phase 10: 既存MCPサーバー全再デプロイ
1. 各サーバーをベアメタル上で `nerdctl build && run`
2. `endpoint_url` をUnix Socketパスに更新
3. `server_regions.machine_id` をコンテナ名に更新

### Phase 11: DNS切り替え（ダウンタイム最小化）
1. TTL を事前に短縮（300s → 60s）
2. `*.nodeflare.tech`（Proxy用）→ ベアメタルIP に変更
3. `api.nodeflare.tech` は **変更なし**（Fly.io継続）
4. Caddy が Let's Encrypt 証明書を自動取得
5. 旧 Fly.io アプリ（proxy, builder, code-runner）を削除

### Phase 12: フロントエンド更新
1. `docs/page.tsx` のアーキテクチャ図テキスト更新
2. `privacy/page.tsx` のFly.io言及を OVHcloud に変更
3. WireGuard UI 非表示

---

## 25. 変更しない箇所

| コンポーネント | 理由 |
|--------------|------|
| `nodeflare-api.fly.dev` のデプロイ自体 | Fly.io継続 |
| API の認証・課金ロジック | Fly.io依存なし |
| API の GitHub連携 / OAuth | Fly.io依存なし |
| API の Stripe課金 | Fly.io依存なし |
| API の `middleware/rate_limit.rs` | APIはFly.io継続のため `fly-client-ip` 有効のまま |
| `fly.toml` / `fly.api.toml` | APIデプロイ用として継続利用 |
| Proxy の認証・スコープ管理ロジック | Fly.io依存なし |
| Proxy の tools/list, tools/call 処理 | Fly.io依存なし |
| Proxy の search_tools, run_code モード | Fly.io依存なし（URLのみ変更） |
| フロントエンド全体（機能） | Vercel継続、テキスト修正のみ |
| DBスキーマ（上記以外） | Fly.io依存なし |
| Redis / PostgreSQL の接続先 | Upstash / Neon 継続 |
| services/code-runner/main.ts（ロジック） | コメント修正のみ |
| stdio-adapter.cjs（ロジック） | コメント修正のみ |
| `crates/detect/` | Fly.io依存なし |

---

## 26. リスクと注意事項

### WireGuard機能削除
- 現在使用中のユーザーがいる場合は事前通知が必要
- フロントエンドのVPN設定ページを非表示にする

### `server_regions.machine_id` 形式変更
- console.rs がこのカラムを直接デコードしている
- 既存レコードは `"app_name:machine_id"` 形式 → ベアメタル移行後に `container_name` で上書き
- Phase 9（全再デプロイ）で自然に更新される

### セッションアフィニティ（将来の水平スケール）
- 現状: Proxy単一インスタンス → 問題なし
- 複数Proxy時: Redisベースのセッションルーティングが必要（`affinity.rs` にロジック追加）

### `fly-client-ip` ヘッダー
- ベアメタルにはCaddyがいるため `X-Real-IP` を使用
- セキュリティ: Caddyが信頼できるリバースプロキシとして設定されていることが前提

### コンテナレジストリ認証
- ビルドしたイメージを ghcr.io に push するための PAT（`CONTAINER_REGISTRY_TOKEN`）が必要
- Builder の secrets/env に設定

### ポート管理
- `--net=host` モードで各MCPサーバーを起動する場合、ポート衝突を避けるために動的割り当てが必要
- Unix Socketを使えばポート管理が不要（推奨）

### リージョンコード
- 既存の `nrt`/`iad` 等がDBに残っている
- 新しいリージョン名（`ovh-jp`等）に移行する際は既存ユーザーデータの一括UPDATE必要
