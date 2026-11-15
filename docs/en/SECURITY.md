# 🔒 Nebula ID Security

> This document describes Nebula ID's security design, supply-chain gates, vulnerability reporting process, and security best practices. The content is based on the actual implementation in the repository code, CI configuration, and Git hooks.

[🏠 Home](../../README.md) • [🏗️ Architecture](ARCHITECTURE.md) • [🔧 Config Migration Guide](CONFIG_MIGRATION_GUIDE.md)

---

## 📋 Table of Contents

- [Supported Versions](#supported-versions)
- [Vulnerability Reporting Process](#vulnerability-reporting-process)
- [Security Design Overview](#security-design-overview)
- [Supply Chain and Gates](#supply-chain-and-gates)
- [Security Best Practices](#security-best-practices)
- [Related Documents](#related-documents)

---

## Supported Versions

We recommend that all users always run the latest release to receive the full set of security fixes. For security fixes in each release, see the [CHANGELOG](CHANGELOG.md):

| Version | Support status |
|---------|----------------|
| latest (0.3.0-rc.3 and Unreleased) | ✅ Receives security fixes |
| Earlier versions | ❌ No longer receives fixes |

Dependency security is monitored by `cargo-audit` (run with `--deny warnings` in both CI and the release preflight) and `cargo-deny` (license + security advisories checks, configured in `deny.toml`).

---

## Vulnerability Reporting Process

**Please do not report security vulnerabilities through public issues.**

Use the private GitHub [Security Advisories](https://github.com/Kirky-X/NebulaId/security/policy) disclosure channel, and include where possible: affected versions, reproduction steps or a proof of concept, and an impact assessment. Kirky.X will follow up with assessment and fixes after confirming receipt; fixes are disclosed in the `Security` section of the [CHANGELOG](CHANGELOG.md).

Expectations for reporters and handling priority: issues exploitable remotely without authentication (auth bypass, injection, DoS) are handled first; issues requiring an authenticated account or local access come second.

---

## Security Design Overview

### Authentication (garrison-auth)

- API key verification is taken over by [garrison](https://crates.io/crates/garrison) (`web-axum` middleware + `protocol-apikey` ApiKeyHandler + `cache-memory` KV backend); the hand-written scheme is no longer used.
- Keys are stored as **Argon2id** hashes (`argon2 0.6`, fixing CWE-916; the salt comes from `NEBULA_API_KEY_SALT` or `[auth].api_key_salt`), and credential comparison uses a constant-time implementation (`subtle`).
- **Rotation grace period**: when `key_rotation_grace_period_seconds > 0`, rotation persists the previous hash and the window deadline (`prev_secret_hash` / `rotate_expires_at` columns, backfilled automatically by startup migrations — see the [Config Migration Guide](CONFIG_MIGRATION_GUIDE.md#key-rotation-grace-period-and-config-fail-fast-unreleased)); the old key remains verifiable within the grace window and expires lazily at the deadline. Beyond 30 days (2592000 seconds), startup validation rejects outright (fail-fast, not clamped). Defaults to `0` (disabled).
- **Auth cache**: the in-process `MemoryGarrisonDao` implementation serves cache hits first and falls back to the DB on miss; cached values contain only workspace_id / role / expiry, **never the secret**. Revocation/rotation/disable takes effect immediately in this process; in multi-node deployments other nodes lag by at most one TTL. Cache invalidation uses exact prefix matching — key_ids with glob shapes (containing `*` / `?`) cannot bulk-clear other entries.
- **gRPC authentication**: every RPC (including bidirectional streams) is verified at each entry by the single-point `authenticate()` on the `authorization` header (Basic/ApiKey). A disabled or expired key returns `PermissionDenied`; invalid or unknown credentials return `Unauthenticated`; with `auth.enabled = false` requests pass through.
- **Single admin key guard**: the guard for creating/revoking admin keys uses an SQL-side count of `role='admin' AND enabled=true` (exact row-id lookup, no pagination upper bound), blocking an "admin creates another admin" persistence backdoor. Note the scope: the guard only applies to the HTTP `POST /api-keys` endpoint; admin keys bootstrapped from environment/config at startup are inserted directly through the repository.

### Traffic Control (Rate Limiting)

- Token-bucket rate limiting (`limiteron` primitives) is genuinely mounted on the HTTP stack, **inside** CORS / security headers; 429 responses carry the canonical lowercase `x-ratelimit-limit` / `x-ratelimit-remaining` headers (429 additionally sets `x-ratelimit-remaining: 0`).
- With `[rate_limit].enabled = false` the rate-limit layer is not mounted; `POST /config/rate-limit` hot updates reuse the startup-time `Config::validate` (including the `burst ≤ 10 × default_rps` combined validation).
- Requests are attributed to the real peer connection address (`PeerAddr` carrying `ConnectInfo`), so rate-limit keys, auth failure counters, and the audit `client_ip` no longer collapse into a shared anonymous bucket; `X-Forwarded-For` is only trusted under a trusted-proxy configuration (IP spoofing protection).

### Transport and Response Security

- rustls TLS with `tls.min_tls_version` genuinely enforced: with `tls13`, ClientHello messages below TLS 1.3 are rejected outright (HTTP side via `ServerConfig::builder_with_protocol_versions`); the gRPC-side ServerConfig is assembled by tonic and cannot inject a version set, so startup marks this boundary honestly with a `warn`.
- With `tls.enabled = true` and a missing/unparseable certificate, startup is **refused** (fail-fast, no silent fallback to plaintext); a contradiction between `enabled=false` and per-port switches produces an explicit warn.
- Each TLS handshake runs in its own per-connection task with a 10-second handshake timeout, so unauthenticated clients cannot freeze the listening port by "connecting without sending a ClientHello"; accept errors back off.
- Security response headers: X-Content-Type-Options, X-Frame-Options, CSP, HSTS, X-XSS-Protection, Referrer-Policy; CORS uses a strict allowlist.

### Audit and Tenant Isolation

- Full audit coverage of ID generation and key operations (`src/core/algorithm/audit_trait.rs`, `src/server/audit/`), with `client_ip` attributed to the real peer address; audit reasons and i18n copy share a single source.
- biz-tags queries filter workspaces by role (User sees only its own workspace; Admin may filter by parameter) — this fixes an IDOR (strix-0001, see the [CHANGELOG](CHANGELOG.md)).
- `Locale` derives from the `Accept-Language` user input and is forgeable; it is used for content negotiation only and **must not** participate in any authentication/authorization decision.
- SDK errors return only a fixed summary + `error_id` externally; internal details go to logs only (internal errors are never echoed back).

### Configuration Surface Security

- All 18 config structs carry `#[serde(deny_unknown_fields)]`; a bad config (read failure, parse failure, validation failure) always aborts startup with exit code 1 — no silent fallback to defaults.
- Sensitive values are injected via `${VAR}` environment expansion (`NEBULA_DATABASE_PASSWORD`, `NEBULA_API_KEY_SALT`); for the migration aligning the `algorithm_type` database ENUM with the code, see the [Config Migration Guide](CONFIG_MIGRATION_GUIDE.md#algorithm_type-enum-migration-uuid_v7--uuid_v4---uuid_v8).

---

## Supply Chain and Gates

Gates run twice: in CI (`.github/workflows/`) and in local Git hooks (`lefthook.yml` + `scripts/run.sh`):

| Gate | CI location | Local equivalent | Content |
|------|-------------|------------------|---------|
| Format + Lint | `ci.yml` · fmt-clippy | lefthook pre-commit / `run.sh lint` | `cargo fmt --check`; `cargo clippy --all-features -- -D warnings` (0 warnings) |
| Dependency licenses and advisories | `ci.yml` · deny | — | `cargo-deny check` (`deny.toml`) |
| Dependency vulnerability audit | `ci.yml` · audit; `release.yml` release preflight | — | `cargo audit --deny warnings` |
| Static security analysis | `codeql.yml` | — | CodeQL Rust analysis (the README Security badge) |
| Tests + coverage | `ci.yml` · test (default / all matrix; coverage gate only on the default leg) | lefthook pre-push | `cargo llvm-cov --fail-under-lines 95` (CI default leg) / `≥ 90%` with all features (pre-push local gate, `scripts/_coverage_gate.sh`), excluding `server/proto/` generated code |
| Private-key scan | — | lefthook pre-commit | gitleaks (`scripts/_gitleaks_scan.sh`) |
| Aggregate gate | `ci.yml` · gate | — | Any failing upstream job blocks the merge |

Additional facts:

- `--all-features` is buildable (the sqlite feature was removed; dbnexus's embedded/server exclusivity no longer triggers), and CI and docs uniformly adopt that switch; `--no-default-features` additionally gets a light compile check (garrison fallback paths and similar).
- The Docker build context excludes `.env` / `.env.*` (keeping `.env.example`) plus `target/` and `.git` via `.dockerignore`; all dependencies come from crates.io, so local credentials never enter image layers.
- `Cargo.lock` is committed, making CI and release builds dependency-reproducible.

---

## Security Best Practices

**For deployers:**

1. **Secrets**: `NEBULA_DATABASE_PASSWORD` and `NEBULA_API_KEY_SALT` must be injected via environment variables; never write them in plaintext into config files. Use a high-strength random salt and never leak it after deployment.
2. **TLS**: set `tls.enabled = true` in production and prefer `min_tls_version = "tls13"`; keep certificate private-key file permissions minimal.
3. **Authentication**: keep `auth.enabled = true` and `cache_ttl_seconds > 0`; understand that revocation converges across nodes within one TTL — for emergencies, rolling-restart instances.
4. **admin key**: create admin keys only via the HTTP `POST /api-keys` endpoint (protected by the single-admin guard); avoid relying on the startup bootstrap path that bypasses the guard; rotate periodically and enable the grace period for smooth transitions.
5. **Rate limiting**: keep `rate_limit.enabled = true`, and set `default_rps` / `burst_size` according to business capacity to prevent a single tenant from exhausting the shared quota.
6. **Network**: do not expose PostgreSQL / Redis / etcd to the public internet; keep the `/metrics` port open only to the monitoring network segment.
7. **Upgrades**: run the preflight commands from the [Config Migration Guide](CONFIG_MIGRATION_GUIDE.md) before upgrading to confirm there are no unknown keys or missing required keys; watch the Breaking and Security sections in the CHANGELOG.

**For contributors:**

1. New config fields must be synced into `Config::validate()` and the documentation ([Config Migration Guide · config reference](CONFIG_MIGRATION_GUIDE.md#config-reference-all-options-and-validation-rules)).
2. Code touching credentials must use constant-time comparison and must never log secrets; new sensitive keys must go through the `${VAR}` expansion path.
3. New dependencies must pass the `cargo-deny` / `cargo-audit` gates; prefer small, focused, actively maintained crates.
4. Run `./scripts/run.sh pre-commit` before committing (the gitleaks private-key scan is included).

---

## Related Documents

- [CHANGELOG](CHANGELOG.md): security fixes and behavior changes
- [Config Migration Guide](CONFIG_MIGRATION_GUIDE.md): key rotation grace period and config fail-fast
- [Deployment Guide](DEPLOYMENT.md): production deployment and monitoring
- [API Reference](API_REFERENCE.md): auth headers and error-code semantics
