use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::database::{AlgorithmType, BizTagRepository, IdFormat, UpdateBizTagRequest};
use crate::types::Result;

/// 动态配置请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DynamicConfigRequest {
    pub workspace_id: Uuid,
    pub group_id: Option<Uuid>,
    pub biz_tag: String,
    pub algorithm: Option<AlgorithmType>,
    pub format: Option<IdFormat>,
    pub prefix: Option<String>,
    pub base_step: Option<i32>,
    pub max_step: Option<i32>,
    pub datacenter_ids: Option<Vec<i32>>,
}

/// 动态配置响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DynamicConfigResponse {
    pub workspace_id: Uuid,
    pub group_id: Uuid,
    pub biz_tag_id: Uuid,
    pub biz_tag: String,
    pub algorithm: AlgorithmType,
    pub format: IdFormat,
    pub prefix: String,
    pub base_step: i32,
    pub max_step: i32,
    pub datacenter_ids: Vec<i32>,
}

/// 动态配置管理服务
pub struct DynamicConfigService<R>
where
    R: BizTagRepository + Send + Sync,
{
    repository: Arc<R>,
}

impl<R> DynamicConfigService<R>
where
    R: BizTagRepository + Send + Sync,
{
    /// 创建新的动态配置服务实例
    ///
    /// # Arguments
    /// * `repository` - 业务标签仓库实现
    ///
    /// # Returns
    /// 返回动态配置服务实例
    pub fn new(repository: Arc<R>) -> Self {
        Self { repository }
    }

    /// 更新业务标签的配置参数
    ///
    /// # Arguments
    /// * `request` - 动态配置更新请求
    ///
    /// # Returns
    /// 返回更新后的配置信息
    ///
    /// # Errors
    /// 当业务标签不存在或请求参数无效时返回错误
    pub async fn update_config(
        &self,
        request: &DynamicConfigRequest,
    ) -> Result<DynamicConfigResponse> {
        // 验证请求参数
        if request.biz_tag.trim().is_empty() {
            return Err(crate::CoreError::InvalidInput(
                "BizTag name cannot be empty".to_string(),
            ));
        }

        if request.group_id.is_none() {
            return Err(crate::CoreError::InvalidInput(
                "Group ID is required to update configuration".to_string(),
            ));
        }

        // 根据workspace_id, group_id和biz_tag名称查找业务标签
        let biz_tag_opt = if let Some(group_id) = request.group_id {
            self.repository
                .get_biz_tag_by_workspace_group_and_name(
                    request.workspace_id,
                    group_id,
                    &request.biz_tag,
                )
                .await?
        } else {
            // 如果没有提供group_id，我们需要先找到该workspace下的某个组中的biz_tag
            // 这里假设我们需要先通过其他方式确定组，或者查找所有组中的biz_tag
            // 为了简化，我们假设用户必须提供group_id
            return Err(crate::CoreError::InvalidInput(
                "Group ID is required to update configuration".to_string(),
            ));
        };

        let mut biz_tag = biz_tag_opt.ok_or_else(|| {
            crate::CoreError::NotFound(format!(
                "BizTag not found: workspace_id={}, group_id={:?}, name={}",
                request.workspace_id, request.group_id, request.biz_tag
            ))
        })?;

        // 更新业务标签配置
        let update_request = UpdateBizTagRequest {
            name: None,        // 不更新名称
            description: None, // 不更新描述
            algorithm: request.algorithm.clone(),
            format: request.format.clone(),
            prefix: request.prefix.clone(),
            base_step: request.base_step,
            max_step: request.max_step,
            datacenter_ids: request.datacenter_ids.clone(),
        };

        let updated_biz_tag = self
            .repository
            .update_biz_tag(biz_tag.id, &update_request)
            .await?;

        Ok(DynamicConfigResponse {
            workspace_id: updated_biz_tag.workspace_id,
            group_id: updated_biz_tag.group_id,
            biz_tag_id: updated_biz_tag.id,
            biz_tag: updated_biz_tag.name,
            algorithm: updated_biz_tag.algorithm,
            format: updated_biz_tag.format,
            prefix: updated_biz_tag.prefix,
            base_step: updated_biz_tag.base_step,
            max_step: updated_biz_tag.max_step,
            datacenter_ids: updated_biz_tag.datacenter_ids,
        })
    }

    /// 获取业务标签的配置参数
    ///
    /// # Arguments
    /// * `workspace_id` - 工作空间ID
    /// * `group_id` - 组ID
    /// * `biz_tag` - 业务标签名称
    ///
    /// # Returns
    /// 返回可选的配置信息，如果不存在则返回None
    ///
    /// # Errors
    /// 当数据库操作失败时返回错误
    pub async fn get_config(
        &self,
        workspace_id: Uuid,
        group_id: Uuid,
        biz_tag: &str,
    ) -> Result<Option<DynamicConfigResponse>> {
        // 验证请求参数
        if biz_tag.trim().is_empty() {
            return Err(crate::CoreError::InvalidInput(
                "BizTag name cannot be empty".to_string(),
            ));
        }

        let biz_tag_opt = self
            .repository
            .get_biz_tag_by_workspace_group_and_name(workspace_id, group_id, biz_tag)
            .await?;

        if let Some(biz_tag_model) = biz_tag_opt {
            Ok(Some(DynamicConfigResponse {
                workspace_id: biz_tag_model.workspace_id,
                group_id: biz_tag_model.group_id,
                biz_tag_id: biz_tag_model.id,
                biz_tag: biz_tag_model.name,
                algorithm: biz_tag_model.algorithm,
                format: biz_tag_model.format,
                prefix: biz_tag_model.prefix,
                base_step: biz_tag_model.base_step,
                max_step: biz_tag_model.max_step,
                datacenter_ids: biz_tag_model.datacenter_ids,
            }))
        } else {
            Ok(None)
        }
    }

    /// 批量更新配置参数
    ///
    /// # Arguments
    /// * `requests` - 动态配置更新请求列表
    ///
    /// # Returns
    /// 返回更新后的配置信息列表
    ///
    /// # Errors
    /// 当任一业务标签不存在或请求参数无效时返回错误
    pub async fn batch_update_config(
        &self,
        requests: Vec<DynamicConfigRequest>,
    ) -> Result<Vec<DynamicConfigResponse>> {
        // 验证请求参数
        if requests.is_empty() {
            return Err(crate::CoreError::InvalidInput(
                "Request list cannot be empty".to_string(),
            ));
        }

        for request in &requests {
            if request.biz_tag.trim().is_empty() {
                return Err(crate::CoreError::InvalidInput(
                    "BizTag name cannot be empty".to_string(),
                ));
            }

            if request.group_id.is_none() {
                return Err(crate::CoreError::InvalidInput(
                    "Group ID is required to update configuration".to_string(),
                ));
            }
        }

        let mut results = Vec::new();

        for request in requests {
            match self.update_config(&request).await {
                Ok(response) => results.push(response),
                Err(e) => return Err(e), // 简化处理，遇到错误就返回
            }
        }

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::{BizTag, CreateBizTagRequest, UpdateBizTagRequest};
    use crate::types::id::{AlgorithmType, IdFormat};
    use mockall::{mock, predicate};
    use uuid::Uuid;

    // 创建模拟仓库实现用于测试
    mock! {
        pub TestRepository {}

        #[async_trait]
        impl BizTagRepository for TestRepository {
            async fn create_biz_tag(&self, biz_tag: &CreateBizTagRequest) -> Result<BizTag>;
            async fn get_biz_tag(&self, id: Uuid) -> Result<Option<BizTag>>;
            async fn get_biz_tag_by_workspace_group_and_name(&self, workspace_id: Uuid, group_id: Uuid, name: &str) -> Result<Option<BizTag>>;
            async fn update_biz_tag(&self, id: Uuid, biz_tag: &UpdateBizTagRequest) -> Result<BizTag>;
            async fn delete_biz_tag(&self, id: Uuid) -> Result<()>;
            async fn list_biz_tags(&self, workspace_id: Uuid, group_id: Option<Uuid>, limit: Option<u32>, offset: Option<u32>) -> Result<Vec<BizTag>>;
        }
    }

    #[tokio::test]
    async fn test_batch_update_config_with_empty_list_should_fail() {
        let mock_repo = MockTestRepository::new();
        let requests = vec![];

        let service = DynamicConfigService::new(Arc::new(mock_repo));
        let result: Result<Vec<DynamicConfigResponse>> =
            service.batch_update_config(requests).await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::CoreError::InvalidInput(_)
        ));
    }

    #[tokio::test]
    async fn test_update_config_with_valid_request() {
        let mut mock_repo = MockTestRepository::new();
        let workspace_id = Uuid::new_v4();
        let group_id = Uuid::new_v4();
        let request = DynamicConfigRequest {
            workspace_id,
            group_id: Some(group_id),
            biz_tag: "test_biz_tag".to_string(),
            algorithm: Some(AlgorithmType::Segment),
            format: Some(IdFormat::Numeric),
            prefix: Some("TEST".to_string()),
            base_step: Some(100),
            max_step: Some(1000),
            datacenter_ids: Some(vec![0]),
        };

        let existing_biz_tag = BizTag {
            id: Uuid::new_v4(),
            workspace_id,
            group_id,
            name: "test_biz_tag".to_string(),
            description: Some("Test BizTag".to_string()),
            algorithm: AlgorithmType::Snowflake,
            format: IdFormat::Uuid,
            prefix: "OLD".to_string(),
            base_step: 50,
            max_step: 500,
            datacenter_ids: vec![1],
            created_at: chrono::Utc::now().naive_utc(),
            updated_at: chrono::Utc::now().naive_utc(),
        };

        let updated_biz_tag = BizTag {
            id: Uuid::new_v4(),
            workspace_id,
            group_id,
            name: "test_biz_tag".to_string(),
            description: Some("Test BizTag".to_string()),
            algorithm: AlgorithmType::Segment,
            format: IdFormat::Numeric,
            prefix: "TEST".to_string(),
            base_step: 100,
            max_step: 1000,
            datacenter_ids: vec![0],
            created_at: chrono::Utc::now().naive_utc(),
            updated_at: chrono::Utc::now().naive_utc(),
        };

        // Mock the get_biz_tag_by_workspace_group_and_name call
        mock_repo
            .expect_get_biz_tag_by_workspace_group_and_name()
            .with(
                predicate::eq(workspace_id),
                predicate::eq(group_id),
                predicate::eq("test_biz_tag".to_string()),
            )
            .times(1)
            .returning(move |_, _, _| Ok(Some(existing_biz_tag.clone())));

        // Mock the update_biz_tag call
        mock_repo
            .expect_update_biz_tag()
            .times(1)
            .returning(move |_, _| Ok(updated_biz_tag.clone()));

        let service = DynamicConfigService::new(Arc::new(mock_repo));
        let result: Result<DynamicConfigResponse> = service.update_config(&request).await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_update_config_with_empty_biz_tag_name_should_fail() {
        let mock_repo = MockTestRepository::new();
        let request = DynamicConfigRequest {
            workspace_id: Uuid::new_v4(),
            group_id: Some(Uuid::new_v4()),
            biz_tag: "".to_string(),
            algorithm: Some(AlgorithmType::Segment),
            format: Some(IdFormat::Numeric),
            prefix: Some("TEST".to_string()),
            base_step: Some(100),
            max_step: Some(1000),
            datacenter_ids: Some(vec![0]),
        };

        let service = DynamicConfigService::new(Arc::new(mock_repo));
        let result: Result<DynamicConfigResponse> = service.update_config(&request).await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::CoreError::InvalidInput(_)
        ));
    }

    #[tokio::test]
    async fn test_update_config_without_group_id_should_fail() {
        let mock_repo = MockTestRepository::new();
        let request = DynamicConfigRequest {
            workspace_id: Uuid::new_v4(),
            group_id: None, // Missing group_id
            biz_tag: "test_biz_tag".to_string(),
            algorithm: Some(AlgorithmType::Segment),
            format: Some(IdFormat::Numeric),
            prefix: Some("TEST".to_string()),
            base_step: Some(100),
            max_step: Some(1000),
            datacenter_ids: Some(vec![0]),
        };

        let service = DynamicConfigService::new(Arc::new(mock_repo));
        let result: Result<DynamicConfigResponse> = service.update_config(&request).await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::CoreError::InvalidInput(_)
        ));
    }

    #[tokio::test]
    async fn test_get_config_with_valid_request() {
        let mut mock_repo = MockTestRepository::new();
        let workspace_id = Uuid::new_v4();
        let group_id = Uuid::new_v4();
        let biz_tag_name = "test_biz_tag";

        let biz_tag = BizTag {
            id: Uuid::new_v4(),
            workspace_id,
            group_id,
            name: biz_tag_name.to_string(),
            description: Some("Test BizTag".to_string()),
            algorithm: AlgorithmType::Snowflake,
            format: IdFormat::Prefixed,
            prefix: "TEST".to_string(),
            base_step: 100,
            max_step: 1000,
            datacenter_ids: vec![1, 2],
            created_at: chrono::Utc::now().naive_utc(),
            updated_at: chrono::Utc::now().naive_utc(),
        };

        mock_repo
            .expect_get_biz_tag_by_workspace_group_and_name()
            .with(
                predicate::eq(workspace_id),
                predicate::eq(group_id),
                predicate::eq(biz_tag_name.to_string()),
            )
            .times(1)
            .returning(move |_, _, _| Ok(Some(biz_tag.clone())));

        let service = DynamicConfigService::new(Arc::new(mock_repo));
        let result: Result<Option<DynamicConfigResponse>> = service
            .get_config(workspace_id, group_id, biz_tag_name)
            .await;

        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
    }

    #[tokio::test]
    async fn test_get_config_with_empty_biz_tag_name_should_fail() {
        let mock_repo = MockTestRepository::new();
        let workspace_id = Uuid::new_v4();
        let group_id = Uuid::new_v4();
        let biz_tag_name = "";

        let service = DynamicConfigService::new(Arc::new(mock_repo));
        let result: Result<Option<DynamicConfigResponse>> = service
            .get_config(workspace_id, group_id, biz_tag_name)
            .await;

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::CoreError::InvalidInput(_)
        ));
    }

    #[tokio::test]
    async fn test_batch_update_config_with_valid_requests() {
        let mut mock_repo = MockTestRepository::new();
        let workspace_id = Uuid::new_v4();
        let group_id = Uuid::new_v4();

        let requests = vec![
            DynamicConfigRequest {
                workspace_id,
                group_id: Some(group_id),
                biz_tag: "test_biz_tag_1".to_string(),
                algorithm: Some(AlgorithmType::Segment),
                format: Some(IdFormat::Numeric),
                prefix: Some("TEST1".to_string()),
                base_step: Some(100),
                max_step: Some(1000),
                datacenter_ids: Some(vec![0]),
            },
            DynamicConfigRequest {
                workspace_id,
                group_id: Some(group_id),
                biz_tag: "test_biz_tag_2".to_string(),
                algorithm: Some(AlgorithmType::Snowflake),
                format: Some(IdFormat::Prefixed),
                prefix: Some("TEST2".to_string()),
                base_step: Some(200),
                max_step: Some(2000),
                datacenter_ids: Some(vec![1]),
            },
        ];

        // Mock the get_biz_tag_by_workspace_group_and_name calls
        mock_repo
            .expect_get_biz_tag_by_workspace_group_and_name()
            .with(
                predicate::eq(workspace_id),
                predicate::eq(group_id),
                predicate::eq("test_biz_tag_1".to_string()),
            )
            .times(1)
            .returning({
                let biz_tag = BizTag {
                    id: Uuid::new_v4(),
                    workspace_id,
                    group_id,
                    name: "test_biz_tag_1".to_string(),
                    description: Some("Test BizTag 1".to_string()),
                    algorithm: AlgorithmType::Snowflake,
                    format: IdFormat::Uuid,
                    prefix: "OLD1".to_string(),
                    base_step: 50,
                    max_step: 500,
                    datacenter_ids: vec![1],
                    created_at: chrono::Utc::now().naive_utc(),
                    updated_at: chrono::Utc::now().naive_utc(),
                };
                move |_, _, _| Ok(Some(biz_tag.clone()))
            });

        mock_repo.expect_update_biz_tag().times(1).returning({
            let updated_biz_tag = BizTag {
                id: Uuid::new_v4(),
                workspace_id,
                group_id,
                name: "test_biz_tag_1".to_string(),
                description: Some("Test BizTag 1".to_string()),
                algorithm: AlgorithmType::Segment,
                format: IdFormat::Numeric,
                prefix: "TEST1".to_string(),
                base_step: 100,
                max_step: 1000,
                datacenter_ids: vec![0],
                created_at: chrono::Utc::now().naive_utc(),
                updated_at: chrono::Utc::now().naive_utc(),
            };
            move |_, _| Ok(updated_biz_tag.clone())
        });

        mock_repo
            .expect_get_biz_tag_by_workspace_group_and_name()
            .with(
                predicate::eq(workspace_id),
                predicate::eq(group_id),
                predicate::eq("test_biz_tag_2".to_string()),
            )
            .times(1)
            .returning({
                let biz_tag = BizTag {
                    id: Uuid::new_v4(),
                    workspace_id,
                    group_id,
                    name: "test_biz_tag_2".to_string(),
                    description: Some("Test BizTag 2".to_string()),
                    algorithm: AlgorithmType::Segment,
                    format: IdFormat::Numeric,
                    prefix: "OLD2".to_string(),
                    base_step: 150,
                    max_step: 1500,
                    datacenter_ids: vec![2],
                    created_at: chrono::Utc::now().naive_utc(),
                    updated_at: chrono::Utc::now().naive_utc(),
                };
                move |_, _, _| Ok(Some(biz_tag.clone()))
            });

        mock_repo.expect_update_biz_tag().times(1).returning({
            let updated_biz_tag = BizTag {
                id: Uuid::new_v4(),
                workspace_id,
                group_id,
                name: "test_biz_tag_2".to_string(),
                description: Some("Test BizTag 2".to_string()),
                algorithm: AlgorithmType::Snowflake,
                format: IdFormat::Prefixed,
                prefix: "TEST2".to_string(),
                base_step: 200,
                max_step: 2000,
                datacenter_ids: vec![1],
                created_at: chrono::Utc::now().naive_utc(),
                updated_at: chrono::Utc::now().naive_utc(),
            };
            move |_, _| Ok(updated_biz_tag.clone())
        });

        let service = DynamicConfigService::new(Arc::new(mock_repo));
        let result: Result<Vec<DynamicConfigResponse>> =
            service.batch_update_config(requests).await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 2);
    }
}
