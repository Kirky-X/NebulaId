// Copyright © 2026 Kirky.X
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::middleware::Next;
use axum::response::IntoResponse;
use axum::response::Response;
use base64::Engine;
use nebula_core::database::ApiKeyRepository;
use std::sync::Arc;

pub(crate) mod utils;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApiKeyRole {
    Admin,
    User,
}

impl From<&str> for ApiKeyRole {
    fn from(s: &str) -> Self {
        match s {
            "admin" => ApiKeyRole::Admin,
            _ => ApiKeyRole::User,
        }
    }
}

impl From<ApiKeyRole> for &str {
    fn from(role: ApiKeyRole) -> Self {
        match role {
            ApiKeyRole::Admin => "admin",
            ApiKeyRole::User => "user",
        }
    }
}

impl From<ApiKeyRole> for nebula_core::database::ApiKeyRole {
    fn from(role: ApiKeyRole) -> Self {
        match role {
            ApiKeyRole::Admin => nebula_core::database::ApiKeyRole::Admin,
            ApiKeyRole::User => nebula_core::database::ApiKeyRole::User,
        }
    }
}

#[derive(Clone)]
pub struct ApiKeyAuth {
    pub(crate) repo: Arc<dyn ApiKeyRepository>,
}

impl ApiKeyAuth {
    pub fn new(repo: Arc<dyn ApiKeyRepository>) -> Self {
        Self { repo }
    }

    pub async fn validate_key(
        &self,
        key_id: &str,
        key_secret: &str,
    ) -> Option<(Option<uuid::Uuid>, ApiKeyRole)> {
        match self.repo.validate_api_key(key_id, key_secret).await {
            Ok(Some((workspace_id, role))) => {
                let role: ApiKeyRole = match role {
                    nebula_core::database::ApiKeyRole::Admin => ApiKeyRole::Admin,
                    nebula_core::database::ApiKeyRole::User => ApiKeyRole::User,
                };
                Some((workspace_id, role))
            }
            _ => None,
        }
    }

    pub async fn auth_middleware(&self, mut req: Request<Body>, next: Next) -> Response {
        let path = req.uri().path().to_string();
        tracing::debug!(event = "auth_middleware", path = %path, "Auth middleware called");

        let auth_header = req.headers().get("authorization").cloned();

        if let Some(header) = auth_header {
            if let Ok(value) = header.to_str() {
                // Support both "Basic base64(key_id:key_secret)" and "ApiKey key_id:key_secret"
                let (key_id, key_secret) = if let Some(credentials) = value.strip_prefix("Basic ") {
                    if let Ok(decoded) =
                        base64::engine::general_purpose::STANDARD.decode(credentials)
                    {
                        if let Ok(cred_str) = String::from_utf8(decoded) {
                            let parts: Vec<&str> = cred_str.splitn(2, ':').collect();
                            if parts.len() == 2 {
                                (parts[0].to_string(), parts[1].to_string())
                            } else {
                                tracing::warn!(
                                    event = "auth_failure",
                                    reason = "invalid_basic_format",
                                    "Invalid Basic auth format: no colon separator"
                                );
                                return self.unauthorized_response();
                            }
                        } else {
                            tracing::warn!(
                                event = "auth_failure",
                                reason = "invalid_encoding",
                                "Invalid Base64 encoding in auth header"
                            );
                            return self.unauthorized_response();
                        }
                    } else {
                        tracing::warn!(
                            event = "auth_failure",
                            reason = "base64_decode_failed",
                            "Failed to decode Base64 auth header"
                        );
                        return self.unauthorized_response();
                    }
                } else if let Some(api_key) = value.strip_prefix("ApiKey ") {
                    let parts: Vec<&str> = api_key.splitn(2, ':').collect();
                    if parts.len() == 2 {
                        (parts[0].to_string(), parts[1].to_string())
                    } else {
                        tracing::warn!(
                            event = "auth_failure",
                            reason = "invalid_apikey_format",
                            "Invalid ApiKey format: no colon separator"
                        );
                        return self.unauthorized_response();
                    }
                } else {
                    tracing::warn!(
                        event = "auth_failure",
                        reason = "unsupported_format",
                        "Unsupported auth format"
                    );
                    return self.unauthorized_response();
                };

                // Validate input lengths to prevent empty credentials
                if key_id.is_empty() || key_secret.is_empty() {
                    tracing::warn!(
                        event = "auth_failure",
                        reason = "empty_credentials",
                        "Empty key_id or key_secret"
                    );
                    return self.unauthorized_response();
                }

                if let Some((workspace_id, role)) = self.validate_key(&key_id, &key_secret).await {
                    req.extensions_mut().insert(workspace_id);
                    req.extensions_mut().insert(role);
                    return next.run(req).await;
                } else {
                    // Log auth failure with key_id prefix (masked for security)
                    let key_id_prefix = key_id.chars().take(8).collect::<String>();
                    tracing::warn!(event = "auth_failure", reason = "invalid_credentials", key_id_prefix = %key_id_prefix, "Invalid API key credentials");
                }
            }
        }

        // Return 401 for both unknown routes and missing auth to avoid information disclosure
        // This prevents attackers from discovering which API endpoints exist
        self.unauthorized_response()
    }

    fn unauthorized_response(&self) -> Response {
        let response = axum::Json(serde_json::json!({
            "code": 401,
            "message": "Invalid or missing API key"
        }))
        .into_response();
        (StatusCode::UNAUTHORIZED, response).into_response()
    }
}

pub async fn admin_required_middleware(req: Request<Body>, next: Next) -> Response {
    if let Some(role) = req.extensions().get::<ApiKeyRole>() {
        if *role == ApiKeyRole::Admin {
            return next.run(req).await;
        }
    }

    let response = axum::Json(serde_json::json!({
        "code": 403,
        "message": "Admin access required"
    }))
    .into_response();
    (StatusCode::FORBIDDEN, response).into_response()
}

pub async fn auth_middleware_fn(
    State(auth): State<Arc<ApiKeyAuth>>,
    req: Request<Body>,
    next: Next,
) -> Response {
    auth.auth_middleware(req, next).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use nebula_core::database::{
        ApiKeyInfo, ApiKeyRepository, ApiKeyResponse, ApiKeyRole as CoreApiKeyRole,
        ApiKeyWithSecret, CreateApiKeyRequest,
    };
    use nebula_core::types::Result;
    use sha2::Digest;
    use uuid::Uuid;

    #[derive(Clone)]
    struct MockApiKeyRepo {
        keys: std::collections::HashMap<String, (String, ApiKeyRole)>,
    }

    impl MockApiKeyRepo {
        fn hash_secret(secret: &str) -> String {
            let mut hasher = sha2::Sha256::default();
            hasher.update(secret);
            hex::encode(hasher.finalize())
        }
    }

    #[async_trait]
    impl ApiKeyRepository for MockApiKeyRepo {
        async fn create_api_key(&self, _request: &CreateApiKeyRequest) -> Result<ApiKeyWithSecret> {
            Ok(ApiKeyWithSecret {
                key: ApiKeyResponse {
                    id: Uuid::new_v4(),
                    key_id: "mock_key_id".to_string(),
                    key_prefix: "nino_".to_string(),
                    name: "Mock Key".to_string(),
                    description: None,
                    role: CoreApiKeyRole::User,
                    rate_limit: 10000,
                    enabled: true,
                    expires_at: None,
                    created_at: chrono::Utc::now().naive_utc(),
                },
                key_secret: "mock_secret".to_string(),
            })
        }

        async fn get_api_key_by_id(&self, _key_id: &str) -> Result<Option<ApiKeyInfo>> {
            Ok(None)
        }

        async fn validate_api_key(
            &self,
            key_id: &str,
            key_secret: &str,
        ) -> Result<Option<(Option<uuid::Uuid>, nebula_core::database::ApiKeyRole)>> {
            use subtle::ConstantTimeEq;
            if let Some((expected_secret, role)) = self.keys.get(key_id) {
                let incoming_hash = MockApiKeyRepo::hash_secret(key_secret);
                if expected_secret
                    .as_bytes()
                    .ct_eq(incoming_hash.as_bytes())
                    .into()
                {
                    // Admin keys have None workspace_id, user keys have Some(workspace_id)
                    let workspace_id = if *role == ApiKeyRole::Admin {
                        None
                    } else {
                        Some(uuid::Uuid::nil())
                    };
                    return Ok(Some((workspace_id, role.clone().into())));
                }
            }
            Ok(None)
        }

        async fn list_api_keys(
            &self,
            _workspace_id: Uuid,
            _limit: Option<u32>,
            _offset: Option<u32>,
        ) -> Result<Vec<ApiKeyInfo>> {
            Ok(vec![])
        }

        async fn delete_api_key(&self, _id: Uuid) -> Result<()> {
            Ok(())
        }

        async fn revoke_api_key(&self, _id: Uuid) -> Result<()> {
            Ok(())
        }

        async fn update_last_used(&self, _key: Uuid) -> Result<()> {
            Ok(())
        }

        async fn get_admin_api_key(&self, _workspace_id: Uuid) -> Result<Option<ApiKeyInfo>> {
            Ok(None)
        }

        async fn count_api_keys(&self, _workspace_id: Uuid) -> Result<u64> {
            Ok(0)
        }
    }

    #[tokio::test]
    async fn test_api_key_auth_with_mock_repo() {
        let mut mock_keys = std::collections::HashMap::new();
        // Use hash_secret which only hashes the secret, matching the real validation logic
        mock_keys.insert(
            "test-key-id".to_string(),
            (MockApiKeyRepo::hash_secret("test-secret"), ApiKeyRole::User),
        );
        mock_keys.insert(
            "admin-key".to_string(),
            (
                MockApiKeyRepo::hash_secret("admin-secret"),
                ApiKeyRole::Admin,
            ),
        );

        let repo = MockApiKeyRepo { keys: mock_keys };
        let auth = ApiKeyAuth::new(Arc::new(repo));

        // Test valid user key
        let result = auth.validate_key("test-key-id", "test-secret").await;
        assert!(result.is_some());
        assert_eq!(result.unwrap().1, ApiKeyRole::User);

        // Test valid admin key
        let result = auth.validate_key("admin-key", "admin-secret").await;
        assert!(result.is_some());
        assert_eq!(result.unwrap().1, ApiKeyRole::Admin);

        // Test invalid secret
        let result = auth.validate_key("test-key-id", "wrong-secret").await;
        assert!(result.is_none());

        // Test non-existent key
        let result = auth.validate_key("non-existent", "secret").await;
        assert!(result.is_none());
    }
}
