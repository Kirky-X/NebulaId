-- 001: nebula_segments 增加 ceiling 封顶列 + id_format 枚举追加 'obfuscated'
--       （change: fix-audit-defects-r1）
--
-- 升级硬前提：分配热路径 SQL 引用 ceiling 列，存量部署必须先执行本脚本
-- 再滚动升级服务，否则号段分配直接报错（列不存在）。
--
-- 幂等性：ADD COLUMN IF NOT EXISTS / ADD VALUE IF NOT EXISTS 均可安全重复
-- 执行（两者自 PG 9.3 起即支持）。
-- 事务注意：ALTER TYPE ... ADD VALUE 在 PG 12+ 可在事务块内执行；
-- PG 9.3~11 需在事务块外（psql 默认 autocommit 直接执行即可）。
ALTER TABLE nebula_id.nebula_segments
    ADD COLUMN IF NOT EXISTS ceiling BIGINT;

ALTER TYPE nebula_id.id_format ADD VALUE IF NOT EXISTS 'obfuscated';
