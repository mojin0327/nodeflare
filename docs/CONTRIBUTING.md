# Contributing to Nodeflare

Thank you for your interest in contributing to Nodeflare! This document provides guidelines and instructions for contributing.

## Getting Started

1. Fork the repository
2. Clone your fork locally
3. Set up the development environment (see [README.md](README.md))
4. Create a new branch for your changes

## Development Setup

### Prerequisites

- Rust (stable)
- Node.js 20.x or later
- PostgreSQL (or [Neon](https://neon.tech) / [Supabase](https://supabase.com))
- Redis (or [Upstash](https://upstash.com))
- GitHub OAuth App (for authentication)

### Setup Steps

```bash
# Clone your fork
git clone https://github.com/YOUR_USERNAME/nodeflare.git
cd nodeflare

# Copy environment file and configure
cp .env.example .env
# Edit .env with your database, Redis, and API credentials

# Build Rust backend
cargo build

# Install and run frontend
cd apps/web
npm install
npm run dev
```

### Required Environment Variables

See `.env.example` for all required variables. Key ones include:
- `DATABASE_URL` - PostgreSQL connection string
- `REDIS_URL` - Redis connection string
- `JWT_SECRET` - Secret for JWT signing
- `GITHUB_CLIENT_ID` / `GITHUB_CLIENT_SECRET` - GitHub OAuth credentials

## Making Changes

### Code Style

- **Rust**: Follow standard Rust conventions, use `cargo fmt` and `cargo clippy`
- **TypeScript**: Follow the existing code style, use ESLint

### Commit Messages

Use clear, descriptive commit messages:

```
feat: add support for custom domains
fix: resolve authentication timeout issue
docs: update API documentation
refactor: simplify proxy routing logic
```

### Pull Requests

1. Create a focused PR that addresses a single concern
2. Include a clear description of the changes
3. Reference any related issues
4. Ensure all tests pass
5. Update documentation if needed

## Reporting Issues

### Bug Reports

Please include:
- A clear description of the bug
- Steps to reproduce
- Expected vs actual behavior
- Environment details (OS, Rust version, Node version)

### Feature Requests

Please include:
- A clear description of the feature
- Use case / motivation
- Any proposed implementation ideas

## Code of Conduct

Please read and follow our [Code of Conduct](CODE_OF_CONDUCT.md).

## Questions?

Feel free to open a Discussion or Issue if you have questions.

## License

By contributing, you agree that your contributions will be licensed under the MIT License.
