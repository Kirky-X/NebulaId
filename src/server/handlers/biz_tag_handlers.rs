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

//! BizTag CRUD handlers (rule 25 split).

use crate::core::{CoreError, Result};
use crate::server::models::{
    naive_to_rfc3339, BizTagListResponse, BizTagResponse, CreateBizTagRequest, UpdateBizTagRequest,
};

impl super::ApiHandlers {
    pub async fn create_biz_tag(&self, req: CreateBizTagRequest) -> Result<BizTagResponse> {
        let algorithm = req
            .algorithm
            .clone()
            .unwrap_or_else(|| "segment".to_string());
        let format = req.format.clone().unwrap_or_else(|| "numeric".to_string());

        let core_req = crate::core::database::CreateBizTagRequest {
            workspace_id: req.workspace_id,
            group_id: req.group_id,
            name: req.name,
            description: req.description,
            algorithm: Some(
                algorithm
                    .parse()
                    .map_err(|_| CoreError::InvalidAlgorithmType(algorithm.clone()))?,
            ),
            format: Some(
                format
                    .parse()
                    .map_err(|_| CoreError::InvalidIdFormat(format.clone()))?,
            ),
            prefix: req.prefix,
            base_step: req.base_step,
            max_step: req.max_step,
            datacenter_ids: req.datacenter_ids,
        };

        let biz_tag = self.config_service.create_biz_tag(&core_req).await?;

        Ok(BizTagResponse {
            id: biz_tag.id.to_string(),
            workspace_id: biz_tag.workspace_id.to_string(),
            group_id: biz_tag.group_id.to_string(),
            name: biz_tag.name,
            description: biz_tag.description,
            algorithm: biz_tag.algorithm.to_string(),
            format: biz_tag.format.to_string(),
            prefix: biz_tag.prefix,
            base_step: biz_tag.base_step,
            max_step: biz_tag.max_step,
            datacenter_ids: biz_tag.datacenter_ids,
            created_at: naive_to_rfc3339(biz_tag.created_at),
            updated_at: naive_to_rfc3339(biz_tag.updated_at),
        })
    }

    pub async fn update_biz_tag(
        &self,
        id: uuid::Uuid,
        req: UpdateBizTagRequest,
    ) -> Result<BizTagResponse> {
        let core_req = crate::core::database::UpdateBizTagRequest {
            name: req.name,
            description: req.description,
            algorithm: req
                .algorithm
                .map(|a: String| a.parse().map_err(|_| CoreError::InvalidAlgorithmType(a)))
                .transpose()?,
            format: req
                .format
                .map(|f: String| f.parse().map_err(|_| CoreError::InvalidIdFormat(f)))
                .transpose()?,
            prefix: req.prefix,
            base_step: req.base_step,
            max_step: req.max_step,
            datacenter_ids: req.datacenter_ids,
        };

        let biz_tag = self.config_service.update_biz_tag(id, &core_req).await?;

        Ok(BizTagResponse {
            id: biz_tag.id.to_string(),
            workspace_id: biz_tag.workspace_id.to_string(),
            group_id: biz_tag.group_id.to_string(),
            name: biz_tag.name,
            description: biz_tag.description,
            algorithm: biz_tag.algorithm.to_string(),
            format: biz_tag.format.to_string(),
            prefix: biz_tag.prefix,
            base_step: biz_tag.base_step,
            max_step: biz_tag.max_step,
            datacenter_ids: biz_tag.datacenter_ids,
            created_at: naive_to_rfc3339(biz_tag.created_at),
            updated_at: naive_to_rfc3339(biz_tag.updated_at),
        })
    }

    pub async fn get_biz_tag(&self, id: uuid::Uuid) -> Result<BizTagResponse> {
        let biz_tag: crate::core::database::BizTag =
            self.config_service.get_biz_tag(id).await?.ok_or_else(|| {
                CoreError::NotFound(
                    t!("api.error.handlers.biz_tag_handlers.not_found", id = id).to_string(),
                )
            })?;

        Ok(BizTagResponse {
            id: biz_tag.id.to_string(),
            workspace_id: biz_tag.workspace_id.to_string(),
            group_id: biz_tag.group_id.to_string(),
            name: biz_tag.name,
            description: biz_tag.description,
            algorithm: biz_tag.algorithm.to_string(),
            format: biz_tag.format.to_string(),
            prefix: biz_tag.prefix,
            base_step: biz_tag.base_step,
            max_step: biz_tag.max_step,
            datacenter_ids: biz_tag.datacenter_ids,
            created_at: naive_to_rfc3339(biz_tag.created_at),
            updated_at: naive_to_rfc3339(biz_tag.updated_at),
        })
    }

    pub async fn list_biz_tags(
        &self,
        workspace_id: Option<uuid::Uuid>,
        group_id: Option<uuid::Uuid>,
    ) -> Result<BizTagListResponse> {
        // L7 修复：缺失 workspace_id 时返回 InvalidInput，避免静默回退到
        // nil UUID 触发 `WHERE workspace_id = '00000000-...'` 查询，
        // 该查询在底层 SeaORM 中可能意外匹配到 workspace_id 字段为 nil
        // 的脏数据记录，导致越权返回其他 workspace 的 BizTag。
        let workspace_id = workspace_id.ok_or_else(|| {
            CoreError::InvalidInput(
                t!("api.error.handlers.biz_tag_handlers.workspace_id_required_list").to_string(),
            )
        })?;

        let biz_tags: Vec<crate::core::database::BizTag> = self
            .config_service
            .list_biz_tags(workspace_id, group_id, None, None)
            .await?;

        // L8 修复：调用 count_biz_tags 获取真实总数，而非用当前页列表长度
        // 作为 total。原实现 `total = responses.len() as u64` 在分页场景下
        // 会让前端分页控件显示错误的总数（仅当前页条数）。
        let total = self
            .config_service
            .count_biz_tags(workspace_id, group_id)
            .await?;

        let responses: Vec<BizTagResponse> = biz_tags
            .into_iter()
            .map(|bt| BizTagResponse {
                id: bt.id.to_string(),
                workspace_id: bt.workspace_id.to_string(),
                group_id: bt.group_id.to_string(),
                name: bt.name,
                description: bt.description,
                algorithm: bt.algorithm.to_string(),
                format: bt.format.to_string(),
                prefix: bt.prefix,
                base_step: bt.base_step,
                max_step: bt.max_step,
                datacenter_ids: bt.datacenter_ids,
                created_at: naive_to_rfc3339(bt.created_at),
                updated_at: naive_to_rfc3339(bt.updated_at),
            })
            .collect();

        Ok(BizTagListResponse {
            total,
            biz_tags: responses,
            page: 1,
            page_size: total,
        })
    }

    pub async fn list_biz_tags_with_pagination(
        &self,
        workspace_id: Option<uuid::Uuid>,
        group_id: Option<uuid::Uuid>,
        limit: usize,
        offset: usize,
    ) -> Result<BizTagListResponse> {
        if limit == 0 {
            return Err(CoreError::InvalidInput(
                t!("api.error.handlers.biz_tag_handlers.pagination_limit_zero").to_string(),
            ));
        }

        let workspace_id = workspace_id.unwrap_or_else(uuid::Uuid::nil);

        let biz_tags: Vec<crate::core::database::BizTag> = self
            .config_service
            .list_biz_tags(
                workspace_id,
                group_id,
                Some(limit as u32),
                Some(offset as u32),
            )
            .await?;

        let total = self
            .config_service
            .count_biz_tags(workspace_id, group_id)
            .await?;

        let responses: Vec<BizTagResponse> = biz_tags
            .into_iter()
            .map(|bt| BizTagResponse {
                id: bt.id.to_string(),
                workspace_id: bt.workspace_id.to_string(),
                group_id: bt.group_id.to_string(),
                name: bt.name,
                description: bt.description,
                algorithm: bt.algorithm.to_string(),
                format: bt.format.to_string(),
                prefix: bt.prefix,
                base_step: bt.base_step,
                max_step: bt.max_step,
                datacenter_ids: bt.datacenter_ids,
                created_at: naive_to_rfc3339(bt.created_at),
                updated_at: naive_to_rfc3339(bt.updated_at),
            })
            .collect();

        Ok(BizTagListResponse {
            total,
            biz_tags: responses,
            page: (offset / limit + 1) as u64,
            page_size: limit as u64,
        })
    }

    pub async fn delete_biz_tag(&self, id: uuid::Uuid) -> Result<()> {
        self.config_service.delete_biz_tag(id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::database::{BizTag, IdFormat};
    use crate::core::types::AlgorithmType;
    use crate::server::config::management::ConfigManagementService;
    use crate::server::handlers::mock_generator::MockIdGenerator;
    use crate::server::handlers::ApiHandlers;
    use crate::server::models::*;
    use async_trait::async_trait;
    use mockall::mock;
    use std::sync::Arc;
    use uuid::Uuid;

    mock! {
        pub BizTagTestService {}
        #[async_trait]
        impl ConfigManagementService for BizTagTestService {
            fn get_config(&self) -> ConfigResponse;
            fn get_secure_config(&self) -> SecureConfigResponse;
            fn get_batch_max_size(&self) -> u32;
            async fn update_rate_limit(&self, req: UpdateRateLimitRequest) -> UpdateConfigResponse;
            async fn update_logging(&self, req: UpdateLoggingRequest) -> UpdateConfigResponse;
            async fn reload_config(&self) -> UpdateConfigResponse;
            async fn get_rate_limit_override(&self) -> Option<(u32, u32)>;
            async fn set_algorithm(&self, req: SetAlgorithmRequest) -> SetAlgorithmResponse;
            async fn create_biz_tag(&self, request: &crate::core::database::CreateBizTagRequest) -> crate::core::Result<BizTag>;
            async fn get_biz_tag(&self, id: Uuid) -> crate::core::Result<Option<BizTag>>;
            async fn update_biz_tag(&self, id: Uuid, request: &crate::core::database::UpdateBizTagRequest) -> crate::core::Result<BizTag>;
            async fn delete_biz_tag(&self, id: Uuid) -> crate::core::Result<()>;
            async fn count_biz_tags(&self, workspace_id: Uuid, group_id: Option<Uuid>) -> crate::core::Result<u64>;
            async fn list_biz_tags(&self, workspace_id: Uuid, group_id: Option<Uuid>, limit: Option<u32>, offset: Option<u32>) -> crate::core::Result<Vec<BizTag>>;
            async fn create_workspace(&self, req: CreateWorkspaceRequest) -> crate::core::Result<WorkspaceResponse>;
            async fn list_workspaces(&self) -> crate::core::Result<WorkspaceListResponse>;
            async fn get_workspace(&self, name: &str) -> crate::core::Result<Option<WorkspaceResponse>>;
            async fn create_group(&self, req: CreateGroupRequest) -> crate::core::Result<GroupResponse>;
            async fn list_groups(&self, workspace: &str) -> crate::core::Result<GroupListResponse>;
            async fn get_database_metrics(&self) -> DatabaseMetrics;
            async fn get_cache_metrics(&self) -> CacheMetrics;
            async fn get_algorithm_metrics(&self) -> Vec<(AlgorithmType, crate::core::algorithm::AlgorithmMetricsSnapshot)>;
        }
    }

    fn make_handlers(mock: MockBizTagTestService) -> Arc<ApiHandlers> {
        let mock_gen = Arc::new(MockIdGenerator::new());
        let config_service: Arc<dyn ConfigManagementService> = Arc::new(mock);
        Arc::new(ApiHandlers::new(mock_gen, config_service))
    }

    fn make_biz_tag() -> BizTag {
        BizTag {
            id: Uuid::new_v4(),
            workspace_id: Uuid::new_v4(),
            group_id: Uuid::new_v4(),
            name: "test-biz-tag".to_string(),
            description: Some("Test biz tag".to_string()),
            algorithm: AlgorithmType::Segment,
            format: IdFormat::Numeric,
            prefix: "test_".to_string(),
            base_step: 100,
            max_step: 1000,
            datacenter_ids: vec![0],
            created_at: chrono::Utc::now().naive_utc(),
            updated_at: chrono::Utc::now().naive_utc(),
        }
    }

    fn make_create_req() -> CreateBizTagRequest {
        CreateBizTagRequest {
            workspace_id: Uuid::new_v4(),
            group_id: Uuid::new_v4(),
            name: "test-biz-tag".to_string(),
            description: Some("Test biz tag".to_string()),
            algorithm: Some("segment".to_string()),
            format: Some("numeric".to_string()),
            prefix: Some("test_".to_string()),
            base_step: Some(100),
            max_step: Some(1000),
            datacenter_ids: Some(vec![0]),
        }
    }

    // ===== create_biz_tag =====

    #[tokio::test]
    async fn test_create_biz_tag_happy_path() {
        let biz_tag = make_biz_tag();
        let expected_id = biz_tag.id.to_string();
        let biz_tag_clone = biz_tag.clone();
        let mut mock = MockBizTagTestService::new();
        mock.expect_create_biz_tag()
            .return_once(move |_| Ok(biz_tag_clone));
        let handlers = make_handlers(mock);

        let result = handlers.create_biz_tag(make_create_req()).await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.id, expected_id);
        assert_eq!(response.name, "test-biz-tag");
        assert_eq!(response.algorithm, "segment");
        assert_eq!(response.format, "numeric");
        assert_eq!(response.prefix, "test_");
        assert_eq!(response.base_step, 100);
        assert_eq!(response.max_step, 1000);
        assert_eq!(response.datacenter_ids, vec![0]);
    }

    #[tokio::test]
    async fn test_create_biz_tag_invalid_algorithm() {
        let handlers = make_handlers(MockBizTagTestService::new());
        let mut req = make_create_req();
        req.algorithm = Some("invalid-algo".to_string());

        let result = handlers.create_biz_tag(req).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CoreError::InvalidAlgorithmType(s) => assert_eq!(s, "invalid-algo"),
            other => panic!("Expected InvalidAlgorithmType, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_create_biz_tag_invalid_format() {
        let handlers = make_handlers(MockBizTagTestService::new());
        let mut req = make_create_req();
        req.format = Some("invalid-format".to_string());

        let result = handlers.create_biz_tag(req).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CoreError::InvalidIdFormat(s) => assert_eq!(s, "invalid-format"),
            other => panic!("Expected InvalidIdFormat, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_create_biz_tag_service_error() {
        let mut mock = MockBizTagTestService::new();
        mock.expect_create_biz_tag()
            .return_once(|_| Err(CoreError::InternalError("Database error".to_string())));
        let handlers = make_handlers(mock);

        let result = handlers.create_biz_tag(make_create_req()).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CoreError::InternalError(msg) => assert!(msg.contains("Database error")),
            other => panic!("Expected InternalError, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_create_biz_tag_defaults_when_algo_format_none() {
        // algorithm=None defaults to "segment", format=None defaults to "numeric".
        let biz_tag = make_biz_tag();
        let biz_tag_clone = biz_tag.clone();
        let mut mock = MockBizTagTestService::new();
        mock.expect_create_biz_tag()
            .return_once(move |_| Ok(biz_tag_clone));
        let handlers = make_handlers(mock);

        let mut req = make_create_req();
        req.algorithm = None;
        req.format = None;

        let result = handlers.create_biz_tag(req).await;
        assert!(result.is_ok());
    }

    // ===== update_biz_tag =====

    #[tokio::test]
    async fn test_update_biz_tag_happy_path() {
        let biz_tag = make_biz_tag();
        let expected_id = biz_tag.id.to_string();
        let biz_tag_clone = biz_tag.clone();
        let mut mock = MockBizTagTestService::new();
        mock.expect_update_biz_tag()
            .return_once(move |_, _| Ok(biz_tag_clone));
        let handlers = make_handlers(mock);

        let req = UpdateBizTagRequest {
            name: Some("updated".to_string()),
            description: None,
            algorithm: Some("segment".to_string()),
            format: Some("numeric".to_string()),
            prefix: None,
            base_step: None,
            max_step: None,
            datacenter_ids: None,
        };

        let result = handlers.update_biz_tag(Uuid::new_v4(), req).await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.id, expected_id);
    }

    #[tokio::test]
    async fn test_update_biz_tag_invalid_algorithm() {
        let handlers = make_handlers(MockBizTagTestService::new());
        let req = UpdateBizTagRequest {
            name: None,
            description: None,
            algorithm: Some("bad-algo".to_string()),
            format: None,
            prefix: None,
            base_step: None,
            max_step: None,
            datacenter_ids: None,
        };

        let result = handlers.update_biz_tag(Uuid::new_v4(), req).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            CoreError::InvalidAlgorithmType(_)
        ));
    }

    #[tokio::test]
    async fn test_update_biz_tag_invalid_format() {
        let handlers = make_handlers(MockBizTagTestService::new());
        let req = UpdateBizTagRequest {
            name: None,
            description: None,
            algorithm: None,
            format: Some("bad-format".to_string()),
            prefix: None,
            base_step: None,
            max_step: None,
            datacenter_ids: None,
        };

        let result = handlers.update_biz_tag(Uuid::new_v4(), req).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), CoreError::InvalidIdFormat(_)));
    }

    #[tokio::test]
    async fn test_update_biz_tag_service_error() {
        let mut mock = MockBizTagTestService::new();
        mock.expect_update_biz_tag()
            .return_once(|_, _| Err(CoreError::NotFound("not found".to_string())));
        let handlers = make_handlers(mock);

        let req = UpdateBizTagRequest {
            name: None,
            description: None,
            algorithm: None,
            format: None,
            prefix: None,
            base_step: None,
            max_step: None,
            datacenter_ids: None,
        };

        let result = handlers.update_biz_tag(Uuid::new_v4(), req).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), CoreError::NotFound(_)));
    }

    #[tokio::test]
    async fn test_update_biz_tag_no_fields_still_calls_service() {
        // 全 None 的 update 仍应调用 service。
        let biz_tag = make_biz_tag();
        let biz_tag_clone = biz_tag.clone();
        let mut mock = MockBizTagTestService::new();
        mock.expect_update_biz_tag()
            .return_once(move |_, _| Ok(biz_tag_clone));
        let handlers = make_handlers(mock);

        let req = UpdateBizTagRequest {
            name: None,
            description: None,
            algorithm: None,
            format: None,
            prefix: None,
            base_step: None,
            max_step: None,
            datacenter_ids: None,
        };

        let result = handlers.update_biz_tag(Uuid::new_v4(), req).await;
        assert!(result.is_ok());
    }

    // ===== get_biz_tag =====

    #[tokio::test]
    async fn test_get_biz_tag_happy_path() {
        let biz_tag = make_biz_tag();
        let expected_id = biz_tag.id.to_string();
        let biz_tag_clone = biz_tag.clone();
        let mut mock = MockBizTagTestService::new();
        mock.expect_get_biz_tag()
            .return_once(move |_| Ok(Some(biz_tag_clone)));
        let handlers = make_handlers(mock);

        let result = handlers.get_biz_tag(Uuid::new_v4()).await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.id, expected_id);
        assert_eq!(response.name, "test-biz-tag");
    }

    #[tokio::test]
    async fn test_get_biz_tag_not_found() {
        let mut mock = MockBizTagTestService::new();
        mock.expect_get_biz_tag().return_once(|_| Ok(None));
        let handlers = make_handlers(mock);

        let result = handlers.get_biz_tag(Uuid::new_v4()).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CoreError::NotFound(msg) => assert!(msg.contains("BizTag not found")),
            other => panic!("Expected NotFound, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_get_biz_tag_service_error() {
        let mut mock = MockBizTagTestService::new();
        mock.expect_get_biz_tag()
            .return_once(|_| Err(CoreError::InternalError("db error".to_string())));
        let handlers = make_handlers(mock);

        let result = handlers.get_biz_tag(Uuid::new_v4()).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), CoreError::InternalError(_)));
    }

    // ===== list_biz_tags =====

    #[tokio::test]
    async fn test_list_biz_tags_happy_path() {
        let biz_tag = make_biz_tag();
        let biz_tag_clone = biz_tag.clone();
        let mut mock = MockBizTagTestService::new();
        mock.expect_list_biz_tags()
            .return_once(move |_, _, _, _| Ok(vec![biz_tag_clone]));
        mock.expect_count_biz_tags().return_once(|_, _| Ok(1));
        let handlers = make_handlers(mock);

        let result = handlers.list_biz_tags(Some(Uuid::new_v4()), None).await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.total, 1);
        assert_eq!(response.biz_tags.len(), 1);
        assert_eq!(response.biz_tags[0].name, "test-biz-tag");
        assert_eq!(response.page, 1);
    }

    #[tokio::test]
    async fn test_list_biz_tags_missing_workspace_id() {
        let handlers = make_handlers(MockBizTagTestService::new());

        let result = handlers.list_biz_tags(None, None).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CoreError::InvalidInput(msg) => {
                assert!(msg.contains("workspace_id is required"))
            }
            other => panic!("Expected InvalidInput, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_list_biz_tags_empty_list() {
        let mut mock = MockBizTagTestService::new();
        mock.expect_list_biz_tags()
            .return_once(|_, _, _, _| Ok(Vec::new()));
        mock.expect_count_biz_tags().return_once(|_, _| Ok(0));
        let handlers = make_handlers(mock);

        let result = handlers.list_biz_tags(Some(Uuid::new_v4()), None).await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.total, 0);
        assert!(response.biz_tags.is_empty());
    }

    #[tokio::test]
    async fn test_list_biz_tags_service_error() {
        let mut mock = MockBizTagTestService::new();
        mock.expect_list_biz_tags()
            .return_once(|_, _, _, _| Err(CoreError::InternalError("db error".to_string())));
        let handlers = make_handlers(mock);

        let result = handlers.list_biz_tags(Some(Uuid::new_v4()), None).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), CoreError::InternalError(_)));
    }

    #[tokio::test]
    async fn test_list_biz_tags_count_service_error() {
        // list 成功但 count 失败 — 应当返回错误。
        let mut mock = MockBizTagTestService::new();
        mock.expect_list_biz_tags()
            .return_once(|_, _, _, _| Ok(Vec::new()));
        mock.expect_count_biz_tags()
            .return_once(|_, _| Err(CoreError::InternalError("count err".to_string())));
        let handlers = make_handlers(mock);

        let result = handlers.list_biz_tags(Some(Uuid::new_v4()), None).await;
        assert!(result.is_err());
    }

    // ===== list_biz_tags_with_pagination =====

    #[tokio::test]
    async fn test_list_biz_tags_with_pagination_happy_path() {
        let biz_tag = make_biz_tag();
        let biz_tag_clone = biz_tag.clone();
        let mut mock = MockBizTagTestService::new();
        mock.expect_list_biz_tags()
            .return_once(move |_, _, _, _| Ok(vec![biz_tag_clone]));
        mock.expect_count_biz_tags().return_once(|_, _| Ok(10));
        let handlers = make_handlers(mock);

        let result = handlers
            .list_biz_tags_with_pagination(Some(Uuid::new_v4()), None, 5, 0)
            .await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.total, 10);
        assert_eq!(response.biz_tags.len(), 1);
        assert_eq!(response.page, 1); // offset=0, limit=5 → page 1
        assert_eq!(response.page_size, 5);
    }

    #[tokio::test]
    async fn test_list_biz_tags_with_pagination_zero_limit_errors() {
        let handlers = make_handlers(MockBizTagTestService::new());

        let result = handlers
            .list_biz_tags_with_pagination(Some(Uuid::new_v4()), None, 0, 0)
            .await;
        assert!(result.is_err());
        match result.unwrap_err() {
            CoreError::InvalidInput(msg) => {
                assert!(msg.contains("Pagination limit cannot be zero"))
            }
            other => panic!("Expected InvalidInput, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_list_biz_tags_with_pagination_page_calculation() {
        // offset=10, limit=5 → page = (10/5)+1 = 3
        let mut mock = MockBizTagTestService::new();
        mock.expect_list_biz_tags()
            .return_once(|_, _, _, _| Ok(Vec::new()));
        mock.expect_count_biz_tags().return_once(|_, _| Ok(15));
        let handlers = make_handlers(mock);

        let result = handlers
            .list_biz_tags_with_pagination(Some(Uuid::new_v4()), None, 5, 10)
            .await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.page, 3);
        assert_eq!(response.page_size, 5);
    }

    #[tokio::test]
    async fn test_list_biz_tags_with_pagination_nil_workspace_when_none() {
        // workspace_id=None 时使用 Uuid::nil() 作为 fallback。
        let mut mock = MockBizTagTestService::new();
        mock.expect_list_biz_tags()
            .withf(|ws, _, _, _| *ws == Uuid::nil())
            .return_once(|_, _, _, _| Ok(Vec::new()));
        mock.expect_count_biz_tags()
            .withf(|ws, _| *ws == Uuid::nil())
            .return_once(|_, _| Ok(0));
        let handlers = make_handlers(mock);

        let result = handlers
            .list_biz_tags_with_pagination(None, None, 10, 0)
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_list_biz_tags_with_pagination_service_error() {
        let mut mock = MockBizTagTestService::new();
        mock.expect_list_biz_tags()
            .return_once(|_, _, _, _| Err(CoreError::InternalError("db err".to_string())));
        let handlers = make_handlers(mock);

        let result = handlers
            .list_biz_tags_with_pagination(Some(Uuid::new_v4()), None, 10, 0)
            .await;
        assert!(result.is_err());
    }

    // ===== delete_biz_tag =====

    #[tokio::test]
    async fn test_delete_biz_tag_happy_path() {
        let mut mock = MockBizTagTestService::new();
        mock.expect_delete_biz_tag().return_once(|_| Ok(()));
        let handlers = make_handlers(mock);

        let result = handlers.delete_biz_tag(Uuid::new_v4()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_delete_biz_tag_service_error() {
        let mut mock = MockBizTagTestService::new();
        mock.expect_delete_biz_tag()
            .return_once(|_| Err(CoreError::NotFound("not found".to_string())));
        let handlers = make_handlers(mock);

        let result = handlers.delete_biz_tag(Uuid::new_v4()).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), CoreError::NotFound(_)));
    }

    #[tokio::test]
    async fn test_delete_biz_tag_internal_error() {
        let mut mock = MockBizTagTestService::new();
        mock.expect_delete_biz_tag()
            .return_once(|_| Err(CoreError::InternalError("db error".to_string())));
        let handlers = make_handlers(mock);

        let result = handlers.delete_biz_tag(Uuid::new_v4()).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), CoreError::InternalError(_)));
    }
}
