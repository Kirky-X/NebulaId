-- 001: nebula_segments 增加 ceiling 封顶列（change: fix-audit-defects-r1）
--
-- 升级硬前提：分配热路径 SQL 引用 ceiling 列，存量部署必须先执行本脚本
-- 再滚动升级服务，否则号段分配直接报错（列不存在）。
-- 幂等：IF NOT EXISTS，可安全重复执行。
ALTER TABLE nebula_id.nebula_segments
    ADD COLUMN IF NOT EXISTS ceiling BIGINT;

-- 同批：id_format 枚举追加 'obfuscated'（防猜测混淆格式，biz_tag 可选）。
-- 幂等：PG 无 ADD VALUE IF NOT EXISTS（14 前无），用 DO 块判存在；
-- 注意 ALTER TYPE ADD VALUE 不能在事务块内（PG<12），执行时若报错请
-- 单独以 autocommit 模式运行本段。
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_enum e
        JOIN pg_type t ON t.oid = e.enumtypid
        WHERE t.typname = 'id_format' AND e.enumlabel = 'obfuscated'
    ) THEN
        ALTER TYPE nebula_id.id_format ADD VALUE 'obfuscated';
    END IF;
END
$$;
