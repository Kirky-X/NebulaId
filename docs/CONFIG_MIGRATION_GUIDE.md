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
