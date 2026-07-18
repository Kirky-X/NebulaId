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
        let biz_tag: crate::core::database::BizTag = self
            .config_service
            .get_biz_tag(id)
            .await?
            .ok_or_else(|| CoreError::NotFound(format!("BizTag not found: {}", id)))?;

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
        let workspace_id = workspace_id.unwrap_or_else(uuid::Uuid::nil);
        let biz_tags: Vec<crate::core::database::BizTag> = self
            .config_service
            .list_biz_tags(workspace_id, group_id, None, None)
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

        let total = responses.len() as u64;
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
                "Pagination limit cannot be zero".to_string(),
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
