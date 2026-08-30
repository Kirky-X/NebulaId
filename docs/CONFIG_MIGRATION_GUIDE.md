# Nebula ID 配置迁移指南

## 安全加固更新 (v0.1.2)

### 重要变更

从 v0.1.2 开始，Nebula ID 实施了更严格的安全策略，所有配置文件中的敏感信息必须通过环境变量提供。

### 数据库密码配置

#### 之前的方式（已废弃）
```toml
[database]
password = "idgen123"
```

#### 新的方式（强制）

**方法 1: 使用环境变量展开**
```toml
[database]
password = "${NEBULA_DATABASE_PASSWORD}"
```

设置环境变量：
```bash
export NEBULA_DATABASE_PASSWORD="your_secure_password_here"
```

**方法 2: 使用完整数据库 URL**
```toml
[database]
url = "${DATABASE_URL}"
```

设置环境变量：
```bash
export DATABASE_URL="postgresql://idgen:your_password@localhost:5432/idgen"
```

### 快速开始

#### 开发环境

1. 复制示例环境变量文件：
```bash
cp docker/.env.example .env
```

2. 生成强密码：
```bash
# Linux/macOS
openssl rand -base64 32

# Windows PowerShell
-join ((48..57) + (65..90) + (97..122) | Get-Random -Count 32 | ForEach-Object {[char]$_})
```

3. 编辑 `.env` 文件，设置密码：
```bash
POSTGRES_PASSWORD=your_generated_password
NEBULA_DATABASE_PASSWORD=your_generated_password
DATABASE_URL=postgresql://idgen:your_generated_password@localhost:5432/idgen
```

4. 启动服务：
```bash
docker-compose up -d
```

#### 生产环境

**必须设置的变量：**

```bash
# 数据库密码（必须使用强密码）
export NEBULA_DATABASE_PASSWORD="$(openssl rand -base64 32)"

# 或者使用完整 URL
export DATABASE_URL="postgresql://idgen:$(openssl rand -base64 32)@db-host:5432/idgen"

# API 密钥盐值（必须设置）
export NEBULA_API_KEY_SALT="$(openssl rand -hex 32)"
```

**Docker Compose 示例：**

```yaml
version: '3.8'
services:
  nebula-id:
    image: nebulaid/nebula-id:latest
    environment:
      - NEBULA_DATABASE_PASSWORD=${NEBULA_DATABASE_PASSWORD}
      - NEBULA_API_KEY_SALT=${NEBULA_API_KEY_SALT}
      - RUST_LOG=info
    ports:
      - "8080:8080"
    volumes:
      - ./config:/app/config
```

### 配置文件位置

- **主配置文件**: `config/config.toml`
- **无 etcd 配置**: `config/config_no_etcd.toml`
- **Docker 测试配置**: `docker/test-server-config.toml`
- **环境变量示例**: `docker/.env.example`

### 环境变量优先级

环境变量配置按以下优先级覆盖：

1. 操作系统环境变量（最高优先级）
2. `.env` 文件中的变量
3. 配置文件中的默认值（最低优先级）

### 安全建议

1. **永远不要**在配置文件中硬编码密码
2. **永远不要**提交包含真实密码的 `.env` 文件到版本控制
3. **总是**使用强密码（至少 16 位，包含大小写字母、数字和特殊字符）
4. **定期**轮换密码和密钥
5. **限制**知晓密码的人员范围

### 故障排查

#### 错误："NEBULA_DATABASE_PASSWORD environment variable must be set"

**原因**: 生产环境下未设置数据库密码环境变量

**解决方案**:
```bash
export NEBULA_DATABASE_PASSWORD="your_password"
# 或
export DATABASE_URL="postgresql://..."
```

#### 警告："Weak or empty database password detected"

**原因**: 检测到使用了弱密码或空密码

**解决方案**: 立即更改为强密码

### 向后兼容性

此变更**不向后兼容**。升级到 v0.1.2+ 后必须：

1. 更新所有配置文件使用环境变量引用
2. 设置必要的环境变量
3. 重启所有服务实例

### 需要帮助？

- 查看完整文档：[USER_GUIDE.md](../docs/USER_GUIDE.md)
- 提交问题：https://github.com/Kirky-X/NebulaId/issues
- 社区讨论：GitHub Discussions

## algorithm_type ENUM 迁移（uuid_v7 / uuid_v4 -> uuid_v8）

### 背景

代码侧 `AlgorithmTypeDb`（`src/core/database/biz_tag_entity.rs:138-147`）当前只有三个取值，
且通过 `From<AlgorithmType> for AlgorithmTypeDb` 与 `AlgorithmType`（`src/core/types/id.rs:173-188`）双向一一映射：

| DB ENUM label | SeaORM 变体 | `AlgorithmType` |
|---------------|-------------|-----------------|
| `segment`     | `AlgorithmTypeDb::Segment`   | `AlgorithmType::Segment`   |
| `snowflake`   | `AlgorithmTypeDb::Snowflake` | `AlgorithmType::Snowflake` |
| `uuid_v8`     | `AlgorithmTypeDb::UuidV8`    | `AlgorithmType::UuidV8`    |

`uuid_v7` / `uuid_v4` 只作为 `AlgorithmType::from_str` 的**输入别名**保留
（`src/core/types/id.rs:197-198`，用于 API/配置层的向后兼容），**不再是合法的 DB 取值**。
因此库里残留 `uuid_v7` / `uuid_v4` 行时，新版程序 `find` 这些 `biz_tags` 行会因无法转换成
enum 变体而直接报错。

- 新建库：`scripts/init.sql` 已改为正确取值，无需迁移。
- 存量库：必须执行本节迁移。注意 `init.sql` 的 `DO ... WHEN duplicate_object THEN null` 块
  对已存在的类型是**静默跳过**的，重复执行 `init.sql` 不会修复旧库。

受影响对象（由 `DeriveActiveEnum` 的 `enum_name = "algorithm_type"` 及表/列声明核对得出）：

| 类型 | 对象 |
|------|------|
| ENUM 类型 | `algorithm_type`（`scripts/init.sql` 在 `SET search_path TO nebula_id, public` 之后不带限定名创建，实际落在 `nebula_id` schema） |
| 使用该类型的列 | `nebula_id.biz_tags.algorithm`（`scripts/init.sql` 现为 `algorithm algorithm_type NOT NULL DEFAULT 'segment'`；存量库仍是可空列，需按下面「补充步骤」回填后收紧） |

`id_generation_logs.algorithm` 是 `VARCHAR(50)` 纯文本列，与本 ENUM 无关，无需 DDL 变更。
全库只有这一处引用该类型，下面第 1 步会实测确认。

### 0. 执行前检查（只读，先跑这段）

```sql
-- 0.1 类型实际所在的 schema（决定后续语句用什么限定名）
SELECT n.nspname AS type_schema, t.typname, t.oid AS type_oid
FROM pg_type t
JOIN pg_namespace n ON n.oid = t.typnamespace
WHERE t.typname = 'algorithm_type' AND t.typtype = 'e';

-- 0.2 当前取值清单（存量库应看到 uuid_v7 / uuid_v4）
--     注意列名是 enumsortorder（PG 9.4 前叫 enumsort，现已不存在）
SELECT e.enumsortorder, e.enumlabel
FROM pg_type t
JOIN pg_enum e ON e.enumtypid = t.oid
WHERE t.typname = 'algorithm_type'
ORDER BY e.enumsortorder;

-- 0.3 依赖该类型的全部列（预期只有 biz_tags.algorithm，一行）
SELECT table_schema, table_name, column_name, data_type, is_nullable, column_default
FROM information_schema.columns
WHERE udt_name = 'algorithm_type'
ORDER BY table_schema, table_name, column_name;

-- 0.4 需要回填的行数
SELECT algorithm::text AS algorithm_value, count(*) AS row_count
FROM nebula_id.biz_tags
GROUP BY algorithm::text
ORDER BY algorithm::text;

SELECT count(*) AS legacy_rows
FROM nebula_id.biz_tags
WHERE algorithm::text IN ('uuid_v7', 'uuid_v4');

-- 0.5 该类型的全部依赖对象（视图、函数、快照表等都会阻止 3.5 的 DROP TYPE）
--     健康库在 PG 12+ 实测返回 3 行，都属于正常：
--       pg_class  / nebula_id.biz_tags / dependent_column=algorithm  （列本身）
--       pg_attrdef / <oid>            （`DEFAULT 'segment'` 表达式，方案 B 用 DROP DEFAULT 摘掉）
--       pg_type   / <oid>  deptype='i'（枚举自动生成的数组类型 _algorithm_type，内部依赖）
--     出现这三类之外的行 = 还有真依赖（视图 / 函数 / 别的表），必须停下
SELECT d.classid::regclass AS dependent_catalog,
       CASE WHEN d.classid = 'pg_class'::regclass
            THEN d.objid::regclass::text
            ELSE d.objid::text
       END AS dependent_object,
       a.attname AS dependent_column,
       d.deptype
FROM pg_depend d
JOIN pg_type t ON t.oid = d.refobjid
LEFT JOIN pg_attribute a
       ON d.classid = 'pg_class'::regclass
      AND a.attrelid = d.objid
      AND a.attnum = d.objsubid
WHERE t.typname = 'algorithm_type'
  AND d.refclassid = 'pg_type'::regclass
ORDER BY 1, 2, 3;
```

停止条件：0.3 返回多于一行，或 0.5 出现 `biz_tags.algorithm` 之外的依赖对象时**不要**继续，
先把所有引用点一并改造；否则跳过方案 B，只做方案 A。

### 1. 备份

```bash
# 整库自定义格式（pg_restore 可细粒度选择对象）
pg_dump -U idgen -d idgen -n nebula_id -Fc \
  -f "nebula_id_pre_uuid_v8_$(date +%Y%m%d_%H%M%S).dump"
```

```sql
-- 单表文本快照：只存定位所需的列，且 algorithm 存成 text。
-- 必须用 text —— 若快照列仍是 algorithm_type 类型，快照表本身会反向依赖旧类型，
-- 导致方案 B 的 3.5 `DROP TYPE` 失败。
CREATE TABLE nebula_id.biz_tags_bak_pre_uuid_v8 AS
SELECT id, name, algorithm::text AS algorithm_label
FROM nebula_id.biz_tags;

SELECT algorithm_label, count(*) AS row_count
FROM nebula_id.biz_tags_bak_pre_uuid_v8
GROUP BY algorithm_label
ORDER BY algorithm_label;
```

权限要求：执行者必须是 `algorithm_type` 的 owner（或其成员角色）以及 `biz_tags` 的 owner。
Postgres 中该类型的 owner 是 `scripts/init.sql` 的连接用户（默认 `idgen`）。

### 2. 方案 A：在线两阶段（推荐，不停机，类型保留 2 个孤儿取值）

必须**按顺序、分两条独立语句**执行。`ALTER TYPE ... ADD VALUE` 的新取值在加入它的事务提交之前不可使用，
所以禁止把 2.1 和 2.2 包在同一个 `BEGIN ... COMMIT` 里。PG 16 实测合并执行会失败并整体回滚：

```text
ERROR:  unsafe use of new value "uuid_v8" of enum type nebula_id.algorithm_type
HINT:  New enum values must be committed before they can be used.
```

```sql
-- 2.1 先补新取值（独立语句、自动提交；IF NOT EXISTS 需要 PG 9.6+）
ALTER TYPE nebula_id.algorithm_type ADD VALUE IF NOT EXISTS 'uuid_v8';
```

```sql
-- 2.2 回填历史取值（新 binary 启动前必须完成，否则读这些行会报错）
UPDATE nebula_id.biz_tags
SET algorithm = 'uuid_v8'
WHERE algorithm::text IN ('uuid_v7', 'uuid_v4');
```

不要在这条 `UPDATE` 里顺带写 `updated_at = CURRENT_TIMESTAMP`：这是一次纯机械的枚举标签回填，
不是业务变更；改 `updated_at` 会让按更新时间做增量同步或审计的下游误判为配置变更。

```sql
-- 2.3 确认无残留后，再部署 / 重启新版 nebula-id
SELECT count(*) AS legacy_rows
FROM nebula_id.biz_tags
WHERE algorithm::text IN ('uuid_v7', 'uuid_v4');
-- 期望：0
```

方案 A 的结果：读写 `uuid_v8` 正常，旧 binary 也仍能解析 `uuid_v7` / `uuid_v4`（滚动升级期间友好）。
代价是 `algorithm_type` 里遗留 `uuid_v7` / `uuid_v4` 两个取值 —— Postgres **没有** `ALTER TYPE ... DROP VALUE`，
单个取值无法删除。要精确对齐代码取值集合，在下一个维护窗口执行方案 B。

注意 `ADD VALUE` 把新标签追加在枚举**末尾**，所以 `algorithm` 的排序键是加入顺序而不是字典序。
本项目代码只把该列用于等值映射（`AlgorithmTypeDb` 无 `ORDER BY`/范围比较用法），不受影响；
但如果你的自建视图或报表里有 `ORDER BY algorithm` / `algorithm > 'x'`，需要显式改成
`ORDER BY algorithm::text`。

### 3. 方案 B：重建类型（取值精确等于代码，需排他锁）

`ALTER COLUMN ... TYPE` 会重写 `biz_tags` 并对该表持有 `ACCESS EXCLUSIVE` 锁（读写全部阻塞）。
`biz_tags` 通常是小表（默认 `max_biz_tags = 1000`），耗时以毫秒计，但**必须安排在维护窗口**，
并保留 `lock_timeout` 以免锁等待队列堆积把服务拖死。

```sql
BEGIN;

-- 锁等待超时就快速失败，不要排队阻塞业务
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '60s';

-- 3.1 目标类型，取值严格等于 AlgorithmTypeDb
CREATE TYPE nebula_id.algorithm_type_new AS ENUM ('segment', 'snowflake', 'uuid_v8');

-- 3.2 先摘掉依赖旧类型的 DEFAULT 表达式，避免自动 cast 失败
ALTER TABLE nebula_id.biz_tags ALTER COLUMN algorithm DROP DEFAULT;

-- 3.3 迁移列取值：旧值映射到 uuid_v8，其余原样保留
ALTER TABLE nebula_id.biz_tags
    ALTER COLUMN algorithm TYPE nebula_id.algorithm_type_new
    USING (
        CASE
            WHEN algorithm::text IN ('uuid_v7', 'uuid_v4') THEN 'uuid_v8'
            ELSE algorithm::text
        END::nebula_id.algorithm_type_new
    );

-- 3.4 恢复与 init.sql 一致的默认值
ALTER TABLE nebula_id.biz_tags ALTER COLUMN algorithm SET DEFAULT 'segment';

COMMIT;
```

```sql
-- 3.5 锁外清理旧类型（若报错说明还有 0.3 未发现的依赖对象，此时保留旧类型并上报，
--     禁止改用 DROP ... CASCADE 静默删掉别人的视图）
--     执行前再确认一次已无列引用 algorithm_type：
SELECT table_schema, table_name, column_name
FROM information_schema.columns
WHERE udt_name = 'algorithm_type';
-- 期望：0 行

DROP TYPE nebula_id.algorithm_type;
ALTER TYPE nebula_id.algorithm_type_new RENAME TO algorithm_type;
```

方案 B 的停机说明：3.1~3.4 在一个事务里，表锁只覆盖这段执行时间；但**期间旧 binary 与新 binary 都不能工作**，
所以必须在应用已停机或已完成升级后执行。执行完 3.5 之前，`nebula_id.algorithm_type` 已不被任何列引用，
旧 binary 的读路径不受影响（它读的是列，不是类型名）。

### 补充步骤：收紧 `algorithm` 列的 NOT NULL（方案 A / 方案 B 之后都要执行）

SeaORM 实体 `Model.algorithm` 是**非 Option** 的 `AlgorithmTypeDb`
（`src/core/database/biz_tag_entity.rs:30`），任何 `algorithm IS NULL` 的行在 `find` 时都会
解码失败并抛错——与残留 `uuid_v7` 行是同一类故障。`scripts/init.sql` 已把该列声明为
`NOT NULL DEFAULT 'segment'`，但它用的是 `CREATE TABLE IF NOT EXISTS`，**对存量库完全不生效**，
所以约束必须由本步骤手工收紧。

```sql
-- S.1 先看有多少 NULL（为 0 则 S.2 可跳过，直接执行 S.3）
SELECT count(*) AS null_rows FROM nebula_id.biz_tags WHERE algorithm IS NULL;

-- S.2 先回填：NULL -> 'segment'（与列 DEFAULT 一致，不引入新语义）
--     只锁受影响行，不重写整表；不碰 updated_at，理由同 2.2。
UPDATE nebula_id.biz_tags
SET algorithm = 'segment'
WHERE algorithm IS NULL;

-- S.3 再收紧约束（回填未清完时本语句必定失败，见下方"执行顺序"）
ALTER TABLE nebula_id.biz_tags ALTER COLUMN algorithm SET NOT NULL;

-- S.4 验证
SELECT is_nullable, column_default
FROM information_schema.columns
WHERE table_schema = 'nebula_id' AND table_name = 'biz_tags' AND column_name = 'algorithm';
-- 期望：is_nullable = 'NO'；column_default 含 `'segment'`
--       （渲染形式随 PG 版本 / search_path 变化，可能是 `'segment'::algorithm_type`
--         或带 schema 限定的 `'segment'::nebula_id.algorithm_type`，两者都正确）
```

**执行顺序为什么必须是「先回填、后收紧」**：`SET NOT NULL` 在提交前要扫描全表确认现存行没有 NULL，
只要还有一行 NULL，整条语句就以 `ERROR: column "algorithm" contains null values` 失败并回滚，
不会留下"半收紧"的状态。反过来先做 DDL 再回填是没有意义的（回填的目标行根本不存在）。
`UPDATE ... WHERE algorithm IS NULL` 放在前面还能把 S.3 需要校验的数据量降到最低。

回滚：`ALTER TABLE nebula_id.biz_tags ALTER COLUMN algorithm DROP NOT NULL;`（该列允许 NULL 的历史
状态可随时退回，且不影响数据）。

### 4. 为什么不是一句 `RENAME VALUE` 就够

```sql
ALTER TYPE nebula_id.algorithm_type RENAME VALUE 'uuid_v7' TO 'uuid_v8';
```

该语句本身合法（PG 10+，可事务回滚），但两处不成立：

1. `uuid_v4` 也要变成 `uuid_v8`，而 enum 取值名唯一。PG 16 实测（在已执行过 2.1 的库上）：

   ```text
   ERROR:  enum label "uuid_v8" already exists
   ```

   必须先 `UPDATE` 掉所有 `uuid_v4` 行，且 `uuid_v7 -> uuid_v8` 改名会
   同时改写已有行的显示值，语义上等于把两批数据混在一起，不如显式 `UPDATE` 可控。
2. 改完仍然删不掉 `uuid_v4`（无 `DROP VALUE`），取值集合与代码不一致的问题没解决。

因此：`RENAME VALUE` 只适合作为方案 A 中 `ADD VALUE + UPDATE` 的等价替换写法，不能替代方案 B。

### 5. 迁移后验证（可验证性说明）

```sql
-- 5.1 取值集合必须精确等于 segment | snowflake | uuid_v8
SELECT array_agg(enumlabel ORDER BY enumsortorder) AS labels
FROM pg_type t JOIN pg_enum e ON e.enumtypid = t.oid
WHERE t.typname = 'algorithm_type';
-- 方案 B 期望：{segment,snowflake,uuid_v8}
-- 方案 A 期望：{segment,snowflake,uuid_v7,uuid_v4,uuid_v8}（顺序以 enumsortorder 为准，uuid_v8 在最后）

-- 5.2 不存在任何非法/旧值行（返回 0 即通过）
--     必须显式判 NULL：init.sql 里 algorithm 列可空，而 `NULL NOT IN (...)` 结果是 NULL 而不是真，
--     只用 NOT IN 会漏掉 NULL 行造成假通过。
SELECT count(*) FILTER (WHERE algorithm IS NULL)                       AS null_rows,
       count(*) FILTER (WHERE algorithm::text NOT IN
                                    ('segment', 'snowflake', 'uuid_v8')) AS illegal_label_rows,
       count(*) FILTER (WHERE algorithm::text IN ('uuid_v7', 'uuid_v4'))  AS legacy_rows
FROM nebula_id.biz_tags;
-- 期望：illegal_label_rows = 0 且 legacy_rows = 0
-- null_rows 期望 0：`AlgorithmTypeDb` 在实体里是非 Option 字段，NULL 行读出来会报
--   转换失败；若确实为 0 之外的值，属于独立的历史数据问题，需要按快照补齐后再迁移。

-- 5.3 列类型与默认值已指向正确类型
SELECT a.attname AS column_name,
       format_type(a.atttypid, a.atttypmod) AS column_type,
       pg_get_expr(d.adbin, d.adrelid) AS default_expr,
       c.relname AS table_name,
       n.nspname AS schema_name
FROM pg_attribute a
JOIN pg_class c ON c.oid = a.attrelid
JOIN pg_namespace n ON n.oid = c.relnamespace
LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum
WHERE a.attrelid = 'nebula_id.biz_tags'::regclass
  AND a.attname = 'algorithm';
-- 期望 column_type = nebula_id.algorithm_type（或 public.algorithm_type，取决于 0.1 的结果）
--      default_expr = 'segment'::nebula_id.algorithm_type（PG 16 实测带 schema 限定名）

-- 5.4 类型名没被方案 B 的中间名占用（期望 0 行）
SELECT n.nspname, t.typname
FROM pg_type t JOIN pg_namespace n ON n.oid = t.typnamespace
WHERE t.typname LIKE 'algorithm_type%new%';
```

写入路径闭环检查（在事务里试写再回滚，不留副作用；注意不要在执行 2.1 的同一事务里跑，
`ADD VALUE` 的新取值要到该事务提交后才可用）：

```sql
BEGIN;

-- 期望：UPDATE 0（没有匹配行也没关系，重点是语句不报 invalid input value for enum algorithm_type）
UPDATE nebula_id.biz_tags
SET algorithm = 'uuid_v8'
WHERE name = '__migration_probe__';

-- 期望：不报错，返回 0 行
SELECT id, algorithm::text FROM nebula_id.biz_tags WHERE algorithm::text = 'uuid_v8';

ROLLBACK;
```

> `POST /api/v1/config/algorithm` 改的是热加载的**默认算法**（内存态配置），不会写 `biz_tags.algorithm`，
> 所以不能用它验证本次 ENUM 迁移。真正会把 `biz_tags.algorithm` 反序列化成 `AlgorithmTypeDb` 的是
> `GET /api/v1/biz-tags` / `GET /api/v1/biz-tags/{id}`（路由见 `src/server/router.rs:153-162`，
> 前缀 `/api/v1` 见 `src/server/api_version.rs:32`）。迁移后调用一次
> `GET /api/v1/biz-tags`，确认不出现 enum 转换失败类错误即可。

### 6. 回滚

`uuid_v7` 与 `uuid_v4` 到 `uuid_v8` 是**多对一**映射，DB 内部无法反推原始标签。
唯一的无损降级依据是第 1 节建立的文本快照表 `nebula_id.biz_tags_bak_pre_uuid_v8`
（它的 `algorithm_label` 是 `text`，因此不受类型变更影响）—— 没有它就只能整体 `pg_restore`。

**方案 A 回滚**（旧类型仍在，随时可执行，无需 DDL）：

```sql
UPDATE nebula_id.biz_tags t
SET algorithm = b.algorithm_label::nebula_id.algorithm_type
FROM nebula_id.biz_tags_bak_pre_uuid_v8 b
WHERE t.id = b.id
  AND b.algorithm_label IN ('uuid_v7', 'uuid_v4');
```

**方案 B 回滚**（类型已被重建为三值，必须先把四值类型装回去）：

```sql
BEGIN;

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '60s';

-- 6.1 重建原始的四值类型定义
CREATE TYPE nebula_id.algorithm_type_old AS ENUM ('segment', 'snowflake', 'uuid_v7', 'uuid_v4');

-- 6.2 列退回旧类型；uuid_v8 先给占位标签 uuid_v7（下一步用快照修正）
ALTER TABLE nebula_id.biz_tags ALTER COLUMN algorithm DROP DEFAULT;
ALTER TABLE nebula_id.biz_tags
    ALTER COLUMN algorithm TYPE nebula_id.algorithm_type_old
    USING (
        CASE WHEN algorithm::text = 'uuid_v8' THEN 'uuid_v7'
             ELSE algorithm::text
        END::nebula_id.algorithm_type_old
    );
ALTER TABLE nebula_id.biz_tags ALTER COLUMN algorithm SET DEFAULT 'segment';

DROP TYPE nebula_id.algorithm_type;
ALTER TYPE nebula_id.algorithm_type_old RENAME TO algorithm_type;

COMMIT;

-- 6.3 用快照贴回真实标签（不要在 6.2 的 USING 里做关联子查询，
--     ALTER COLUMN TYPE 表达式逐行求值时引用同表不可靠）
UPDATE nebula_id.biz_tags t
SET algorithm = b.algorithm_label::nebula_id.algorithm_type
FROM nebula_id.biz_tags_bak_pre_uuid_v8 b
WHERE t.id = b.id
  AND b.algorithm_label IN ('uuid_v7', 'uuid_v4');
```

**回滚后必须验证**（与 5.2/5.3 同理，但期望值反过来）：

```sql
SELECT algorithm::text AS algorithm_value, count(*) AS row_count
FROM nebula_id.biz_tags
GROUP BY algorithm::text
ORDER BY algorithm::text;
-- 方案 A/B 回滚成功时期望仍能看到 uuid_v7 / uuid_v4 的行（若快照里本来就有）

SELECT count(*) AS mismatch
FROM nebula_id.biz_tags t
JOIN nebula_id.biz_tags_bak_pre_uuid_v8 b ON b.id = t.id
WHERE t.algorithm::text <> b.algorithm_label;
-- 期望：0（所有行与快照一致，回滚无遗漏）
```

- 兜底：`pg_restore --clean --if-exists -n nebula_id -d idgen "nebula_id_pre_uuid_v8_<ts>.dump"`。
- 只有确认不再需要回滚时才删除快照表：`DROP TABLE nebula_id.biz_tags_bak_pre_uuid_v8;`。
  在方案 B 的 3.5 之前它都不应被删除 —— 但也不要长期遗留，避免进入下次备份。

### 7. 已知遗留（不在本 SQL 范围内）

同一重命名工作线的其余残留，**已在本轮 converge 阶段闭环**，记录如下以免按旧描述回改：

- `config/config.toml:49`、`config/config_test.toml:49`：段键名已改为 `[algorithm.uuid_v8]`。
  旧写法不只是"该段不生效"——`AlgorithmConfig::uuid_v8`（`src/core/config/algorithm.rs:109`）
  没有 `#[serde(default)]`，缺字段会让**整个配置文件**反序列化失败，`src/main.rs:532-537` 随后
  退回 `Config::default()`，等于整份 config.toml 被丢弃。
- `docs/API_REFERENCE.md`、`docs/USER_GUIDE.md`、`docs/FAQ.md`：示例已改用 `AlgorithmType::UuidV8`
  / `AlgorithmBuilder` / `Id::from_uuid_v8`；`uuid_v7` / `uuid_v4` 仅作为 `AlgorithmType::from_str`
  的输入别名保留说明（`src/core/types/id.rs:197-198`）。
- `src/core/database/repository.rs`（`integration-tests` 模块 `setup_test_db`）：三个枚举
  （`algorithm_type` / `id_format` / `workspace_status`）已改为在 `nebula_id` schema 创建，
  与建表语句的 `"nebula_id"."xxx"` 引用及 `scripts/init.sql` 一致。
- `scripts/init.sql`：`biz_tags.algorithm` 已加 `NOT NULL`，存量库按上面「补充步骤」收紧。

**本工作线尚未处理**（记录供后续任务）：

- `docs/ARCHITECTURE.md:17`：架构图节点仍写 `UUID v7/v4`。
- `scripts/init.sql` 的 `biz_tags`：`format` / `prefix` / `base_step` / `max_step` / `datacenter_ids`
  对应的实体字段同样是非 Option（`src/core/database/biz_tag_entity.rs:31-36`），但列仍可空，
  与 `algorithm` 是同一类不一致；本轮按任务范围只收紧了 `algorithm`。
