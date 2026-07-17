# 测试配置文件说明

本目录包含 Nebula ID 测试脚本的配置文件模板。

## 文件说明

| 文件 | 说明 |
|------|------|
| `test_config.toml` | 测试脚本默认配置文件 |
| `config.toml` | 主服务配置文件（生产） |
| `config_test.toml` | 服务测试配置（SQLite 内存数据库） |

> 注：无 etcd 环境请通过 `config.toml` 中 `[etcd] endpoints = []` 控制，不再单独维护 `config_no_etcd.toml`。

## 使用方法

### 1. 默认配置（本地开发）

```bash
# 直接运行测试，使用默认配置
./tests/db_concurrency_test.sh
```

### 2. 自定义配置文件

```bash
# 创建自定义配置
cp config/test_config.toml my-test-config.toml
# 编辑配置
vim my-test-config.toml

# 运行测试
export TEST_CONFIG_FILE="my-test-config.toml"
./tests/db_concurrency_test.sh
```

### 3. 环境变量覆盖

```bash
# 使用环境变量覆盖配置
export NEBULA_API_BASE="http://your-server:8080"
export TEST_AUTH_HEADER="Authorization: Basic your-encoded-credentials"

./tests/db_concurrency_test.sh
```

## 配置项说明

### [api] 部分

| 配置项 | 环境变量 | 默认值 | 说明 |
|--------|----------|--------|------|
| `api_base` | `NEBULA_API_BASE` | `http://localhost:8080` | API 服务器地址 |

### [auth] 部分

| 配置项 | 环境变量 | 默认值 | 说明 |
|--------|----------|--------|------|
| `auth_header` | `TEST_AUTH_HEADER` | 自动生成 | 认证请求头 |

### [workspace] 部分

| 配置项 | 环境变量 | 默认值 | 说明 |
|--------|----------|--------|------|
| `default_workspace` | `TEST_WORKSPACE` | `test-workspace` | 默认测试工作空间 |

### [test] 部分

| 配置项 | 环境变量 | 默认值 | 说明 |
|--------|----------|--------|------|
| `cleanup_enabled` | `CLEANUP_ENABLED` | `true` | 测试后是否清理数据 |

## 认证头格式

### ApiKey 格式
```
Authorization: ApiKey key_id:key_secret
```

### Basic 格式
```
Authorization: Basic base64(key_id:key_secret)
```

### Salted Basic 格式
```
Authorization: Basic base64(sha256(salt:key_id:key_secret))
```

## 示例配置

### 本地开发配置
```toml
[api]
api_base = "http://localhost:8080"

[auth]
auth_header = ""

[workspace]
default_workspace = "test-workspace"

[test]
cleanup_enabled = true
```

### 远程服务器配置
```toml
[api]
api_base = "https://nebulaid.example.com"

[auth]
auth_header = "Authorization: ApiKey your-key-id:your-key-secret"

[workspace]
default_workspace = "production-test"

[test]
cleanup_enabled = false
```

## 故障排除

### 配置文件未加载

确保配置文件路径正确：
```bash
# 检查配置文件是否存在
ls -la config/test_config.toml

# 使用环境变量指定路径
export TEST_CONFIG_FILE="/full/path/to/config.toml"
```

### 认证失败

1. 检查认证头格式是否正确
2. 确认凭据未过期
3. 尝试使用环境变量覆盖：
```bash
export TEST_AUTH_HEADER="Authorization: ApiKey new-key-id:new-key-secret"
```

### API 地址错误

```bash
# 验证服务器是否可达
curl -v http://localhost:8080/health

# 使用正确的地址
export NEBULA_API_BASE="http://correct-address:8080"
```
