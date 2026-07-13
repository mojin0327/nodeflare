# Upstream OAuth 自動化 実装仕様

## 概要

OAuth必須のMCPサーバー（Google Drive、GitHub等）を NodeFlare上にデプロイした際、
ユーザーが「Claudeで使って」と言うだけで自動的に認可フローが走り、
リフレッシュトークンの取得・保存・注入まで全自動化する。

再起動不要・マルチリージョン対応・ゼロダウンタイムのRedisポーリング方式を採用。

---

## 現状の整理

### 既存実装（変更不要）

| コンポーネント | 場所 | 役割 |
|---|---|---|
| `/.well-known/oauth-protected-resource` | `crates/proxy/src/main.rs` | Claude がauth serverを発見するエンドポイント（RFC 9728） |
| `WWW-Authenticate: Bearer resource_metadata=...` | `crates/proxy/src/main.rs` | 401時にClaudeへPRM URLを伝えるヘッダー |
| `/.well-known/oauth-authorization-server` | `crates/api/src/routes/oauth.rs` | NodeFlareの認可サーバーメタデータ |
| `POST /oauth/register` | `crates/api/src/routes/oauth.rs` | DCR（Dynamic Client Registration） |
| `GET /internal/mcp-token/:server_id` | `crates/api/src/routes/mcp_tokens.rs` | コンテナがRedisからtokenを取得するAPI |
| `PUT /internal/mcp-token/:server_id` | `crates/api/src/routes/mcp_tokens.rs` | コンテナがRedisにtokenを保存するAPI |
| `NODEFLARE_SERVER_TOKEN` | `crates/builder/src/main.rs` | コンテナがinternal APIを叩く際の認証トークン |

### 既存実装の問題点（修正が必要）

- デプロイ時にenv varでtokenを注入しているが、OAuth完了後に**コンテナ再起動が必要になる**設計になっている
- 再起動はマルチリージョン全落ち・競合状態・ダウンタイムを引き起こすため本番では使えない

---

## 採用するアーキテクチャ: Redisポーリング方式

```
コンテナ起動時 → Redisにtokenがあれば読む → なければ401モードで待機
OAuth完了    → Redisに書き込む → コンテナが次のポーリングで検知 → 子プロセスのみ再起動
再起動不要、マルチリージョンも各自独立
```

---

## 全体フロー

```
[デプロイ時]
1. OAUTH_PROVIDER=google をenv varとして注入（MCPサーバー種別に応じて）
2. NODEFLARE_SERVER_TOKEN, NODEFLARE_API_INTERNAL_URL を注入（既存）
3. tokenはenv varで注入しない（Redisから動的に取得する）

[初回リクエスト: tokenなし]
Claude → NodeFlare Proxy
           ↓ Redisに mcp:oauth:{server_id} なし
           ↓ （プロキシはtoken有無を知らない。コンテナが401を返す）
        stdio-adapter → 子プロセス未起動 or 401モード
           ↓
        401 Unauthorized
        WWW-Authenticate: Bearer resource_metadata="https://proxy/.well-known/..."
           ↓
        Claudeが /.well-known/oauth-protected-resource を取得（プロキシが返す: 既存）
           ↓
        ClaudeがNodeFlareの認可サーバーを発見（/.well-known/oauth-authorization-server: 既存）
           ↓
        Claude が DCR実行 → /oauth/authorize のURLを構築
           ↓
        Claudeがチャット上にリンクを表示: 「Google Driveへの接続が必要です [認証する]」
           ↓
        ユーザーがタップ

[OAuth フロー]
        NodeFlare /authorize
           ↓ MCPサーバーに紐づくOAUTH_PROVIDERを確認
           ↓ → Googleの認証画面へリダイレクト
        ユーザーがGoogleで同意
           ↓
        Google → NodeFlare /oauth/upstream-callback?code=...&server_id=...
           ↓
        NodeFlareがcodeをaccess_token + refresh_tokenに交換
           ↓
        Redis: SET mcp:oauth:{server_id} {access_token, refresh_token, expires_at}
           ↓
        ユーザーをダッシュボードにリダイレクト（or Claudeに通知）

[コンテナ側の検知（ポーリング）]
        stdio-adapter が30秒ごとにGET /internal/mcp-token/:server_id を叩く
           ↓ tokenが存在する
        子プロセス(MCP server)を起動（or SIGHUPで再読み込み）
           ↓
        次回のClaudeリクエストから正常に応答

[以降のリクエスト: tokenあり]
Claude → NodeFlare Proxy → stdio-adapter → 子プロセス → 正常応答

[バックグラウンドでのtoken更新]
        NodeFlare APIのバックグラウンドジョブ（毎時）
           ↓ expires_atが5日以内のtokenを検索
           ↓ Googleにrefresh_tokenでリフレッシュ
           ↓ Redisを更新
        stdio-adapterの次回ポーリングで子プロセスが新tokenを使用
```

---

## 実装が必要なコンポーネント

### 1. NodeFlare API: upstream OAuthコールバックエンドポイント（新規）

**場所**: `crates/api/src/routes/oauth.rs` に追加

```
GET /oauth/upstream-callback
  query params: code, state (state にserver_idをエンコード)
```

処理内容:
- stateからserver_idを取り出す
- DBからMCPサーバーのOAUTH_PROVIDER設定を取得
- providerのtoken endpointにcodeを送信してaccess_token + refresh_tokenを取得
- Redisに保存: `mcp:oauth:{server_id}` = `{access_token, refresh_token, expires_at}`
- ユーザーをダッシュボードにリダイレクト

**注意点**:
- callbackはNodeFlareのAPIドメイン（`api.nodeflare.com`）で受ける。MCPコンテナ経由ではない
- stateパラメータはCSRF対策としてHMACで署名する
- server_idとユーザーIDの紐付けをDB側で検証する（他ユーザーのサーバーにtokenを書けないよう）

---

### 2. NodeFlare API: /authorize の拡張（既存に追記）

**場所**: `crates/api/src/routes/oauth.rs` の `authorize` 関数

現状: NodeFlare自身のOAuthフロー（Claude→NodeFlare認証）のみ

追加:
- request paramsに `server_id` が含まれる場合 → upstreamプロバイダーOAuthフローへ分岐
- server_idに対応するOAUTH_PROVIDERをDBから取得
- 対応するプロバイダーの認証URLにリダイレクト（Google, GitHub等）
- stateにserver_id + user_id + nonce をエンコードして渡す

---

### 3. MCPサーバーごとのprovider設定（DBスキーマ追加）

**新規migration**: `migrations/YYYYMMDD_add_server_oauth_provider.sql`

```sql
ALTER TABLE servers
  ADD COLUMN upstream_oauth_provider VARCHAR(32),  -- 'google', 'github', NULL
  ADD COLUMN upstream_oauth_scopes    TEXT[];       -- ['https://www.googleapis.com/auth/drive']
```

デプロイ時にこのカラムを見て `OAUTH_PROVIDER` env varを注入する。

---

### 4. stdio-adapter: Redisポーリング + 401モード（既存ファイルに追記）

**場所**: `crates/builder/assets/stdio-adapter.cjs`

追加する処理:

```
起動時:
  OAUTH_PROVIDER が未設定 → 何もしない（通常通り子プロセス起動）
  OAUTH_PROVIDER が設定済み → Redisからtoken取得を試みる
    tokenあり → GOOGLE_REFRESH_TOKEN等をenv varにセット → 子プロセス起動
    tokenなし → 401モードで起動（子プロセスは起動しない）

401モード中:
  MCP requestに全て401を返す（WWW-Authenticateヘッダー付き）
  /.well-known/oauth-protected-resource は返さない（プロキシが担当）
  30秒ごとにGET /internal/mcp-token/:server_id をポーリング
    tokenが来たら → env varをセット → 子プロセスを起動 → 通常モードに移行

通常モード中:
  60秒ごとにRedisからtokenをポーリング
    tokenが更新されていたら → 子プロセスにSIGHUP（or 再起動）
    tokenがなくなっていたら → 401モードに戻る

ポーリング実装:
  http.request to NODEFLARE_API_INTERNAL_URL + /internal/mcp-token/:server_id
  Bearer: NODEFLARE_SERVER_TOKEN
  SERVER_ID はenv var（デプロイ時に注入）
```

**ポーリング間隔**:
- 401モード中: 30秒（早く復帰させたい）
- 通常モード中: 60秒（token更新の検知）

---

### 5. builderでのenv var注入変更（既存ファイルに追記）

**場所**: `crates/builder/src/main.rs` と `crates/builder/src/flyctl.rs`

変更内容:
- `GOOGLE_REFRESH_TOKEN` 等をenv varで注入するのをやめる
- 代わりに `OAUTH_PROVIDER` と `SERVER_ID` のみ注入
- tokenはstdio-adapterがランタイムでRedisから取得する

注入するenv var:
```
OAUTH_PROVIDER=google          (DBのupstream_oauth_providerから)
SERVER_ID={server_id}          (新規追加)
NODEFLARE_SERVER_TOKEN=...     (既存)
NODEFLARE_API_INTERNAL_URL=... (既存)
```

---

### 6. バックグラウンドtokenリフレッシュジョブ（新規）

**場所**: `crates/api/src/main.rs` にtokio::spawnで追加

処理内容（毎時実行）:
- RedisのキーパターンでOAuthトークンを全スキャン
- `expires_at` が5日以内のものを抽出
- 対応するprovider（Google等）のtoken endpointにrefresh_tokenでリフレッシュ
- Redisを新しいtokenで更新
- リフレッシュ失敗（refresh token失効）時: DBにフラグを立てる + ユーザーに再認証通知メールを送る

---

## 実装の優先順位

| 優先度 | タスク | 依存 |
|---|---|---|
| 1 | DBスキーマ: `upstream_oauth_provider`, `upstream_oauth_scopes` カラム追加 | なし |
| 2 | upstream OAuthコールバックエンドポイント (`/oauth/upstream-callback`) | DBスキーマ |
| 3 | `/authorize` の upstream分岐ロジック | コールバック |
| 4 | stdio-adapter: 401モード + Redisポーリング | コールバック |
| 5 | builder: `SERVER_ID` 注入 + token env var注入削除 | stdio-adapter |
| 6 | バックグラウンドリフレッシュジョブ | コールバック |

---

## セキュリティ考慮事項

- **stateパラメータ**: HMAC署名でCSRF防止。server_id + user_id + nonce を含める
- **callback検証**: server_idとログインユーザーIDの紐付けをDB側で必ず検証
- **callbackドメイン**: `api.nodeflare.com` のみ受け付ける。MCPコンテナ経由は不可
- **tokenの暗号化**: Redis保存時は暗号化（既存のCryptoServiceを使用）
- **ポーリングレート制限**: `GET /internal/mcp-token/:server_id` に10req/分のレート制限追加

---

## 対応するOAuthプロバイダー（初期）

| provider値 | 対応サービス | スコープ例 |
|---|---|---|
| `google` | Google Drive, Gmail, Calendar等 | `https://www.googleapis.com/auth/drive` |
| `github` | GitHub API | `repo`, `read:org` |

プロバイダーごとのclient_id/secretはNodeFlare側のenv varとして管理する。
（ユーザーが自前のOAuthアプリを使いたい場合のカスタム設定は将来対応）

---

## 関連ファイル

```
crates/
  api/src/routes/oauth.rs         既存: /authorize, /token, /register, callback追加
  api/src/routes/mcp_tokens.rs    既存: GET/PUT /internal/mcp-token/:server_id
  api/src/main.rs                 バックグラウンドジョブ追加
  proxy/src/main.rs               既存: PRM, 401, WWW-Authenticate（変更不要）
  builder/src/main.rs             SERVER_ID注入追加
  builder/src/flyctl.rs           token env var注入削除
  builder/assets/stdio-adapter.cjs 401モード + Redisポーリング追加

migrations/
  YYYYMMDD_add_server_oauth_provider.sql  新規
```
