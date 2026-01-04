-- Nebula ID Generator Database Schema
-- PostgreSQL initialization script

-- Create extensions
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";

-- Create enums
DO $$ BEGIN
    CREATE TYPE algorithm_type AS ENUM ('segment', 'snowflake', 'uuid_v7', 'uuid_v4');
EXCEPTION
    WHEN duplicate_object THEN null;
END $$;

DO $$ BEGIN
    CREATE TYPE id_format AS ENUM ('numeric', 'prefixed', 'uuid');
EXCEPTION
    WHEN duplicate_object THEN null;
END $$;

DO $$ BEGIN
    CREATE TYPE workspace_status AS ENUM ('active', 'inactive', 'suspended');
EXCEPTION
    WHEN duplicate_object THEN null;
END $$;

-- Workspaces table
CREATE TABLE IF NOT EXISTS workspaces (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name VARCHAR(255) NOT NULL UNIQUE,
    description TEXT,
    status workspace_status DEFAULT 'active',
    max_groups INT DEFAULT 100,
    max_biz_tags INT DEFAULT 1000,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

-- Groups table
CREATE TABLE IF NOT EXISTS groups (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id UUID NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    name VARCHAR(255) NOT NULL,
    description TEXT,
    max_biz_tags INT DEFAULT 100,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(workspace_id, name)
);

-- Business tags table
CREATE TABLE IF NOT EXISTS biz_tags (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id UUID NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    group_id UUID NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
    name VARCHAR(255) NOT NULL,
    description TEXT,
    algorithm algorithm_type DEFAULT 'segment',
    format id_format DEFAULT 'numeric',
    prefix VARCHAR(50) DEFAULT '',
    base_step INT DEFAULT 1000,
    max_step INT DEFAULT 100000,
    datacenter_ids INT[] DEFAULT ARRAY[0],
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(workspace_id, group_id, name)
);

-- API Keys table
DO $$ BEGIN
    CREATE TYPE api_key_role AS ENUM ('admin', 'user');
EXCEPTION
    WHEN duplicate_object THEN null;
END $$;

-- Drop existing table if upgrading (uncomment if needed for migration)
-- DROP TABLE IF EXISTS api_keys CASCADE;

CREATE TABLE IF NOT EXISTS api_keys (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id UUID NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    key_id VARCHAR(36) NOT NULL UNIQUE,  -- Public key identifier (UUID format)
    key_secret_hash VARCHAR(64) NOT NULL,  -- SHA-256 hash of key_secret
    key_prefix VARCHAR(8) NOT NULL,  -- niad_ for admin, nino_ for user
    role api_key_role DEFAULT 'user',
    name VARCHAR(255) NOT NULL,
    description TEXT,
    rate_limit INT DEFAULT 10000,
    enabled BOOLEAN DEFAULT true,
    expires_at TIMESTAMP WITH TIME ZONE DEFAULT (CURRENT_TIMESTAMP + INTERVAL '30 days'),
    last_used_at TIMESTAMP WITH TIME ZONE,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_api_keys_workspace ON api_keys(workspace_id);
CREATE INDEX IF NOT EXISTS idx_api_keys_key_id ON api_keys(key_id);
CREATE INDEX IF NOT EXISTS idx_api_keys_prefix ON api_keys(key_prefix);
CREATE INDEX IF NOT EXISTS idx_api_keys_role ON api_keys(role);

-- Segments table (号段分配表)
CREATE TABLE IF NOT EXISTS segments (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id UUID NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    group_id UUID NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
    biz_tag_id UUID NOT NULL REFERENCES biz_tags(id) ON DELETE CASCADE,
    datacenter_id INT NOT NULL DEFAULT 0,
    worker_id INT NOT NULL DEFAULT 0,
    start_id BIGINT NOT NULL,
    max_id BIGINT NOT NULL,
    current_id BIGINT NOT NULL,
    step INT NOT NULL DEFAULT 1000,
    version INT NOT NULL DEFAULT 0,
    status VARCHAR(20) DEFAULT 'active',
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(biz_tag_id, datacenter_id, worker_id)
);

CREATE INDEX IF NOT EXISTS idx_segments_biz_tag ON segments(biz_tag_id);
CREATE INDEX IF NOT EXISTS idx_segments_datacenter ON segments(datacenter_id);
CREATE INDEX IF NOT EXISTS idx_segments_status ON segments(status);

-- Worker nodes table
CREATE TABLE IF NOT EXISTS worker_nodes (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    node_id VARCHAR(255) NOT NULL UNIQUE,
    datacenter_id INT NOT NULL DEFAULT 0,
    worker_id INT NOT NULL,
    status VARCHAR(20) DEFAULT 'active',
    hostname VARCHAR(255),
    last_heartbeat TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(datacenter_id, worker_id)
);

-- Audit logs table
CREATE TABLE IF NOT EXISTS audit_logs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id UUID REFERENCES workspaces(id) ON DELETE SET NULL,
    user_id VARCHAR(255),
    action VARCHAR(100) NOT NULL,
    resource_type VARCHAR(100),
    resource_id VARCHAR(255),
    details JSONB,
    ip_address INET,
    user_agent TEXT,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_audit_logs_workspace ON audit_logs(workspace_id);
CREATE INDEX IF NOT EXISTS idx_audit_logs_created ON audit_logs(created_at);

-- ID generation logs (sampled)
CREATE TABLE IF NOT EXISTS id_generation_logs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id UUID NOT NULL,
    group_id UUID NOT NULL,
    biz_tag_id UUID NOT NULL,
    algorithm VARCHAR(50) NOT NULL,
    id_value VARCHAR(255) NOT NULL,
    latency_ms DECIMAL(10, 3),
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP
) PARTITION BY RANGE (created_at);

CREATE INDEX IF NOT EXISTS idx_id_gen_logs_lookup ON id_generation_logs(workspace_id, group_id, biz_tag_id);

-- Initialize default workspace and group
INSERT INTO workspaces (id, name, description)
VALUES ('11111111-1111-1111-1111-111111111111', 'default', 'Default workspace')
ON CONFLICT (name) DO NOTHING;

INSERT INTO groups (id, workspace_id, name, description)
SELECT gen_random_uuid(), id, 'default', 'Default group'
FROM workspaces WHERE name = 'default'
ON CONFLICT (workspace_id, name) DO NOTHING;

-- Insert test biz_tag
INSERT INTO biz_tags (id, workspace_id, group_id, name, algorithm, base_step)
SELECT 
    '22222222-2222-2222-2222-222222222222',
    w.id,
    g.id,
    'test-order',
    'segment',
    1000
FROM workspaces w, groups g
WHERE w.name = 'default' AND g.name = 'default'
ON CONFLICT (workspace_id, group_id, name) DO NOTHING;

-- Initialize segment for test biz_tag
INSERT INTO segments (id, workspace_id, group_id, biz_tag_id, datacenter_id, worker_id, start_id, max_id, current_id, step)
SELECT 
    '33333333-3333-3333-3333-333333333333',
    s.workspace_id,
    s.group_id,
    s.id,
    0,
    0,
    1,
    1000000000000000,
    1,
    1000
FROM workspaces w, groups g, biz_tags s
WHERE w.name = 'default' AND g.name = 'default' AND s.name = 'test-order'
ON CONFLICT (biz_tag_id, datacenter_id, worker_id) DO NOTHING;
