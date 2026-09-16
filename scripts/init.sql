-- Nebula ID Generator Database Schema
-- PostgreSQL initialization script
--
-- 本脚本是 `src/core/database/connection.rs::run_migrations`（`nebula-id
-- migrate` 命令，schema 事实源）的运维镜像，用于 DBA 预置数据库。两者必须
-- 保持一致：实体枚举（biz_tag_entity 的 AlgorithmTypeDb / IdFormatDb 以
-- `db_type = "Enum"` 映射）依赖本 schema 内的 ENUM 类型；workspaces.status
-- 为 VARCHAR(20)（workspace_entity 声明 String(20)），不是枚举。
-- 存量表结构修复：应用启动时的 run_migrations 只对缺失表执行 CREATE
-- TABLE IF NOT EXISTS，不会改写旧 init.sql 建出的旧形态表（如 segments、
-- VARCHAR(36) 的 api_keys.key_id）——旧库需按迁移 DDL 手工对齐。

-- Create schema
CREATE SCHEMA IF NOT EXISTS nebula_id;

-- 枚举类型必须先于 biz_tags 表创建（entity 生成的 CAST 不带 schema 限定，
-- 连接的 search_path 需包含 nebula_id；应用连接串由 create_connection
-- 自动注入 search_path=nebula_id,public）
DO $$ BEGIN
    CREATE TYPE nebula_id.algorithm_type AS ENUM ('segment', 'snowflake', 'uuid_v8');
EXCEPTION
    WHEN duplicate_object THEN null;
END $$;

DO $$ BEGIN
    CREATE TYPE nebula_id.id_format AS ENUM ('numeric', 'prefixed', 'uuid');
EXCEPTION
    WHEN duplicate_object THEN null;
END $$;

-- Workspaces table
CREATE TABLE IF NOT EXISTS nebula_id.workspaces (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name VARCHAR(255) NOT NULL UNIQUE,
    description TEXT,
    status VARCHAR(20) DEFAULT 'active',
    max_groups INT DEFAULT 100,
    max_biz_tags INT DEFAULT 1000,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- Groups table
CREATE TABLE IF NOT EXISTS nebula_id.groups (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id UUID NOT NULL REFERENCES nebula_id.workspaces(id) ON DELETE CASCADE,
    name VARCHAR(255) NOT NULL,
    description TEXT,
    max_biz_tags INT DEFAULT 100,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(workspace_id, name)
);

-- Business tags table
CREATE TABLE IF NOT EXISTS nebula_id.biz_tags (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id UUID NOT NULL REFERENCES nebula_id.workspaces(id) ON DELETE CASCADE,
    group_id UUID NOT NULL REFERENCES nebula_id.groups(id) ON DELETE CASCADE,
    name VARCHAR(255) NOT NULL,
    description TEXT,
    algorithm nebula_id.algorithm_type NOT NULL DEFAULT 'segment',
    format nebula_id.id_format DEFAULT 'numeric',
    prefix VARCHAR(50) DEFAULT '',
    base_step INT DEFAULT 1000,
    max_step INT DEFAULT 100000,
    datacenter_ids JSONB DEFAULT '[]',
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(workspace_id, group_id, name)
);

-- API Keys table
CREATE TABLE IF NOT EXISTS nebula_id.api_keys (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    key_id VARCHAR(64) NOT NULL UNIQUE,
    key_secret_hash VARCHAR(128) NOT NULL,
    prev_secret_hash VARCHAR(128),
    rotate_expires_at TIMESTAMP,
    key_prefix VARCHAR(16) NOT NULL,
    role VARCHAR(20) NOT NULL DEFAULT 'user',
    workspace_id UUID,  -- 允许 NULL，用于全局 admin key
    name VARCHAR(255) NOT NULL,
    description TEXT,
    rate_limit INT DEFAULT 1000,
    enabled BOOLEAN DEFAULT true,
    expires_at TIMESTAMP,
    last_used_at TIMESTAMP,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    CONSTRAINT check_admin_key CHECK (
        (workspace_id IS NULL AND role = 'admin')
        OR (workspace_id IS NOT NULL AND role != 'admin')
    )
);

-- 号段分配表（segment_entity: table_name = "nebula_segments"）
CREATE TABLE IF NOT EXISTS nebula_id.nebula_segments (
    id BIGSERIAL PRIMARY KEY,
    workspace_id VARCHAR(255) NOT NULL,
    biz_tag VARCHAR(255) NOT NULL,
    current_id BIGINT NOT NULL,
    max_id BIGINT NOT NULL,
    step INT NOT NULL DEFAULT 1000,
    delta INT NOT NULL DEFAULT 1,
    dc_id INT NOT NULL DEFAULT 0,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    -- 号段原子分配（单语句 UPDATE ... RETURNING）依赖此约束：
    -- 1) INSERT ... ON CONFLICT (workspace_id, biz_tag, dc_id) DO NOTHING
    --    需要它匹配冲突目标；
    -- 2) 杜绝并发首分配插入重复行（否则 UPDATE ... RETURNING 会命中多行，
    --    区间可能交叠）。非 dc 变体按 dc_id = 0 读写，dc 变体按 dc_id 读写。
    CONSTRAINT uq_nebula_segments_ws_tag_dc UNIQUE (workspace_id, biz_tag, dc_id)
);

-- Worker nodes table（运维预留：实体暂未使用）
CREATE TABLE IF NOT EXISTS nebula_id.worker_nodes (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    node_id VARCHAR(255) NOT NULL UNIQUE,
    datacenter_id INT NOT NULL DEFAULT 0,
    worker_id INT NOT NULL,
    status VARCHAR(20) DEFAULT 'active',
    hostname VARCHAR(255),
    last_heartbeat TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(datacenter_id, worker_id)
);

-- Audit logs table（运维预留：实体暂未使用）
CREATE TABLE IF NOT EXISTS nebula_id.audit_logs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id UUID REFERENCES nebula_id.workspaces(id) ON DELETE SET NULL,
    user_id VARCHAR(255),
    action VARCHAR(100) NOT NULL,
    resource_type VARCHAR(100),
    resource_id VARCHAR(255),
    details JSONB,
    ip_address INET,
    user_agent TEXT,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_audit_logs_workspace ON nebula_id.audit_logs(workspace_id);
CREATE INDEX IF NOT EXISTS idx_audit_logs_created ON nebula_id.audit_logs(created_at);

-- ID generation logs (sampled，运维预留：实体暂未使用)
CREATE TABLE IF NOT EXISTS nebula_id.id_generation_logs (
    id UUID DEFAULT gen_random_uuid(),
    workspace_id UUID NOT NULL,
    group_id UUID NOT NULL,
    biz_tag_id UUID NOT NULL,
    algorithm VARCHAR(50) NOT NULL,
    id_value VARCHAR(255) NOT NULL,
    latency_ms DECIMAL(10, 3),
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (id, created_at)
) PARTITION BY RANGE (created_at);

-- Create default partition
CREATE TABLE IF NOT EXISTS nebula_id.id_generation_logs_default PARTITION OF nebula_id.id_generation_logs DEFAULT;

CREATE INDEX IF NOT EXISTS idx_id_gen_logs_lookup ON nebula_id.id_generation_logs(workspace_id, group_id, biz_tag_id);
