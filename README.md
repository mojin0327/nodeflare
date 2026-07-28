# Nodeflare

Deploy, manage, and scale MCP servers — Vercel for MCP.

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

🌐 **[Try it now → nodeflare.tech](https://nodeflare.tech)**

## What is Nodeflare?

Nodeflare is an MCP (Model Context Protocol) hosting platform that lets you deploy any MCP server with just a GitHub URL. It automatically converts stdio-based MCP servers to the Streamable HTTP transport, making them accessible from remote AI assistants like Claude.

**Key Features:**
- 🚀 **One-click deployment** — Just paste a GitHub URL
- 🔄 **Automatic stdio→Streamable HTTP conversion** — Works with any MCP server
- 🔐 **Built-in authentication** — API keys, OAuth 2.0, scoped permissions
- 📊 **Access logging** — Full audit trail for enterprise compliance
- 🌍 **Global edge deployment** — Powered by bare-metal + Fly.io

## Architecture

```
[Vercel]              [Fly.io]
Next.js Frontend      API Server (Axum)
       │                    │
       └──────────┬──────────┘
                  │ (Neon PostgreSQL + Upstash Redis)
                  │
            [Bare-metal]
         Caddy (TLS termination)
                  │
          Proxy Gateway (Axum)
          Builder Worker (Axum)
                  │
       containerd / Unix Socket
        ┌────┬────┬────┐
     MCP Srv MCP Srv ...  (per-server containers)
```

## Tech Stack

- **Backend**: Rust (Axum, SQLx, Tokio)
- **Frontend**: Next.js 16, TypeScript, Tailwind CSS
- **Database**: [Neon](https://neon.tech) (Serverless PostgreSQL)
- **Cache/Queue**: [Upstash](https://upstash.com) (Serverless Redis)
- **Container Runtime**: containerd (bare-metal) / Fly.io (API)
- **Billing**: Stripe

## Project Structure

```
nodeflare/
├── crates/
│   ├── api/            # Main API server (Axum)
│   ├── auth/           # JWT, OAuth, API keys, encryption
│   ├── billing/        # Stripe billing
│   ├── builder/        # Build worker (Docker, containerd)
│   ├── common/         # Shared types, config, errors
│   ├── container/      # Container runtime abstraction
│   ├── db/             # Database models & repositories
│   ├── email/          # Email sending (Resend)
│   ├── github/         # GitHub App integration
│   ├── mcp-runtime/    # MCP protocol types
│   ├── proxy/          # MCP Proxy gateway
│   └── queue/          # Job definitions
├── apps/
│   └── web/            # Next.js frontend
├── migrations/         # Database migrations
└── docker/             # Dockerfiles
```

## Getting Started

### Prerequisites

- Rust 1.75+
- Node.js 20+
- Docker & Docker Compose

### Local Development

1. **Clone and setup**

```bash
git clone https://github.com/nodeflare-dev/nodeflare.git
cd nodeflare
cp .env.example .env
```

2. **Setup Neon (PostgreSQL)**

- Create an account at [neon.tech](https://neon.tech)
- Create a new project
- Copy the connection string to `.env`:
  ```
  DATABASE_URL=postgresql://user:pass@ep-xxx.region.aws.neon.tech/dbname?sslmode=require
  ```

3. **Setup Upstash (Redis)**

- Create an account at [upstash.com](https://upstash.com)
- Create a new Redis database
- Copy the connection string to `.env`:
  ```
  REDIS_URL=rediss://default:xxx@xxx.upstash.io:6379
  ```

4. **Configure environment variables**

Set GitHub OAuth and encryption keys in `.env` (see Configuration section below)

5. **Run database migrations**

```bash
cargo install sqlx-cli
sqlx migrate run
```

6. **Start backend services**

```bash
# Terminal 1: API server
cargo run --bin mcp-api

# Terminal 2: Proxy gateway
cargo run --bin mcp-proxy

# Terminal 3: Builder worker
cargo run --bin mcp-builder
```

7. **Start frontend**

```bash
cd apps/web
npm install
npm run dev
```

8. **Open in browser**

Navigate to http://localhost:3000

## Configuration

### Required

| Variable | Description |
|----------|-------------|
| `DATABASE_URL` | PostgreSQL connection string |
| `REDIS_URL` | Redis connection string |
| `JWT_SECRET` | JWT signing secret (64+ bytes) |
| `ENCRYPTION_KEY` | AES-256 key for secrets (32 bytes, base64) |
| `GITHUB_CLIENT_ID` | GitHub OAuth App client ID |
| `GITHUB_CLIENT_SECRET` | GitHub OAuth App client secret |
| `GITHUB_APP_ID` | GitHub App ID for repo access |
| `GITHUB_APP_PRIVATE_KEY` | GitHub App private key (PEM) |

### Stripe (Billing)

| Variable | Description |
|----------|-------------|
| `STRIPE_SECRET_KEY` | Stripe API secret key |
| `STRIPE_WEBHOOK_SECRET` | Stripe webhook secret |
| `APP_URL` | App URL for Stripe redirects |

### Email (Resend)

| Variable | Description |
|----------|-------------|
| `RESEND_API_KEY` | Resend API key |
| `EMAIL_FROM` | Sender email address |

### Generate Keys

```bash
# JWT Secret
openssl rand -base64 64

# Encryption Key
openssl rand -base64 32
```

## Deployment

### Production

```bash
# API (Fly.io)
fly deploy -c fly.api.toml

# Proxy / Builder / Code Runner → bare-metal (systemd)
# See docs/baremetal/migration.md

# Frontend → Vercel (auto-deploy on push)
```

## MCP Proxy

The proxy gateway handles MCP requests via **subdomain-based routing**:

```
POST https://{server-slug}.nodeflare.tech/mcp
Authorization: Bearer {access-token}
```

Example: If server slug is `my-notion-mcp`:
```
https://my-notion-mcp.nodeflare.tech/mcp
```

Features:
- **Subdomain-based routing** — Clean URLs like Vercel
- Access token / OAuth authentication
- Rate limiting (sliding window)
- Request logging
- Tool-level permissions

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## License

MIT
