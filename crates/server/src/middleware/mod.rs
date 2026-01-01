use axum::extract::State;
use axum::middleware::Next;
use axum::{
    body::Body,
    http::{Request, StatusCode},
    response::IntoResponse,
    response::Response,
};
use base64::Engine;
use sha2::Digest;
use std::collections::HashMap;
use std::sync::Arc;
use subtle::ConstantTimeEq;
use tokio::sync::RwLock;

pub mod utils;

#[derive(Clone)]
pub struct ApiKeyAuth {
    valid_keys: Arc<RwLock<HashMap<String, ApiKeyData>>>,
}

#[derive(Clone, Debug)]
pub struct ApiKeyData {
    pub key_hash: String,
    pub workspace_id: String,
    pub key_prefix: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub enabled: bool,
}

impl Default for ApiKeyAuth {
    fn default() -> Self {
        Self::new()
    }
}

impl ApiKeyAuth {
    pub fn new() -> Self {
        Self {
            valid_keys: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn load_key(&self, key_id: String, key_hash: String, workspace_id: String) {
        let key_id_clone = key_id.clone();
        let mut keys = self.valid_keys.write().await;
        keys.insert(
            key_id_clone,
            ApiKeyData {
                key_hash,
                workspace_id,
                key_prefix: key_id[..8].to_string(),
                created_at: chrono::Utc::now(),
                expires_at: None,
                enabled: true,
            },
        );
    }

    pub async fn validate_key(&self, key_id: &str, key_secret: &str) -> Option<String> {
        let keys = self.valid_keys.read().await;
        if let Some(key_data) = keys.get(key_id) {
            if !key_data.enabled {
                return None;
            }
            if let Some(expires_at) = key_data.expires_at {
                if expires_at < chrono::Utc::now() {
                    return None;
                }
            }
            let expected_hash = Self::hash_key(key_id, key_secret);
            // Use constant-time comparison to prevent timing attacks
            if expected_hash
                .as_bytes()
                .ct_eq(key_data.key_hash.as_bytes())
                .into()
            {
                return Some(key_data.workspace_id.clone());
            }
        }
        None
    }

    pub(crate) fn hash_key(key_id: &str, key_secret: &str) -> String {
        let mut hasher = sha2::Sha256::default();
        hasher.update(format!("{}-{}", key_id, key_secret));
        hex::encode(hasher.finalize())
    }

    pub fn compute_key_hash(key_id: &str, key_secret: &str) -> String {
        Self::hash_key(key_id, key_secret)
    }

    pub async fn auth_middleware(&self, mut req: Request<Body>, next: Next) -> Response {
        let auth_header = req.headers().get("authorization").cloned();

        if let Some(header) = auth_header {
            if let Ok(value) = header.to_str() {
                if let Some(credentials) = value.strip_prefix("Basic ") {
                    if let Ok(decoded) =
                        base64::engine::general_purpose::STANDARD.decode(credentials)
                    {
                        if let Ok(cred_str) = String::from_utf8(decoded) {
                            let parts: Vec<&str> = cred_str.splitn(2, ':').collect();
                            if parts.len() == 2 {
                                let (key_id, key_secret) = (parts[0], parts[1]);
                                if let Some(workspace_id) =
                                    self.validate_key(key_id, key_secret).await
                                {
                                    req.extensions_mut().insert(workspace_id);
                                    return next.run(req).await;
                                }
                            }
                        }
                    }
                } else if let Some(api_key) = value.strip_prefix("ApiKey ") {
                    let parts: Vec<&str> = api_key.splitn(2, '_').collect();
                    if parts.len() == 2 {
                        if let Some(workspace_id) = self.validate_key(parts[0], parts[1]).await {
                            req.extensions_mut().insert(workspace_id);
                            return next.run(req).await;
                        }
                    }
                }
            }
        }

        let response = axum::Json(serde_json::json!({
            "code": 401,
            "message": "Invalid or missing API key"
        }))
        .into_response();
        (StatusCode::UNAUTHORIZED, response).into_response()
    }
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

    #[tokio::test]
    async fn test_api_key_auth() {
        let auth = ApiKeyAuth::new();
        let key_id = "test-key-id";
        let key_secret = "test-secret";
        let workspace_id = "test-workspace";
        let key_hash = ApiKeyAuth::hash_key(key_id, key_secret);

        auth.load_key(key_id.to_string(), key_hash, workspace_id.to_string())
            .await;

        let result = auth.validate_key(key_id, key_secret).await;
        assert_eq!(result, Some(workspace_id.to_string()));

        let result = auth.validate_key(key_id, "wrong-secret").await;
        assert_eq!(result, None);
    }
}
