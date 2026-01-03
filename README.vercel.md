# Deploying Nebula ID to Vercel

This guide explains how to deploy Nebula ID to Vercel as a Serverless Function without using Etcd.

## Overview

In this Vercel deployment:
- **Etcd is NOT used**. We use `nebula-api` which is a Vercel-compatible adapter.
- **Serverless**: The application runs as a serverless function.
- **Stateless/Stateful**:
    - `uuid_v7` and `uuid_v4` are stateless and work out of the box.
    - `snowflake` requires `WORKER_ID` to be unique per instance (see below).
    - `segment` requires a Postgres database.

## Prerequisites

1.  **Vercel Account**: [Sign up here](https://vercel.com).
2.  **Vercel CLI**: `npm i -g vercel`
3.  **Postgres Database** (Optional but recommended for `segment` algorithm):
    - You can use [Vercel Postgres](https://vercel.com/docs/storage/vercel-postgres), Neon, or any other provider.
4.  **Redis** (Optional): Used for caching if configured, but not strictly required for basic operation.

## Configuration

Set the following Environment Variables in your Vercel Project settings:

| Variable | Description | Default |
|----------|-------------|---------|
| `DATABASE_URL` | Postgres connection string | `postgres://...` |
| `REDIS_URL` | Redis connection string | `redis://...` |
| `RUST_LOG` | Log level | `info` |
| `DC_ID` | Datacenter ID (0-31) | `0` |
| `WORKER_ID` | Worker ID (0-255). **Must be unique** if using Snowflake. | `0` |

### Important Note on Snowflake Algorithm
If you use the `snowflake` algorithm on Vercel, keep in mind that Vercel may scale your function to multiple instances. If two instances have the same `WORKER_ID` (default 0), they might generate colliding IDs if triggered in the exact same millisecond.
**Recommendation**: For serverless deployments, prefer `uuid_v7` or ensure you understand the concurrency model.

## Deployment Steps

1.  **Install Dependencies**:
    Ensure you have Rust installed.

2.  **Deploy with Vercel CLI**:
    Run the following command in the project root:

    ```bash
    vercel
    ```

    Follow the prompts to link your project.

3.  **Test the API**:
    Once deployed, you can access the API at the provided URL.

    ```bash
    # Generate ID using default algorithm
    curl https://your-project.vercel.app/api/v1/generate \
      -H "Content-Type: application/json" \
      -d '{"workspace": "default", "group": "orders", "biz_tag": "order_id"}'
    ```

## Local Development

You can run the API locally using `cargo run` (for the standard server) or `vercel dev` (if you want to simulate Vercel).

To run the standard server without etcd locally:

```bash
# Disable default features (etcd)
cargo run --no-default-features
```
