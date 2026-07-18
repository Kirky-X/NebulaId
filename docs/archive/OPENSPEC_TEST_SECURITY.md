# OpenSpec: 测试脚本安全修复规范

**版本**: 1.1.0
**状态**: 规范中
**创建日期**: 2026-01-12
**最后更新**: 2026-01-12
**适用文件**: `tests/*.sh`

---

## 1. 概述

本文档定义了 Nebula ID 测试脚本的安全修复规范，旨在消除硬编码凭据、改进错误处理、增强测试环境的灵活性。

### 1.1 问题摘要

| 严重程度 | 问题数量 | 主要影响 |
|----------|----------|----------|
| 🔴 严重 (Critical) | 3 | 安全漏洞，凭据暴露 |
| 🟠 高风险 (High) | 3 | 生产环境安全风险，测试稳定性差 |
| 🟡 中等 (Medium) | 3 | 测试环境不灵活，维护困难 |
| 🔵 建议 (Low) | 2 | 文档不完整，结果分析困难 |

### 1.2 配置优先级

配置加载遵循以下优先级顺序：

```
优先级 1 (最高): 环境变量
   └── TEST_AUTH_HEADER, NEBULA_API_BASE, TEST_CONFIG_FILE

优先级 2: 配置文件
   └── config/test_config.toml (或自定义路径)

优先级 3 (最低): 硬编码默认值
   └── AUTH_HEADER, API_BASE="http://localhost:8080"
```

### 1.3 配置文件模板

配置文件模板位于 `config/test_config.toml`：

```toml
# ============================================
# Nebula ID 测试配置文件
# ============================================

[api]
# API 基础地址
# 环境变量: NEBULA_API_BASE
api_base = "http://localhost:8080"

[auth]
# 认证头格式
# 环境变量: TEST_AUTH_HEADER
auth_header = ""

[workspace]
# 默认测试工作空间
default_workspace = "test-workspace"

[test]
# 是否清理测试数据
cleanup_enabled = true
```

---

## 2. 安全问题修复 (Critical)

### 2.1 硬编码认证凭据

#### TEST-CRIT-001: lib.sh 第 5 行

**当前问题代码**:
```bash
AUTH_HEADER="Authorization: Basic dGVzdC1rZXktaWQ6dGVzdC1zZWNyZXQ="
```

**问题分析**:
- Base64 编码的凭据 `test-key-id:test-secret` 直接硬编码
- 任何可以访问代码仓库的人都能解码获取凭据
- 若此凭据具有生产环境权限，将造成严重安全风险

**修复规范**:

```bash
# 方案: 多级配置加载机制

# 全局配置变量（默认值，可通过环境变量覆盖）
NEBULA_API_BASE="${NEBULA_API_BASE:-http://localhost:8080}"
NEBULA_AUTH_HEADER="${NEBULA_AUTH_HEADER:-}"

# 配置加载函数
load_test_config() {
    local config_file="${TEST_CONFIG_FILE:-config/test_config.toml}"
    
    # 优先级1: 环境变量
    if [ -n "$NEBULA_AUTH_HEADER" ]; then
        AUTH_HEADER="$NEBULA_AUTH_HEADER"
        return 0
    fi
    
    # 优先级2: 配置文件
    if [ -f "$config_file" ]; then
        local file_header=$(head -c 5 "$config_file" 2>/dev/null || echo "")
        if [ "$file_header" = "#ENV" ]; then
            # ENV 格式配置文件
            source <(grep -E '^[A-Z_]+=' "$config_file" | sed 's/=/="/;s/$/"/' | sed 's/^/export /')
            if [ -n "$NEBULA_AUTH_HEADER" ]; then
                AUTH_HEADER="$NEBULA_AUTH_HEADER"
                return 0
            fi
        else
            # TOML 格式配置文件
            AUTH_HEADER=$(grep -E '^auth_header\s*=' "$config_file" 2>/dev/null | sed 's/.*=\s*//' | tr -d '"' || echo "")
            if [ -n "$AUTH_HEADER" ]; then
                return 0
            fi
        fi
    fi
    
    # 优先级3: 动态生成测试凭据（仅限测试环境）
    local test_key_id="${TEST_API_KEY_ID:-test-key-id}"
    local test_secret="${TEST_API_SECRET:-test-secret}"
    AUTH_HEADER="Authorization: Basic $(echo -n "${test_key_id}:${test_secret}" | base64)"
    
    echo "[WARN] 使用测试凭据，建议通过环境变量或配置文件提供正式凭据"
}

# 在脚本开头调用
load_test_config
```

#### TEST-CRIT-002: db_concurrency_test.sh 第 4 行

**当前问题代码**:
```bash
AUTH_HEADER="Authorization: Basic dGVzdC1rZXktaWQ6dGVzdC1zZWNyZXQ="
```

**修复规范**:

```bash
#!/bin/bash
# 数据库并发测试 - 验证号段分配的并发安全性

# 加载配置
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/lib.sh"

# 确保 lib.sh 的 load_test_config 被调用
if declare -f load_test_config > /dev/null; then
    load_test_config
else
    # 内联配置加载（如果 lib.sh 未提供）
    load_db_test_config() {
        local config_file="${TEST_CONFIG_FILE:-config/test_config.toml}"
        
        # 尝试从环境变量读取
        if [ -n "$TEST_AUTH_HEADER" ]; then
            AUTH_HEADER="$TEST_AUTH_HEADER"
            return 0
        fi
        
        # 尝试从配置文件读取
        if [ -f "$config_file" ]; then
            AUTH_HEADER=$(grep -E '^auth_header\s*=' "$config_file" 2>/dev/null | sed 's/.*=\s*//' | tr -d '"' || echo "")
            if [ -n "$AUTH_HEADER" ]; then
                return 0
            fi
        fi
        
        # 回退到环境变量生成
        local key_id="${TEST_API_KEY_ID:-test-key-id}"
        local secret="${TEST_API_SECRET:-test-secret}"
        AUTH_HEADER="Authorization: Basic $(echo -n "${key_id}:${secret}" | base64)"
    }
    load_db_test_config
fi

echo "=========================================="
echo "数据库并发测试 - 号段分配安全性"
echo "=========================================="
echo "[INFO] API 地址: ${API_BASE:-http://localhost:8080}"
```

#### TEST-CRIT-003: degradation_test.sh 第 4 行

**修复规范**:

```bash
#!/bin/bash
# 降级链测试 - 验证故障降级机制

# 加载配置
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SCRIPT_DIR}/lib.sh"

# 确保配置已加载
if [ -z "$AUTH_HEADER" ]; then
    if declare -f load_test_config > /dev/null; then
        load_test_config
    else
        AUTH_HEADER="Authorization: Basic $(echo -n 'test-key-id:test-secret' | base64)"
    fi
fi
```

---

## 3. 错误处理修复 (High)

### 3.1 set -e 替代方案

#### TEST-HIGH-002: api_test.sh 第 25 行

**当前问题代码**:
```bash
set -e
```

**问题分析**:
- `set -e` 会导致脚本在任何命令返回非零退出码时立即退出
- curl 请求失败、grep 未找到匹配项等都会导致脚本意外终止
- 测试脚本应该有更细粒度的错误处理

**修复规范**:

```bash
# 方案: 细粒度错误处理

# 错误处理函数
handle_error() {
    local exit_code=$?
    local line_number=$1
    local command="${BASH_COMMAND}"
    
    echo "[ERROR] 命令执行失败在第 $line_number 行" >&2
    echo "[ERROR] 命令: $command" >&2
    echo "[ERROR] 退出码: $exit_code" >&2
    
    # 调用清理函数（如果存在）
    if declare -f cleanup_on_exit > /dev/null; then
        cleanup_on_exit
    fi
    
    exit $exit_code
}

# 关键命令后检查结果
check_curl_result() {
    local result=$1
    local operation=$2
    
    if [ $? -ne 0 ]; then
        echo "[ERROR] $operation 失败" >&2
        return 1
    fi
    
    # 检查 HTTP 状态码
    local http_code=$(echo "$result" | tail -n1)
    local body=$(echo "$result" | sed '$d')
    
    if [ "$http_code" != "200" ] && [ "$http_code" != "201" ]; then
        echo "[ERROR] $operation 返回 HTTP $http_code" >&2
        echo "[ERROR] 响应: $body" >&2
        return 1
    fi
    
    return 0
}

# 替代 set -e 的方式
set +e  # 禁用立即退出

# 使用 trap 捕获未处理的错误
trap 'handle_error $LINENO' ERR

# 更安全的 curl 调用模式
safe_curl() {
    local method="$1"
    local url="$2"
    local headers="${3:-}"
    local data="${4:-}"
    
    if [ -n "$data" ]; then
        curl -s -w "\n%{http_code}" -X "$method" "$url" \
            -H "Content-Type: application/json" \
            -H "$headers" \
            -d "$data"
    else
        curl -s -w "\n%{http_code}" -X "$method" "$url" \
            -H "$headers"
    fi
}

# 示例使用
response=$(safe_curl "POST" "$BASE_URL/api/v1/generate" \
    "$AUTH_HEADER" \
    '{"workspace":"test","group":"default","biz_tag":"test"}')

if ! check_curl_result "$response" "生成ID"; then
    echo "[WARN] 跳过此测试项"
    continue
fi
```

### 3.2 distributed_test.sh 错误处理

**修复规范**:

```bash
#!/bin/bash

# 分布式一致性测试和雪花算法验证脚本

# 禁用 set -e，改用细粒度错误处理
set +e

# 错误处理
error_handler() {
    local exit_code=$?
    local line=$1
    echo "[ERROR] 脚本在第 $line 行异常退出，退出码: $exit_code" >&2
    
    # 清理临时文件
    if [ -n "$TMPDIR" ] && [ -d "$TMPDIR" ]; then
        rm -rf "$TMPDIR"
    fi
    
    exit $exit_code
}

trap 'error_handler $LINENO' ERR

# 安全的 API 调用
safe_api_call() {
    local func_name="$1"
    shift
    "$@" 2>/dev/null
    return $?
}

# 测试函数中的错误处理
test_concurrent_uniqueness() {
    echo "=== 测试1: 并发生成唯一性测试 ==="
    
    local tmpdir=$(mktemp -d)
    local pids=()
    local error_occurred=0
    
    trap 'error_occurred=1; cleanup_tmpdir' RETURN
    
    cleanup_tmpdir() {
        if [ -d "$tmpdir" ]; then
            rm -rf "$tmpdir"
        fi
    }
    
    for i in $(seq 1 10); do
        (
            for j in $(seq 1 10); do
                local result
                result=$(generate_id "dist-test" "consistency" "concurrent-test" 2>/dev/null)
                if [ -n "$result" ]; then
                    local id=$(get_id_from_response "$result")
                    echo "$id" >> "$tmpdir/result.$i"
                fi
            done
        ) &
        pids+=($!)
    done
    
    for pid in "${pids[@]}"; do
        wait $pid 2>/dev/null || true
    done
    
    # 解析结果
    local all_ids=$(cat "$tmpdir"/result.* 2>/dev/null | sort -n)
    # ... 后续逻辑
    
    cleanup_tmpdir
    trap - RETURN
    
    return $error_occurred
}
```

---

## 4. 配置灵活性修复 (Medium)

### 4.1 环境配置加载

#### TEST-MED-001: 硬编码 localhost:8080

**修复规范**:

```bash
# lib.sh - 增强配置加载

# 多级配置优先级
# 1. 环境变量 (最高优先级)
# 2. 配置文件
# 3. 默认值 (最低优先级)

load_environment_config() {
    # API 基础地址
    NEBULA_API_BASE="${NEBULA_API_BASE:-}"
    NEBULA_API_BASE="${NEBULA_API_BASE:-$(read_config 'api_base' 'http://localhost:8080')}"
    export NEBULA_API_BASE
    
    # 监控地址
    NEBULA_METRICS_URL="${NEBULA_METRICS_URL:-}"
    NEBULA_METRICS_URL="${NEBULA_METRICS_URL:-$(read_config 'metrics_url' 'http://localhost:9091')}"
    export NEBULA_METRICS_URL
    
    # gRPC 地址
    NEBULA_GRPC_URL="${NEBULA_GRPC_URL:-}"
    NEBULA_GRPC_URL="${NEBULA_GRPC_URL:-$(read_config 'grpc_url' 'http://localhost:50051')}"
    export NEBULA_GRPC_URL
    
    # 超时配置
    NEBULA_API_TIMEOUT="${NEBULA_API_TIMEOUT:-$(read_config 'api_timeout' '30')}"
    export NEBULA_API_TIMEOUT
    
    echo "[INFO] 使用 API 地址: $NEBULA_API_BASE"
    echo "[INFO] 使用监控地址: $NEBULA_METRICS_URL"
}

# 配置文件读取
read_config() {
    local key=$1
    local default=$2
    local config_file="${TEST_CONFIG_FILE:-config/test_config.toml}"
    
    if [ -f "$config_file" ]; then
        case "$config_file" in
            *.toml)
                grep -E "^${key}\s*=" "$config_file" 2>/dev/null | \
                    sed 's/.*=\s*//' | tr -d ' "''' || echo "$default"
                ;;
            *)
                grep -E "^${key}=" "$config_file" 2>/dev/null | \
                    cut -d'=' -f2- | tr -d '"' || echo "$default"
                ;;
        esac
    else
        echo "$default"
    fi
}

# 便捷函数：获取 API 地址
get_api_base() {
    echo "${NEBULA_API_BASE:-http://localhost:8080}"
}

# 便捷函数：构建完整 URL
build_url() {
    local path="$1"
    local base=$(get_api_base)
    # 移除末尾斜杠，添加路径前导斜杠
    base="${base%/}"
    path="${path#/}"
    echo "${base}/${path}"
}
```

### 4.2 测试数据清理

#### TEST-MED-002: db_concurrency_test.sh 缺少清理

**修复规范**:

```bash
#!/bin/bash
# 数据库并发测试

# 清理函数
cleanup_test_data() {
    echo "[INFO] 清理测试数据..."
    
    local cleanup_workspace="${TEST_WORKSPACE:-concurrency}"
    local cleanup_count=0
    
    # 尝试清理 workspace（如果 API 支持）
    if [ -n "$AUTH_HEADER" ] && [ -n "$NEBULA_API_BASE" ]; then
        local response=$(curl -s -w "\n%{http_code}" -X DELETE \
            "${NEBULA_API_BASE}/api/v1/workspaces/${cleanup_workspace}" \
            -H "$AUTH_HEADER" 2>/dev/null)
        
        local http_code=$(echo "$response" | tail -n1)
        if [ "$http_code" = "200" ] || [ "$http_code" = "204" ]; then
            cleanup_count=$((cleanup_count + 1))
        fi
    fi
    
    # 清理临时文件
    local tmpdir="${TMPDIR:-/tmp/nebula_test}"
    if [ -d "$tmpdir" ]; then
        rm -rf "$tmpdir" 2>/dev/null && cleanup_count=$((cleanup_count + 1))
    fi
    
    echo "[INFO] 清理完成，清理项: $cleanup_count"
}

# 注册清理函数
trap cleanup_test_data EXIT

# 初始化测试数据
init_test_data() {
    echo "[INFO] 初始化测试环境..."
    
    local workspace="${TEST_WORKSPACE:-concurrency}"
    local base_url=$(get_api_base)
    
    # 配置测试用的业务标签
    curl -s -X POST "${base_url}/api/v1/config/algorithm" \
        -H "Content-Type: application/json" \
        -H "$AUTH_HEADER" \
        -d "{\"biz_tag\": \"concurrency:test\", \"algorithm\": \"segment\"}" 2>/dev/null || true
    
    echo "[INFO] 测试环境初始化完成"
}
```

---

## 5. 测试设计改进

### 5.1 并发测试逻辑简化

#### TEST-MED-003: distributed_test.sh 简化

**修复规范**:

```bash
#!/bin/bash
# 简化的并发测试

run_concurrent_test() {
    local name="$1"
    local worker_count="$2"
    local requests_per_worker="$3"
    local workspace="$4"
    local group="$5"
    local biz_tag="$6"
    
    echo "[TEST] $name"
    echo "  Workers: $worker_count, Requests/Worker: $requests_per_worker"
    
    local tmpdir=$(mktemp -d)
    local pids=()
    local success_count=0
    local fail_count=0
    
    # 并发执行
    for w in $(seq 1 $worker_count); do
        (
            local worker_success=0
            for r in $(seq 1 $requests_per_worker); do
                local http_code=$(curl -s -o /dev/null -w "%{http_code}" \
                    -X POST "$(get_api_base)/api/v1/generate" \
                    -H "Content-Type: application/json" \
                    -H "$AUTH_HEADER" \
                    -d "{\"workspace\": \"${workspace}\", \"group\": \"${group}\", \"biz_tag\": \"${biz_tag}\"}")
                
                if [ "$http_code" = "200" ]; then
                    worker_success=$((worker_success + 1))
                fi
            done
            echo "$worker_success" > "${tmpdir}/worker_${w}.result"
        ) &
        pids+=($!)
    done
    
    # 等待并收集结果
    for pid in "${pids[@]}"; do
        wait $pid 2>/dev/null || true
    done
    
    for w in $(seq 1 $worker_count); do
        if [ -f "${tmpdir}/worker_${w}.result" ]; then
            local count=$(cat "${tmpdir}/worker_${w}.result")
            success_count=$((success_count + count))
            fail_count=$((fail_count + requests_per_worker - count))
            rm -f "${tmpdir}/worker_${w}.result"
        fi
    done
    
    rm -rf "$tmpdir"
    
    local total=$((worker_count * requests_per_worker))
    echo "  结果: $success_count/$total 成功"
    
    if [ "$fail_count" -eq 0 ]; then
        echo "  [PASS]"
        return 0
    else
        echo "  [FAIL] $fail_count 个请求失败"
        return 1
    fi
}
```

---

## 6. 文档改进 (Low)

### 6.1 测试脚本文档模板

**规范要求**:

```bash
#!/bin/bash
#
# @file        db_concurrency_test.sh
# @brief       数据库并发测试 - 验证号段分配的并发安全性
#
# @description
#     此脚本测试 Nebula ID 在高并发场景下的数据库号段分配安全性。
#     主要验证:
#     1. 多线程/多进程同时请求 ID 时不产生重复
#     2. 批量请求的原子性和一致性
#     3. 短时间大量请求的压力测试
#
# @usage
#     # 使用默认配置（本地服务）
#     ./db_concurrency_test.sh
#
#     # 使用自定义配置
#     export NEBULA_API_BASE="http://your-server:8080"
#     export TEST_AUTH_HEADER="Authorization: Basic ..."
#     export TEST_WORKSPACE="your-workspace"
#     ./db_concurrency_test.sh
#
# @requirements
#     - curl
#     - jq (用于 JSON 解析)
#     - 运行的 Nebula ID 服务
#
# @environment_variables
#     NEBULA_API_BASE       - API 服务器地址 (默认: http://localhost:8080)
#     TEST_AUTH_HEADER      - 认证头 (默认: 从配置生成)
#     TEST_WORKSPACE        - 测试工作空间 (默认: concurrency)
#     TEST_CONFIG_FILE      - 配置文件路径
#
# @exit_codes
#     0 - 所有测试通过
#     1 - 部分测试失败
#     2 - 缺少必要工具或服务不可用
#
# @see
#     lib.sh - 通用测试函数库
#     degradation_test.sh - 降级机制测试
#
# @author      Nebula ID Team
# @version     1.0.0
# @date        2026-01-12
#
```

---

## 7. 统一测试报告格式

### 7.1 报告生成改进

**规范要求**:

```bash
#!/bin/bash
# 增强的报告生成

# 报告格式
REPORT_FORMAT="${REPORT_FORMAT:-json}"  # json | junit | text

# 初始化报告
init_report() {
    local report_file="${TEST_REPORT_FILE:-test_results_$(date +%Y%m%d_%H%M%S)}"
    
    case "$REPORT_FORMAT" in
        json)
            echo '{"test_suite": "Nebula ID Tests", "timestamp": "'$(date -Iseconds)'", "tests": []}' > "$report_file.json"
            REPORT_FILE="$report_file.json"
            ;;
        junit)
            cat > "$report_file.xml" << 'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<testsuites name="Nebula ID Tests">
  <testsuite name="Integration Tests">
EOF
            REPORT_FILE="$report_file.xml"
            ;;
        *)
            echo "Nebula ID API Test Report" > "$report_file.txt"
            echo "Generated: $(date)" >> "$report_file.txt"
            echo "Base URL: $(get_api_base)" >> "$report_file.txt"
            echo "========================================" >> "$report_file.txt"
            REPORT_FILE="$report_file.txt"
            ;;
    esac
}

# 记录测试结果
log_result() {
    local test_name="$1"
    local status="$2"  # PASS | FAIL | SKIP
    local duration="$3"
    local message="$4"
    
    case "$REPORT_FORMAT" in
        json)
            # 使用 jq 追加结果（需要 jq 工具）
            local temp_file=$(mktemp)
            cat "$REPORT_FILE" | jq --arg name "$test_name" \
                --arg status "$status" \
                --arg duration "$duration" \
                --arg message "$message" \
                '.tests += [{"name": $name, "status": $status, "duration": $duration, "message": $message}]' > "$temp_file"
            mv "$temp_file" "$REPORT_FILE"
            ;;
        junit)
            local classname="NebulaID"
            local testcase_name=$(echo "$test_name" | tr ' ' '_')
            echo "    <testcase name=\"$testcase_name\" classname=\"$classname\" time=\"$duration\">" >> "$REPORT_FILE"
            if [ "$status" = "FAIL" ]; then
                echo "      <failure message=\"$message\">FAIL</failure>" >> "$REPORT_FILE"
            elif [ "$status" = "SKIP" ]; then
                echo "      <skipped message=\"$message\">SKIP</skipped>" >> "$REPORT_FILE"
            fi
            echo "    </testcase>" >> "$REPORT_FILE"
            ;;
        *)
            echo "[$(date '+%Y-%m-%d %H:%M:%S')] [$status] ${test_name}: ${message} (${duration}ms)" >> "$REPORT_FILE"
            ;;
    esac
}

# 完成报告
finalize_report() {
    local total_tests="$1"
    local passed="$2"
    local failed="$3"
    local skipped="$4"
    
    case "$REPORT_FORMAT" in
        json)
            local temp_file=$(mktemp)
            cat "$REPORT_FILE" | jq --arg total "$total_tests" \
                --arg passed "$passed" \
                --arg failed "$failed" \
                --arg skipped "$skipped" \
                '.summary = {"total": ($total | tonumber), "passed": ($passed | tonumber), "failed": ($failed | tonumber), "skipped": ($skipped | tonumber)}' > "$temp_file"
            mv "$temp_file" "$REPORT_FILE"
            ;;
        junit)
            echo "    <system-out>Total: $total_tests, Passed: $passed, Failed: $failed, Skipped: $skipped</system-out>" >> "$REPORT_FILE"
            echo "  </testsuite>" >> "$REPORT_FILE"
            echo "</testsuites>" >> "$REPORT_FILE"
            ;;
        *)
            echo "" >> "$REPORT_FILE"
            echo "========================================" >> "$REPORT_FILE"
            echo "测试结果汇总" >> "$REPORT_FILE"
            echo "========================================" >> "$REPORT_FILE"
            echo "总计: $total_tests" >> "$REPORT_FILE"
            echo "通过: $passed" >> "$REPORT_FILE"
            echo "失败: $failed" >> "$REPORT_FILE"
            echo "跳过: $skipped" >> "$REPORT_FILE"
            ;;
    esac
    
    echo "[INFO] 报告已保存到: $REPORT_FILE"
}
```

---

## 8. 配置文件示例

### 8.1 test_config.toml

```toml
# Nebula ID 测试配置文件
# 优先级: 环境变量 > 此配置文件 > 默认值

[api]
# API 服务器地址
base = "http://localhost:8080"
# 超时时间（秒）
timeout = 30

[auth]
# 认证头（优先使用环境变量 TEST_AUTH_HEADER）
# header = "Authorization: Basic ..."

[workspace]
# 默认测试工作空间
name = "test-workspace"
# 默认测试分组
group = "default"

[logging]
# 日志级别: debug | info | warn | error
level = "info"
# 是否彩色输出
color = true

[report]
# 报告格式: json | junit | text
format = "text"
# 报告文件路径（包含文件名，不包含扩展名）
output = "test_results"
```

### 8.2 test_config.env (ENV 格式)

```bash
# ENV 格式测试配置
# 适用于简单场景，直接通过 source 加载

# API 配置
export NEBULA_API_BASE="http://localhost:8080"
export NEBULA_API_TIMEOUT="30"

# 认证配置（必须通过环境变量提供）
export TEST_AUTH_HEADER=""

# 工作空间配置
export TEST_WORKSPACE="test-workspace"
export TEST_GROUP="default"

# 报告配置
export TEST_REPORT_FORMAT="json"
```

---

## 9. 实施检查清单

### 9.1 修复优先级

| 优先级 | 问题 ID | 修复内容 | 预计工时 |
|--------|---------|----------|----------|
| P0 | TEST-CRIT-001/002/003 | 移除所有硬编码凭据 | 2h |
| P1 | TEST-HIGH-001 | 配置集中管理 | 2h |
| P1 | TEST-HIGH-002/003 | 改进错误处理 | 3h |
| P2 | TEST-MED-001 | 统一配置加载 | 2h |
| P2 | TEST-MED-002 | 添加测试清理 | 1h |
| P2 | TEST-MED-003 | 简化并发逻辑 | 2h |
| P3 | TEST-LOW-001 | 添加文档注释 | 1h |
| P3 | TEST-LOW-002 | 统一报告格式 | 2h |

### 9.2 验证步骤

1. ✅ 所有脚本不再包含硬编码的 Base64 凭据
2. ✅ 配置通过环境变量或配置文件加载
3. ✅ 测试脚本在 API 不可用时不会意外退出
4. ✅ 所有测试结果生成统一格式的报告
5. ✅ 测试完成后自动清理测试数据

---

## 10. 相关文档

- [用户指南](../docs/USER_GUIDE.md)
- [API 参考](../docs/API_REFERENCE.md)
- [贡献指南](../docs/CONTRIBUTING.md)

---

**文档版本历史**:

| 版本 | 日期 | 作者 | 变更说明 |
|------|------|------|----------|
| 1.0.0 | 2026-01-12 | Nebula ID Team | 初始版本 |
