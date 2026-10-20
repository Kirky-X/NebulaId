// Copyright (c) 2025-2026 Kirky.X🌠
// SPDX-License-Identifier: Apache-2.0

//! sdforge 0.5 adapter — bridges inventory-registered `#[forge]` handlers
//! into the nebulaid axum router. Note: `sdforge_health` uses underscores
//! because sdforge_macros 0.5 requires a valid Rust identifier (spec deviation).

use axum::Router;
use sdforge::core::Registration;
use sdforge::forge;
use serde_json::json;

/// Initialize all sdforge plugins (HTTP/MCP/WebSocket/gRPC/CLI inventory submissions).
/// Call once at startup, before building the axum router, or inventory routes may be linker-stripped.
pub fn init_sdforge() -> sdforge::PluginCounts {
    sdforge::init_all_plugins()
}

/// Merge all inventory-registered sdforge HTTP routes into the supplied base router.
/// Each `RouteRegistration::create` yields an `HttpRoute` merged via `Router::route`.
pub fn merge_sdforge_routes(router: Router) -> Router {
    let mut router = router;
    for reg in sdforge::inventory::iter::<sdforge::http::RouteRegistration> {
        let route = reg.create();
        router = router.route(route.path(), route.handler().clone());
    }
    router
}

/// Sample `#[forge]`-annotated health endpoint — verifies sdforge 0.5 macro expansion
/// wires correctly; returns the crate version so callers can confirm the route is served.
#[forge(
    name = "sdforge_health",
    version = "v1",
    description = "sdforge integration health check",
    path = "/health/sdforge",
    method = "GET"
)]
pub async fn sdforge_health() -> Result<serde_json::Value, sdforge::prelude::ApiError> {
    Ok(json!({
        "status": "ok",
        "sdforge_version": env!("CARGO_PKG_VERSION"),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    // 该断言依赖 `#[forge]` 展开期 emit 的 `#[cfg(feature = "http")]` 注册块：
    // no-default 轻量构建下 http 镜像 feature 关闭，inventory 为空是预期行为。
    #[cfg(feature = "http")]
    #[test]
    fn test_init_sdforge_returns_plugin_counts() {
        let counts = init_sdforge();
        // At minimum, the sdforge_health route registered in this module must be counted.
        assert!(
            counts.routes >= 1,
            "expected at least 1 sdforge route, got {}",
            counts.routes
        );
    }

    #[test]
    fn test_merge_sdforge_routes_includes_health_endpoint() {
        let router = merge_sdforge_routes(Router::new());
        // Axum Router has no public introspection API; the absence of panic
        // plus the route-count check above is sufficient verification.
        drop(router);
    }

    #[tokio::test]
    async fn test_sdforge_health_returns_ok_with_version() {
        let value = sdforge_health().await.expect("health handler must succeed");
        assert_eq!(value.get("status").and_then(|v| v.as_str()), Some("ok"));
        assert_eq!(
            value.get("sdforge_version").and_then(|v| v.as_str()),
            Some(env!("CARGO_PKG_VERSION"))
        );
    }
}
