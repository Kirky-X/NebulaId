# 🔧 Nebula ID Config Migration Guide

## 📋 Table of Contents

- [Security hardening update (v0.1.2)](#security-hardening-update-v012)
- [algorithm_type ENUM migration (uuid_v7 / uuid_v4 -> uuid_v8)](#algorithm_type-enum-migration-uuid_v7--uuid_v4---uuid_v8)
- [Key rotation grace period and config fail-fast (unreleased)](#key-rotation-grace-period-and-config-fail-fast-unreleased)
- [Audit fix round migration (fix-audit-defects-r1)](#audit-fix-round-migration-fix-audit-defects-r1)
- [Config reference (all options and validation rules)](#config-reference-all-options-and-validation-rules)

---

## Security hardening update (v0.1.2)

### Important changes

Starting from v0.1.2, Nebula ID enforces a stricter security policy: all sensitive values in config files must be provided through environment variables.

### Database password configuration

#### The old way (deprecated)
```toml
[database]
password = "idgen123"
```

#### The new way (mandatory)

**Option 1: environment variable expansion**
```toml
[database]
password = "${NEBULA_DATABASE_PASSWORD}"
```

Set the environment variable:
```bash
export NEBULA_DATABASE_PASSWORD="your_secure_password_here"
```

**Option 2: use the full database URL**
```toml
[database]
url = "${DATABASE_URL}"
```

Set the environment variable:
```bash
export DATABASE_URL="postgresql://idgen:your_password@localhost:5432/idgen"
```

### Quick start

#### Development environment

1. Copy the example environment file:
```bash
cp docker/.env.example .env
```

2. Generate a strong password:
```bash
# Linux/macOS
openssl rand -base64 32

# Windows PowerShell
-join ((48..57) + (65..90) + (97..122) | Get-Random -Count 32 | ForEach-Object {[char]$_})
```

3. Edit the `.env` file and set the passwords:
```bash
POSTGRES_PASSWORD=your_generated_password
NEBULA_DATABASE_PASSWORD=your_generated_password
DATABASE_URL=postgresql://idgen:your_generated_password@localhost:5432/idgen
```

4. Start the services:
```bash
docker-compose up -d
```

#### Production environment

**Variables that must be set:**

```bash
# Database password (must be strong)
export NEBULA_DATABASE_PASSWORD="$(openssl rand -base64 32)"

# Or use the full URL
export DATABASE_URL="postgresql://idgen:$(openssl rand -base64 32)@db-host:5432/idgen"

# API key salt (must be set)
export NEBULA_API_KEY_SALT="$(openssl rand -hex 32)"
```

**Docker Compose example:**

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

### Config file locations

- **Main config file**: `config/config.toml`
- **Test config file**: `config/test_config.toml` (a template for the test scripts; see `config/README_TEST_CONFIG.md`; the no-etcd environment is controlled via `config.toml`'s `[etcd] endpoints = []`, and `config_no_etcd.toml` is no longer maintained separately)
- **Benchmark/load-test configs**: `config/bench_a.toml` / `bench_b.toml` / `bench_e.toml` (HTTP end-to-end load tests; see [PERFORMANCE.md](PERFORMANCE.md))

### Environment variable precedence

Environment variable configuration overrides in this order:

1. OS environment variables (highest priority)
2. Variables in the `.env` file
3. Defaults in the config file (lowest priority)

### Security advice

1. **Never** hardcode passwords in config files
2. **Never** commit `.env` files containing real passwords to version control
3. **Always** use strong passwords (at least 16 characters, mixing upper/lowercase letters, digits, and special characters)
4. **Rotate** passwords and keys periodically
5. **Limit** who knows the passwords

### Troubleshooting

#### Error: "NEBULA_DATABASE_PASSWORD environment variable must be set"

**Cause**: the database password environment variable is not set in production

**Solution**:
```bash
export NEBULA_DATABASE_PASSWORD="your_password"
# Or
export DATABASE_URL="postgresql://..."
```

#### Warning: "Weak or empty database password detected"

**Cause**: a weak or empty password was detected

**Solution**: switch to a strong password immediately

### Backward compatibility

This change is **not backward compatible**. After upgrading to v0.1.2+ you must:

1. Update all config files to use environment variable references
2. Set the required environment variables
3. Restart all service instances

### Need help?

- Read the full docs: [USER_GUIDE.md](USER_GUIDE.md)
- File issues: https://github.com/Kirky-X/NebulaId/issues
- Community: GitHub Discussions

## algorithm_type ENUM migration (uuid_v7 / uuid_v4 -> uuid_v8)

### Background

The code-side `AlgorithmTypeDb` (`src/core/database/biz_tag_entity.rs`) currently carries five values,
with a bidirectional one-to-one mapping to `AlgorithmType` (`src/core/types/id.rs`) via
`From<AlgorithmType> for AlgorithmTypeDb`:

| DB ENUM label | SeaORM variant | `AlgorithmType` |
|---------------|----------------|-----------------|
| `segment`     | `AlgorithmTypeDb::Segment`   | `AlgorithmType::Segment`   |
| `snowflake`   | `AlgorithmTypeDb::Snowflake` | `AlgorithmType::Snowflake` |
| `uuid_v8`     | `AlgorithmTypeDb::UuidV8`    | `AlgorithmType::UuidV8`    |
| `uuid_v4`     | `AlgorithmTypeDb::UuidV4`    | `AlgorithmType::UuidV4`    |
| `uuid_v7`     | `AlgorithmTypeDb::UuidV7`    | `AlgorithmType::UuidV7`    |

After add-uuid-v4-v7 (2026-10), `uuid_v7` / `uuid_v4` reversed from "input aliases of
`AlgorithmType::from_str` (not DB values)" into **legitimate DB values** (the enum has been widened; existing
databases are backfilled idempotently by `run_migrations`' `ADD VALUE`).

- New databases: `scripts/init.sql` already creates the correct values; no migration needed.
- Existing databases: the migration in this section is mandatory. Note that `init.sql`'s
  `DO ... WHEN duplicate_object THEN null` block **silently skips** existing types — re-running
  `init.sql` will not fix the old database.

Affected objects (verified via `DeriveActiveEnum`'s `enum_name = "algorithm_type"` and the table/column declarations):

| Type | Object |
|------|--------|
| ENUM type | `algorithm_type` (`scripts/init.sql` creates it unqualified after `SET search_path TO nebula_id, public`, so it lands in the `nebula_id` schema) |
| Columns using it | `nebula_id.biz_tags.algorithm` (`scripts/init.sql` now reads `algorithm algorithm_type NOT NULL DEFAULT 'segment'`; existing databases still have a nullable column — backfill per the "supplementary steps" below, then tighten) |

`id_generation_logs.algorithm` is a plain `VARCHAR(50)` text column, unrelated to this ENUM — no DDL change needed.
This is the only reference to the type in the whole database; step 1 below confirms it empirically.

### 0. Preflight checks (read-only; run this first)

```sql
-- 0.1 Which schema the type actually lives in (decides the qualified names used below)
SELECT n.nspname AS type_schema, t.typname, t.oid AS type_oid
FROM pg_type t
JOIN pg_namespace n ON n.oid = t.typnamespace
WHERE t.typname = 'algorithm_type' AND t.typtype = 'e';

-- 0.2 Current value list (an existing database should show uuid_v7 / uuid_v4)
--     Note the column name is enumsortorder (called enumsort before PG 9.4; it no longer exists)
SELECT e.enumsortorder, e.enumlabel
FROM pg_type t
JOIN pg_enum e ON e.enumtypid = t.oid
WHERE t.typname = 'algorithm_type'
ORDER BY e.enumsortorder;

-- 0.3 All columns depending on the type (expect only biz_tags.algorithm, one row)
SELECT table_schema, table_name, column_name, data_type, is_nullable, column_default
FROM information_schema.columns
WHERE udt_name = 'algorithm_type'
ORDER BY table_schema, table_name, column_name;

-- 0.4 Row counts needing backfill
SELECT algorithm::text AS algorithm_value, count(*) AS row_count
FROM nebula_id.biz_tags
GROUP BY algorithm::text
ORDER BY algorithm::text;

SELECT count(*) AS legacy_rows
FROM nebula_id.biz_tags
WHERE algorithm::text IN ('uuid_v7', 'uuid_v4');

-- 0.5 All dependent objects of the type (views, functions, snapshot tables, etc. all block 3.5's DROP TYPE)
--     On a healthy PG 12+ database this returns 3 rows, all expected:
--       pg_class  / nebula_id.biz_tags / dependent_column=algorithm  (the column itself)
--       pg_attrdef / <oid>            (the `DEFAULT 'segment'` expression; Plan B drops it with DROP DEFAULT)
--       pg_type   / <oid>  deptype='i' (the enum's auto-generated array type _algorithm_type, an internal dependency)
--     Any row beyond these three categories = a real dependency remains (views / functions / other tables); stop
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

Stop conditions: if 0.3 returns more than one row, or 0.5 shows dependent objects other than
`biz_tags.algorithm`, **do not** proceed — refactor all reference points first; otherwise skip Plan B and do Plan A only.

### 1. Backup

```bash
# Whole-database custom format (pg_restore can pick objects at fine granularity)
pg_dump -U idgen -d idgen -n nebula_id -Fc \
  -f "nebula_id_pre_uuid_v8_$(date +%Y%m%d_%H%M%S).dump"
```

```sql
-- Single-table text snapshot: only the columns needed for locating rows, with algorithm stored as text.
-- text is mandatory — if the snapshot column still had the algorithm_type type, the snapshot table itself would
-- depend on the old type, breaking Plan B's 3.5 `DROP TYPE`.
CREATE TABLE nebula_id.biz_tags_bak_pre_uuid_v8 AS
SELECT id, name, algorithm::text AS algorithm_label
FROM nebula_id.biz_tags;

SELECT algorithm_label, count(*) AS row_count
FROM nebula_id.biz_tags_bak_pre_uuid_v8
GROUP BY algorithm_label
ORDER BY algorithm_label;
```

Permission requirements: the executor must own `algorithm_type` (or be a member of its owning role) and own `biz_tags`.
In Postgres the type's owner is the user `scripts/init.sql` connects as (default `idgen`).

### 2. Plan A: online two-phase (recommended; no downtime; the type keeps 2 orphan values)

Must be executed **in order, as two separate statements**. `ALTER TYPE ... ADD VALUE`'s new value is unusable until the
transaction that added it commits, so 2.1 and 2.2 must not share one `BEGIN ... COMMIT`. Measured on PG 16, combining them fails and rolls back entirely:

```text
ERROR:  unsafe use of new value "uuid_v8" of enum type nebula_id.algorithm_type
HINT:  New enum values must be committed before they can be used.
```

```sql
-- 2.1 Add the new value first (standalone statement, auto-commit; IF NOT EXISTS needs PG 9.6+)
ALTER TYPE nebula_id.algorithm_type ADD VALUE IF NOT EXISTS 'uuid_v8';
```

```sql
-- 2.2 Backfill historical values (must finish before the new binary starts, otherwise reading these rows errors)
UPDATE nebula_id.biz_tags
SET algorithm = 'uuid_v8'
WHERE algorithm::text IN ('uuid_v7', 'uuid_v4');
```

Do not sneak `updated_at = CURRENT_TIMESTAMP` into this `UPDATE`: this is a purely mechanical enum-label backfill,
not a business change; touching `updated_at` would make downstream systems doing incremental sync or audit by
update time misread it as a config change.

```sql
-- 2.3 After confirming no residue, deploy / restart the new nebula-id
SELECT count(*) AS legacy_rows
FROM nebula_id.biz_tags
WHERE algorithm::text IN ('uuid_v7', 'uuid_v4');
-- Expected: 0
```

Plan A's result: reading and writing `uuid_v8` works, and the old binary can still parse `uuid_v7` / `uuid_v4` (friendly during rolling upgrades).
The cost is two leftover values `uuid_v7` / `uuid_v4` in `algorithm_type` — Postgres has **no**
`ALTER TYPE ... DROP VALUE`; a single value cannot be deleted. To align the value set exactly with the code, run Plan B in the next maintenance window.

Note that `ADD VALUE` appends the new label at the **end** of the enum, so `algorithm`'s sort key is insertion order, not lexicographic.
This project's code only uses the column for equality mapping (`AlgorithmTypeDb` has no `ORDER BY`/range comparisons), so it is unaffected;
but if your own views or reports use `ORDER BY algorithm` / `algorithm > 'x'`, change them explicitly to
`ORDER BY algorithm::text`.

### 3. Plan B: rebuild the type (values exactly equal to the code; requires an exclusive lock)

`ALTER COLUMN ... TYPE` rewrites `biz_tags` and holds an `ACCESS EXCLUSIVE` lock on the table (all reads and writes blocked).
`biz_tags` is usually small (when a new workspace does not specify otherwise, the application's write path defaults `max_biz_tags = 100`,
see `src/core/database/workspace_repository.rs:81`; the SQL column default of 1000 at `scripts/init.sql:38`
is unreachable for SeaORM's explicit inserts), so the rewrite takes milliseconds, but it **must be scheduled in a maintenance window**
with a `lock_timeout` retained, lest lock-wait queues pile up and drag the service down.

```sql
BEGIN;

-- Fail fast on lock-wait timeout; do not queue and block the business
SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '60s';

-- 3.1 Target type, values strictly equal to AlgorithmTypeDb
CREATE TYPE nebula_id.algorithm_type_new AS ENUM ('segment', 'snowflake', 'uuid_v8');

-- 3.2 Drop the DEFAULT expression that depends on the old type first, avoiding automatic cast failures
ALTER TABLE nebula_id.biz_tags ALTER COLUMN algorithm DROP DEFAULT;

-- 3.3 Migrate column values: old values map to uuid_v8, the rest stay as-is
ALTER TABLE nebula_id.biz_tags
    ALTER COLUMN algorithm TYPE nebula_id.algorithm_type_new
    USING (
        CASE
            WHEN algorithm::text IN ('uuid_v7', 'uuid_v4') THEN 'uuid_v8'
            ELSE algorithm::text
        END::nebula_id.algorithm_type_new
    );

-- 3.4 Restore the init.sql-consistent default
ALTER TABLE nebula_id.biz_tags ALTER COLUMN algorithm SET DEFAULT 'segment';

COMMIT;
```

```sql
-- 3.5 Clean up the old type outside the lock (if this errors, there are still dependencies 0.3 missed; keep the old type and escalate —
--     never switch to DROP ... CASCADE and silently drop someone else's views)
--     Re-confirm no column references algorithm_type before running:
SELECT table_schema, table_name, column_name
FROM information_schema.columns
WHERE udt_name = 'algorithm_type';
-- Expected: 0 rows

DROP TYPE nebula_id.algorithm_type;
ALTER TYPE nebula_id.algorithm_type_new RENAME TO algorithm_type;
```

Plan B downtime note: 3.1~3.4 run in one transaction, so the table lock covers only that window; but **during it neither the old nor the new binary can serve**,
so it must run after the app is stopped or the upgrade is complete. Before 3.5 completes, `nebula_id.algorithm_type` is no longer referenced by any column,
and the old binary's read path is unaffected (it reads columns, not type names).

### Supplementary step: tighten the `algorithm` column's NOT NULL (after Plan A / Plan B alike)

The SeaORM entity `Model.algorithm` is a **non-Option** `AlgorithmTypeDb`
(`src/core/database/biz_tag_entity.rs:19`); any row with `algorithm IS NULL` fails decoding and raises an error at `find` time —
the same class of failure as leftover `uuid_v7` rows. `scripts/init.sql` declares the column
`NOT NULL DEFAULT 'segment'`, but it uses `CREATE TABLE IF NOT EXISTS`, which **has no effect on existing databases**,
so the constraint must be tightened manually here.

```sql
-- S.1 Count the NULLs first (if 0, skip S.2 and run S.3 directly)
SELECT count(*) AS null_rows FROM nebula_id.biz_tags WHERE algorithm IS NULL;

-- S.2 Backfill: NULL -> 'segment' (consistent with the column DEFAULT; no new semantics)
--     Locks only affected rows, no full-table rewrite; does not touch updated_at, same rationale as 2.2.
UPDATE nebula_id.biz_tags
SET algorithm = 'segment'
WHERE algorithm IS NULL;

-- S.3 Then tighten the constraint (this statement must fail if the backfill is incomplete — see "execution order" below)
ALTER TABLE nebula_id.biz_tags ALTER COLUMN algorithm SET NOT NULL;

-- S.4 Verify
SELECT is_nullable, column_default
FROM information_schema.columns
WHERE table_schema = 'nebula_id' AND table_name = 'biz_tags' AND column_name = 'algorithm';
-- Expected: is_nullable = 'NO'; column_default contains `'segment'`
--       (the rendered form varies by PG version / search_path — it may be `'segment'::algorithm_type`
--         or the schema-qualified `'segment'::nebula_id.algorithm_type`; both are correct)
```

**Why the execution order must be "backfill first, tighten second"**: `SET NOT NULL` scans the whole table before committing to confirm no NULL rows exist;
with even one NULL row, the whole statement fails with `ERROR: column "algorithm" contains null values` and rolls back,
leaving no "half-tightened" state. Doing the DDL first and backfilling after is meaningless (the rows to backfill would not exist).
Placing `UPDATE ... WHERE algorithm IS NULL` first also minimizes the data S.3 must validate.

Rollback: `ALTER TABLE nebula_id.biz_tags ALTER COLUMN algorithm DROP NOT NULL;` (the historical state allowing NULL on this
column can be restored at any time without touching data).

### 4. Why a single `RENAME VALUE` is not enough

```sql
ALTER TYPE nebula_id.algorithm_type RENAME VALUE 'uuid_v7' TO 'uuid_v8';
```

The statement itself is legal (PG 10+, transactional), but two things break:

1. `uuid_v4` must also become `uuid_v8`, and enum labels are unique. Measured on PG 16 (on a database that already ran 2.1):

   ```text
   ERROR:  enum label "uuid_v8" already exists
   ```

   All `uuid_v4` rows must be `UPDATE`d first, and renaming `uuid_v7 -> uuid_v8`
   also rewrites the displayed value of existing rows — semantically mixing two batches of data; an explicit `UPDATE` is more controllable.
2. After the rename, `uuid_v4` still cannot be deleted (no `DROP VALUE`), so the value-set/code mismatch is not solved.

Therefore: `RENAME VALUE` only fits as an equivalent alternative to `ADD VALUE + UPDATE` within Plan A; it cannot replace Plan B.

### 5. Post-migration verification (verifiability notes)

```sql
-- 5.1 The value set must exactly equal segment | snowflake | uuid_v8
SELECT array_agg(enumlabel ORDER BY enumsortorder) AS labels
FROM pg_type t JOIN pg_enum e ON e.enumtypid = t.oid
WHERE t.typname = 'algorithm_type';
-- Plan B expects: {segment,snowflake,uuid_v8}
-- Plan A expects: {segment,snowflake,uuid_v7,uuid_v4,uuid_v8} (order per enumsortorder, uuid_v8 last)

-- 5.2 No illegal/legacy value rows remain (returning 0 passes)
--     NULL must be tested explicitly: init.sql's algorithm column is nullable, and `NULL NOT IN (...)` yields NULL, not true,
--     so NOT IN alone would miss NULL rows and falsely pass.
SELECT count(*) FILTER (WHERE algorithm IS NULL)                       AS null_rows,
       count(*) FILTER (WHERE algorithm::text NOT IN
                                    ('segment', 'snowflake', 'uuid_v8')) AS illegal_label_rows,
       count(*) FILTER (WHERE algorithm::text IN ('uuid_v7', 'uuid_v4'))  AS legacy_rows
FROM nebula_id.biz_tags;
-- Expected: illegal_label_rows = 0 and legacy_rows = 0
-- null_rows is expected to be 0: `AlgorithmTypeDb` is a non-Option field in the entity, so NULL rows fail to read;
--   any other value is a separate legacy-data problem — fix it from the snapshot before migrating.

-- 5.3 Column type and default point at the correct type
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
-- Expected column_type = nebula_id.algorithm_type (or public.algorithm_type, depending on 0.1's result)
--      default_expr = 'segment'::nebula_id.algorithm_type (measured on PG 16 with the schema-qualified name)

-- 5.4 The type name is not occupied by Plan B's intermediate name (expect 0 rows)
SELECT n.nspname, t.typname
FROM pg_type t JOIN pg_namespace n ON n.oid = t.typnamespace
WHERE t.typname LIKE 'algorithm_type%new%';
```

Write-path closed-loop check (trial write inside a transaction then roll back, leaving no side effects; do not run it in the same transaction as 2.1 —
`ADD VALUE`'s new value becomes usable only after that transaction commits):

```sql
BEGIN;

-- Expected: UPDATE 0 (no matching rows is fine; the point is that the statement does not raise invalid input value for enum algorithm_type)
UPDATE nebula_id.biz_tags
SET algorithm = 'uuid_v8'
WHERE name = '__migration_probe__';

-- Expected: no error, 0 rows
SELECT id, algorithm::text FROM nebula_id.biz_tags WHERE algorithm::text = 'uuid_v8';

ROLLBACK;
```

> `POST /api/v1/config/algorithm` changes the hot-loaded **default algorithm** (in-memory config) and never writes `biz_tags.algorithm`,
> so it cannot validate this ENUM migration. What actually deserializes `biz_tags.algorithm` into `AlgorithmTypeDb` is
> `GET /api/v1/biz-tags` / `GET /api/v1/biz-tags/{id}` (routes in `src/server/router.rs:385-394`,
> the `/api/v1` prefix in `src/server/api_version.rs:21-23`). After migrating, call
> `GET /api/v1/biz-tags` once and confirm no enum-conversion errors appear.

### 6. Rollback

`uuid_v7` and `uuid_v4` to `uuid_v8` is a **many-to-one** mapping; the original labels cannot be recovered from the DB alone.
The only lossless rollback basis is the text snapshot table `nebula_id.biz_tags_bak_pre_uuid_v8` created in Section 1
(its `algorithm_label` is `text`, so the type change does not affect it) — without it, only a whole `pg_restore` remains.

**Plan A rollback** (the old type still exists; executable at any time, no DDL needed):

```sql
UPDATE nebula_id.biz_tags t
SET algorithm = b.algorithm_label::nebula_id.algorithm_type
FROM nebula_id.biz_tags_bak_pre_uuid_v8 b
WHERE t.id = b.id
  AND b.algorithm_label IN ('uuid_v7', 'uuid_v4');
```

**Plan B rollback** (the type has been rebuilt with three values; the four-value type must be installed back first):

```sql
BEGIN;

SET LOCAL lock_timeout = '5s';
SET LOCAL statement_timeout = '60s';

-- 6.1 Recreate the original four-value type definition
CREATE TYPE nebula_id.algorithm_type_old AS ENUM ('segment', 'snowflake', 'uuid_v7', 'uuid_v4');

-- 6.2 Move the column back to the old type; uuid_v8 gets the placeholder label uuid_v7 (corrected from the snapshot next)
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

-- 6.3 Paste the real labels back from the snapshot (do not put the correlated subquery inside 6.2's USING —
--     referencing the same table while ALTER COLUMN TYPE's expression is evaluated row-by-row is unreliable)
UPDATE nebula_id.biz_tags t
SET algorithm = b.algorithm_label::nebula_id.algorithm_type
FROM nebula_id.biz_tags_bak_pre_uuid_v8 b
WHERE t.id = b.id
  AND b.algorithm_label IN ('uuid_v7', 'uuid_v4');
```

**Verification after rollback is mandatory** (same idea as 5.2/5.3, but with the expectations inverted):

```sql
SELECT algorithm::text AS algorithm_value, count(*) AS row_count
FROM nebula_id.biz_tags
GROUP BY algorithm::text
ORDER BY algorithm::text;
-- After a successful Plan A/B rollback, uuid_v7 / uuid_v4 rows should be visible again (if the snapshot had them)

SELECT count(*) AS mismatch
FROM nebula_id.biz_tags t
JOIN nebula_id.biz_tags_bak_pre_uuid_v8 b ON b.id = t.id
WHERE t.algorithm::text <> b.algorithm_label;
-- Expected: 0 (all rows match the snapshot; no rollback omissions)
```

- Fallback: `pg_restore --clean --if-exists -n nebula_id -d idgen "nebula_id_pre_uuid_v8_<ts>.dump"`.
- Delete the snapshot table only when rollback is confirmed unnecessary: `DROP TABLE nebula_id.biz_tags_bak_pre_uuid_v8;`.
  It must not be deleted before Plan B's 3.5 — but do not leave it around forever either, lest it enter the next backup.

### 7. Known leftovers (outside this SQL's scope)

The remaining items of the same rename workstream were **closed during this round's converge phase**; recorded here so nobody reverts per the old description:

- `config/config.toml:49`, `config/config_test.toml:49`: the section key has become `[algorithm.uuid_v8]`.
  The old spelling was worse than "this section has no effect" — `AlgorithmConfig::uuid_v8` (`src/core/config/algorithm.rs:109`)
  has no `#[serde(default)]`, so the missing field failed the **entire config file**'s deserialization. Since the unreleased version this failure no longer
  falls back to `Config::default()`: `resolve_startup_config` (`src/main.rs:542`) terminates the process with exit code 1
  (see the next section, "new config restrictions").
- `docs/API_REFERENCE.md`, `docs/USER_GUIDE.md`, `docs/FAQ.md`: examples now use `AlgorithmType::UuidV8`
  / `AlgorithmBuilder` / `Id::from_uuid_v8`; `uuid_v7` / `uuid_v4` remain documented only as input aliases of `AlgorithmType::from_str`
  (`src/core/types/id.rs:197-198`).
- `src/core/database/repository.rs` (the `integration-tests` module's `setup_test_db`): the three enums
  (`algorithm_type` / `id_format` / `workspace_status`) are now created in the `nebula_id` schema, consistent with
  the table statements' `"nebula_id"."xxx"` references and `scripts/init.sql`.
- `scripts/init.sql`: `biz_tags.algorithm` now has `NOT NULL`; existing databases tighten it via the "supplementary steps" above.

**Not yet handled in this workstream** (recorded for follow-up tasks):

- `docs/ARCHITECTURE.md:17`: the architecture diagram node still says `UUID v7/v4`.
- In `scripts/init.sql`'s `biz_tags`: the entity fields for `format` / `prefix` / `base_step` / `max_step` / `datacenter_ids`
  are likewise non-Option (`src/core/database/biz_tag_entity.rs:31-36`), but the columns remain nullable —
  the same class of inconsistency as `algorithm`; this round's scope tightened only `algorithm`.

---

## Key rotation grace period and config fail-fast (unreleased)

Corresponds to the specmark change `key-rotation-and-config-failfast`. The three sections are independent: Section 1 is **optional** DDL
(the startup migration already backfills the columns; manual execution is needed only without DDL rights or with an externally managed schema), Section 2 is a
read-only preflight before enabling the grace period, and Section 3 is a pure behavior change (no SQL, but it directly decides whether the service starts after upgrading).

All SQL in this section was actually executed in a `postgres:16-alpine` container (the same image as `docker/docker-compose.yml:8`);
the "measured output" is the real result; the three config-side messages come from the real binary
`target/debug/nebula-id.exe --config <file>`'s stderr.

### 1. The grace period's two columns (auto-backfilled at startup; manual DDL optional)

When `auth.key_rotation_grace_period_seconds` (default `0` = off) is set to `> 0`, rotation keeps accepting the previous
credential for a while, which depends on two columns of `nebula_id.api_keys` (entity definition
`src/core/database/api_key_entity.rs:31-34`):

| Column | Type | Semantics |
|--------|------|-----------|
| `prev_secret_hash` | `VARCHAR(128)` | The previous credential's hash; non-NULL only after "a rotation completed with the grace period enabled" |
| `rotate_expires_at` | `TIMESTAMP WITHOUT TIME ZONE` | The grace period's absolute deadline (UTC, no time zone); NULL = no active window |

- New databases: `scripts/init.sql:88-94` already creates both columns; skip this section.
- Existing databases: **manual DDL is usually unnecessary** — at startup, `run_migrations` (`src/main.rs:601`, called
  unconditionally) executes the idempotent `ALTER TABLE nebula_id.api_keys ADD COLUMN IF NOT EXISTS
  prev_secret_hash ..., ADD COLUMN IF NOT EXISTS rotate_expires_at ...`
  (`src/core/database/connection.rs:310-339`); if that statement fails, startup aborts (exit 1), so any running
  process already has both columns.
- The three situations still requiring this section manually: the app's DB account has **no DDL rights** (startup would fail then; a DBA must add the columns
  beforehand), the schema is managed by an external tool such as Flyway/Liquibase, or you want **zero downtime** by adding the columns before switching versions.
- Expiry judgment is lazy — `validate_api_key` compares `rotate_expires_at` with the current time at each verification,
  so no background cleanup task and no index are needed.

#### 1.1 Preflight (read-only; run this first)

```sql
-- 2 rows = already the new structure, skip this section; 0 rows = migration needed
SELECT column_name, data_type, is_nullable
FROM information_schema.columns
WHERE table_schema = 'nebula_id'
  AND table_name = 'api_keys'
  AND column_name IN ('prev_secret_hash', 'rotate_expires_at')
ORDER BY column_name;
```

#### 1.2 Backup

```bash
pg_dump -U idgen -d idgen -n nebula_id -Fc \
  -f "nebula_id_pre_grace_columns_$(date +%Y%m%d_%H%M%S).dump"
```

#### 1.3 Migration (replayable)

```sql
ALTER TABLE nebula_id.api_keys ADD COLUMN IF NOT EXISTS prev_secret_hash VARCHAR(128);
ALTER TABLE nebula_id.api_keys ADD COLUMN IF NOT EXISTS rotate_expires_at TIMESTAMP WITHOUT TIME ZONE;
```

Measured behavior:

- First run: two `ALTER TABLE` outputs.
- Re-run: each statement first prints `NOTICE:  column "prev_secret_hash" of relation "api_keys" already exists, skipping`
  (likewise for the second column) then `ALTER TABLE`, with exit code still 0 — idempotent, safe to re-run together with
  `scripts/init.sql`.
- `ADD COLUMN IF NOT EXISTS` needs PG 9.6+ (this repo's compose pins 16).
- Both columns are nullable with no default, so PG 11+ does not rewrite table data; but the statements still briefly hold an `ACCESS EXCLUSIVE` lock —
  run them off-peak on large tables, after confirming no long transactions/queries are queued ahead.

#### 1.4 Post-migration verification

Repeat 1.1's query; expect exactly 2 rows (measured output):

| column_name | data_type | is_nullable |
|-------------|-----------|-------------|
| `prev_secret_hash` | `character varying` | `YES` |
| `rotate_expires_at` | `timestamp without time zone` | `YES` |

#### 1.5 Rollback

```sql
ALTER TABLE nebula_id.api_keys DROP COLUMN IF EXISTS rotate_expires_at;
ALTER TABLE nebula_id.api_keys DROP COLUMN IF EXISTS prev_secret_hash;
```

**Destructive**: `DROP COLUMN` also discards any still-active grace window, instantly invalidating previous credentials inside it. Run only after
`key_rotation_grace_period_seconds` is back to `0` and no active window remains.

### 2. Preflight before enabling: the existing admin key count

The unreleased version fixes the root cause of the admin key guard. The old implementation used `list_api_keys(Uuid::nil(), Some(1000), Some(0))`
plus an in-memory scan to judge "does an admin key already exist", but the global admin key's `workspace_id` is `NULL`,
`NULL = nil_uuid` never holds in SQL → the query was always empty → **the guard never really worked**. It now uses
`count_admin_keys()` (`src/core/database/repository.rs:264-268`: the predicate is only
`role = 'admin' AND enabled = true`, with no workspace filter and no 1000-row pagination cap).

Consequence: after upgrading, creating another admin key via `POST /api-keys` will be **rejected for the first time**. Check first:

```sql
-- Exactly the guard's predicate. Measured: 1 enabled admin + 1 disabled admin + 1 enabled user returns 1
SELECT count(*) AS enabled_admins
FROM nebula_id.api_keys
WHERE role = 'admin' AND enabled;
```

- Returns `0`: the first admin key can still be created via `POST /api-keys`.
- Returns `≥ 1`: further creation is rejected with `CoreError::AuthenticationError`, copy
  `An admin API key already exists; creating additional admin keys is forbidden`
  (`locales/en/messages.ftl`; Chinese in `locales/zh/messages.ftl`). **This is the fixed, expected behavior — not a regression.**
- `role` is a `VARCHAR(20)` text column (the historical ENUM was removed per the comment at `scripts/init.sql:85`);
  compare against the string `'admin'` directly.
- Setting an admin key to `enabled = false` removes it from this count, but also strips its authentication eligibility.

**The guard's scope**: startup-bootstrapped keys go through the repository's `repo.create_api_key()`
(`src/main.rs:122`, `:159`, `:203`, `:264`) and **bypass** this guard; only HTTP
`POST /api-keys` (`src/server/handlers/api_key_handlers.rs:61-88`) is intercepted.

### 3. New config restrictions: bad configs and unknown keys always fail startup

No SQL, but a **breaking change**: previously a bad config was logged and discarded, and the process started normally with built-in defaults.

| Scenario | Old behavior | New behavior | Measured stderr fragment |
|----------|--------------|--------------|--------------------------|
| Misspelled section name (e.g. `[algorithm.uuid_v7]`) | Whole section silently ignored, started with defaults | Parse failure, exit code 1 | ``unknown field `uuid_v7`, expected one of `default`, `segment`, `snowflake`, `uuid_v8` `` |
| Unknown key within a section (e.g. `[logging] backtrace`) | Silently ignored | Parse failure, exit code 1 | ``unknown field `backtrace`, expected one of `level`, `format`, `include_location` `` |
| Missing required key | Parse failed but **still degraded** to defaults | Parse failure, exit code 1 | ``missing field `host` `` |
| Violating `Config::validate()` | Logged an error, then degraded | Exit code 1 | `HTTP port must be between 1 and 65535` |
| No `--config` and `config/config.toml` missing | Silently used defaults | Still uses defaults, but emits a `warn` | — |
| `--config` pointing to a nonexistent path | — | Exit code 1 | `Configuration file not found: <path>` |

The full message shape is
`failed to load configuration from '<path>': Invalid configuration value: <reason>`,
raised by `resolve_startup_config` (`src/main.rs:542`).

Preflight before upgrading (no database needed — config loading completes before connecting):

```bash
# Point at your config file; reaching the normal startup flow means the config surface is clean — Ctrl-C to stop
./nebula-id --config /path/to/your/config.toml
```

The criterion is **whether the process exits immediately**: with `--config` pointing at a nonexistent file or any error on the config surface,
it exits with code 1 within seconds (measured `--config <nonexistent path>` → `exit=1`); with a clean config it proceeds past this stage and keeps running
(measured: the factory `config/config.toml` survived until killed by a timeout).

This tightening also cleans up a config-surface distortion (the same dead key in two factory files):

- `[logging] backtrace = false` in `config/config.toml` and `config/config_test.toml` was deleted —
  `LoggingConfig` has no such field and nothing in the repo's `src/` or `docs/` referenced it; it had always been silently ignored.

Known leftovers (not handled this round, recorded lest they be mistaken for new defects):

- `config/test_config.toml` is **not** a server config file but the input file for the `tests/lib.sh` bash test scripts
  (`[api]` / `[auth]` / `[workspace]` / `[test]` / `[concurrency]` / `[performance]`; no
  `[app]`, `[database]`). Loading it with the server loader reports `unknown field \`api\``; it would have failed before the tightening
  too (its `[auth]` lacks the required `enabled`) — not a new behavior.

---

## Audit fix round migration (fix-audit-defects-r1)

The 2026-09-24 audit fix round introduces three changes deployers must know about:

### 1. Ceiling migration (a hard prerequisite for existing deployments)

The segment table gains a `ceiling BIGINT` cap column (NULL = uncapped). **The allocation hot path's SQL references this column,
so existing deployments must run the migration before rolling out the service upgrade**, otherwise segment allocation errors directly (column missing):

```bash
psql -f scripts/migrations/001_add_segments_ceiling.sql
```

The script is idempotent (`ADD COLUMN IF NOT EXISTS` + enum-value existence check) and safe to re-run.
New deployments use the latest `scripts/init.sql` directly; no separate migration needed.

Setting a cap (optional, per workspace/biz_tag/dc row):

```sql
UPDATE nebula_id.nebula_segments SET ceiling = 10000000000
WHERE workspace_id = 'ws1' AND biz_tag = 'order' AND dc_id = 0;
```

Once capped, allocation returns `SegmentExhausted` (the `max_id` field carries the ceiling value).

### 2. Snowflake restart clock guard (on by default)

New config `algorithm.snowflake.state_guard` (default `true`):

- Every 30s at runtime, the last minted millisecond timestamp is written to `./data/snowflake_state_{dc}_{worker}.json`;
- At startup, if the state file's timestamp is later than the current wall clock (a cross-restart clock rollback), the process refuses to start
  (fail-closed, preventing duplicate IDs colliding with history after a restart).

Read-only-filesystem deployments may disable it explicitly (at their own risk):

```toml
[algorithm.snowflake]
state_guard = false
```

### 3. Guess-resistance obfuscation (optional)

New `algorithm.obfuscation.key_hex` (16 hex characters = a u64 deployment key).
When configured, `IdFormat::Obfuscated` rendering goes through a 4-round Feistel bijection (`Id::render`),
so consecutive IDs scatter across the output space, resisting enumeration and scale inference; internal storage is unchanged, reversible with the key.
When unconfigured, the Obfuscated format errors explicitly (no default weak key is generated). All instances of one deployment
must hold the same key:

```toml
[algorithm.obfuscation]
key_hex = "0011223344556677"
```

The `biz_tag.format` DB enum gains `'obfuscated'` in sync (segment 2 of the migration script).

> **Current scope of effect**: the obfuscated rendering entry sits in the core/SDK layer (`Id::render`). The HTTP generate
> endpoint currently does not render responses per `biz_tag.format` (existing behavior; `prefixed` likewise has no effect) —
> after configuring `key_hex`, obfuscated IDs come only from SDK/core-layer calls.

---

## Config reference (all options and validation rules)

`Config` spans the thirteen sections `app`, `database`, `redis`, `etcd`, `auth`, `algorithm`, `monitoring`, `logging`,
`rate_limit`, `audit`, `hot_reload`, `tls`, and `batch_generate`; apart from `[redis]`, `[audit]`,
and `[hot_reload]` — the three sections carrying `serde(default)` and omittable in full — the other ten sections carry **no**
optional semantics at the top level (`src/core/config/app_config.rs:27-57`). Missing any required field fails the **entire** file's parse,
and **unknown keys** are rejected likewise — all 18 config structs carry `deny_unknown_fields`. A parse failure makes the process
exit with code 1 (`resolve_startup_config`) instead of falling back to
`Config::default()`; only when no `--config` was given and `config/config.toml` genuinely does not exist are the built-in
defaults used, with an extra `warn` emitted.

> ⚠️ **Code fact check**: at server startup, `Config::merge()` overwrites the file values of
> `algorithm.segment` / `algorithm.snowflake` /
> `algorithm.uuid_v8` with the env-derived config, and those are always the defaults
> (`src/core/config/app_config.rs:393-395`, called from `src/main.rs:559`).
> After the merge only `algorithm.default` survives; until that merge logic is fixed, adjust the three sub-tables in code.

### Options table

| Option | Type | `Config::default()` | Required in file | Notes |
|--------|------|---------------------|------------------|-------|
| `app.name` | String | `"nebula-id"` | ✅ | Application name |
| `app.host` | String | `"0.0.0.0"` | ✅ | Server bind address |
| `app.http_port` | u16 | `8080` | ✅ | HTTP port, must be > 0 |
| `app.grpc_port` | u16 | `9091` | ✅ | gRPC port, must be > 0 |
| `app.dc_id` | u8 | `0` | ✅ | Datacenter ID, must be ≤ 7 (`datacenter_id_bits = 3`) |
| `app.worker_id` | u8 | `0` | ✅ | Worker ID |
| `app.shutdown_timeout_seconds` | u64 | `30` | ➖ | Graceful shutdown timeout, must be > 0 |
| `database.engine` | String | `"postgresql"` | ✅ | Only `postgresql` / `postgres` actually work; `mysql` / `sqlite` are enum variants only, with no connection implementation |
| `database.host` / `port` / `username` / `password` / `database` | — | `localhost` / `5432` / `idgen` / `$NEBULA_DATABASE_PASSWORD` / `idgen` | ✅ | Connection parameters item by item |
| `database.url` | String | `""` | ➖ | Whole-string alternative to the fields above |
| `database.max_connections` | u32 | `100` | ✅ | Pool size, must be > 0 (`config.toml` defaults 50) |
| `database.min_connections` | u32 | `10` | ✅ | Must be ≤ `max_connections` (`config.toml` defaults 5) |
| `database.acquire_timeout_seconds` | u64 | `30` | ✅ | Must be > 0 |
| `database.idle_timeout_seconds` | u64 | `300` | ✅ | Idle connection timeout |
| `redis` | section | — | ➖ | The whole section may be omitted |
| `redis.url` | String | `$REDIS_URL` or `redis://localhost:6379` | ✅ (required if `[redis]` is written) | Redis connection URL |
| `redis.pool_size` / `key_prefix` / `ttl_seconds` | u32 / String / u64 | `16` / `"nebula:id:"` / `600` | ➖ | Cache tuning |
| `etcd.endpoints` | Vec&lt;String&gt; | `[]` | ✅ | An empty list or whitespace-only items (`ETCD_ENDPOINTS=""`/trailing commas) falls back to `LocalDistributedLock` (single process, warn); `config.toml` defaults `[]` (the default build lacks the etcd feature), compose injects `etcd:2379`; a non-etcd build with explicitly configured endpoints refuses startup (fail-closed) |
| `etcd.connect_timeout_ms` / `watch_timeout_ms` | u64 | `5000` / `5000` | ✅ | etcd timeouts |
| `auth.enabled` | bool | `true` | ✅ | The API key middleware's master switch |
| `auth.cache_ttl_seconds` | u64 | `300` | ✅ | Auth cache TTL |
| `auth.api_keys` | array | `[]` | ➖ | Entry fields: `key_id`, `key_secret`, `workspace`, `role`, `rate_limit`, `name` (all required). At startup **only the first entry is created**; `workspace` must be a UUID string or `global`; `role` creates an admin only when exactly `admin` |
| `auth.api_key_salt` | String | `$NEBULA_API_KEY_SALT` or `""` | ➖ | Key hashing salt |
| `auth.key_rotation_grace_period_seconds` | u64 | `0` (grace period off) | ➖ | Set to `> 0` to keep the previous credential for that many seconds after rotation; beyond 30 days (2592000 seconds) startup validation rejects outright (fail-fast, `src/core/config/app_config.rs:420-427`), not clamped; requires the grace period's two columns in the database, backfilled automatically by the startup migration (manual only when the DB account has no DDL rights — see this doc's "key rotation grace period" section) |
| `algorithm.default` | String | `"segment"` | ✅ | `segment` / `snowflake` / `uuid_v8` (this column states the code's `Config::default()`; the factory `config/config.toml` file says `snowflake`) |
| `algorithm.segment.base_step` / `min_step` / `max_step` / `switch_threshold` | u64 / u64 / u64 / f64 | `1000` / `500` / `100000` / `0.1` | ✅ | Dynamic step (mind the `merge` note above) |
| `algorithm.snowflake.datacenter_id_bits` / `worker_id_bits` / `sequence_bits` / `clock_drift_threshold_ms` | u8 / u8 / u8 / u64 | `3` / `8` / `10` / `1000` | ✅ | Bit layout; the remainder is the timestamp bits |
| `algorithm.uuid_v8.enabled` | bool | `true` | ✅ | The UUID v8 switch |
| `monitoring.metrics_enabled` / `metrics_path` / `tracing_enabled` / `otlp_endpoint` | bool / String / bool / String | `true` / `"/metrics"` / `false` / `""` | ✅ | Only `metrics_enabled`/`metrics_path` take effect (Prometheus); `tracing_enabled`/`otlp_endpoint` are reserved fields, unconsumed by the current implementation |
| `logging.level` / `format` / `include_location` | String / String / bool | `"info"` / `"json"` / `true` | ✅ | `level`: trace…error; `format`: json/pretty |
| `rate_limit.enabled` | bool | `true` | ✅ | The rate-limiting master switch |
| `rate_limit.default_rps` | u32 | `10000` | ✅ | Requests per second; must be > 0 when enabled |
| `rate_limit.burst_size` | u32 | `100` | ✅ | When enabled, must be ≤ 10 × `default_rps` |
| `audit` | section | — | ➖ | The whole section may be omitted |
| `audit.file_logging_enabled` | bool | `true` | ➖ | Production persists audit events to a file by default (JSON Lines, one event per line) |
| `audit.file_logging_path` | String | `"logs/audit.log"` | ➖ | Audit file path |
| `audit.memory_capacity` | usize | `10000` | ➖ | In-memory ring buffer capacity (an event cap; oldest evicted beyond it) |
| `hot_reload` | section | `auto_watch_enabled = false` | ➖ | The whole section may be omitted |
| `tls.enabled` / `cert_path` / `key_path` / `http_enabled` / `grpc_enabled` | bool / String / String / bool / bool | `false` / `""` / `""` / `false` / `false` | ✅ | TLS for HTTP and gRPC |
| `tls.ca_path` | String? | `null` | ➖ | Optional CA |
| `tls.min_tls_version` | String | `"tls13"` | ➖ | `tls12` / `tls13` |
| `tls.alpn_protocols` | Vec&lt;String&gt; | `["h2", "http/1.1"]` | ➖ | ALPN list |
| `batch_generate.max_batch_size` | u32 | `100` | ✅ | Must be within 1..=10000 |

Defaults are `Config::default()`'s values; "required in file" states whether the field carries a serde default. All 18 config structs carry `#[serde(deny_unknown_fields)]`;
unknown keys are exactly as severe as missing required keys: both fail the **entire** file's parse and abort startup,
so misspelled section names can no longer be silently dropped.

### Environment variables

There is no `NEBULA_APP_*` / `NEBULA_AUTH_API_KEY` family of variables. Only two real mechanisms exist:

```bash
# 1. At startup, `Config::load_from_env()` overlays these onto the file config
#    (only items differing from the defaults take effect):
export APP_HOST="0.0.0.0"
export APP_HTTP_PORT="8080"
export APP_GRPC_PORT="9091"
export DC_ID="0"
export WORKER_ID="0"
export DATABASE_URL="postgresql://idgen:pass@localhost:5432/idgen"
export ETCD_ENDPOINTS="http://localhost:2379,http://localhost:22379"
export RUST_LOG="info"
# TLS: booleans accept true/false/1/0; env can only enable (false does not turn off TLS enabled in the file)
export TLS_ENABLED="true"
export TLS_HTTP_ENABLED="true"
export TLS_GRPC_ENABLED="false"
export TLS_CERT_PATH="/app/certs/server.crt"
export TLS_KEY_PATH="/app/certs/server.key"

# 2. Referenced in the file as ${VAR}, expanded before parsing
#    (`Config::expand_env_vars`):
export NEBULA_DATABASE_PASSWORD="..."   # [database].password / url
export NEBULA_API_KEY_SALT="..."        # [auth].api_key_salt fallback
```

### Validation rules

`Config::validate()` runs immediately after a successful parse (`src/core/config/app_config.rs:173-293`); a violation makes
`Config::load_from_file` return `ConfigError::InvalidValue`, the server startup exits with code 1,
and the message names both the file path and the violated item (e.g.
`failed to load configuration from 'config/config.toml': Invalid configuration value:
HTTP port must be between 1 and 65535`):

| Constraint | Source |
|------------|--------|
| `http_port > 0`, `grpc_port > 0`, `shutdown_timeout_seconds > 0` | `Config::validate` |
| `max_connections > 0`, `min_connections <= max_connections`, `acquire_timeout_seconds > 0` | `Config::validate` |
| `rate_limit.enabled` ⇒ `default_rps > 0`, `burst_size > 0`, `burst_size <= 10 × default_rps` | `Config::validate` |
| `algorithm.default ∈ {segment, snowflake, uuid_v8}` | `Config::validate` |
| `segment.min_step <= segment.max_step` and `min_step <= base_step <= max_step` | `Config::validate` |
| `0.0 <= segment.switch_threshold <= 1.0` | `Config::validate` |
| `snowflake.datacenter_id_bits + worker_id_bits + sequence_bits < 64` (defaults ⇒ 43 timestamp bits) | `Config::validate` |
| `snowflake.clock_drift_threshold_ms > 0` | `Config::validate` |
| `app.dc_id < 2^snowflake.datacenter_id_bits`, `app.worker_id < 2^snowflake.worker_id_bits` | `Config::validate` |
| `obfuscation.key_hex` is None or exactly 16 hex characters | `Config::validate` |
| `1 <= batch_generate.max_batch_size <= 10000` | `Config::validate` |

> Full reference: [`config/config.toml`](../../config/config.toml).
