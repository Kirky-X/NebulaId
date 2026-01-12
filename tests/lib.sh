#!/bin/bash
# 测试通用库 - Nebula ID 测试套件
#
# @description 提供测试脚本共用的函数和配置加载机制
#
# @config_order 优先级: 环境变量 > 配置文件 > 默认值
#
# @required_env
#   NEBULA_API_BASE    - API 服务器地址
#   TEST_AUTH_HEADER   - 认证头
#   TEST_CONFIG_FILE   - 配置文件路径
#
# @usage
#   source lib.sh
#   load_test_config   # 加载配置
#
# @config_files
#   - config/test_config.toml (默认配置)
#   - 其他自定义配置文件 (通过 TEST_CONFIG_FILE 指定)
#
# @example
#   # 使用自定义配置文件
#   export TEST_CONFIG_FILE="/path/to/my-config.toml"
#   source lib.sh
#   load_test_config
#

# 全局配置变量
AUTH_HEADER=""
API_BASE="http://localhost:8080"
REPORT_FILE=""

# 测试统计
TEST_TOTAL=0
TEST_PASSED=0
TEST_FAILED=0
TEST_SKIPPED=0

# 默认配置文件路径
DEFAULT_CONFIG_FILE="config/test_config.toml"

# ========== 配置加载 ==========

# 从配置文件读取值（支持 ENV 和 TOML 格式）
read_config() {
    local key="$1"
    local default="$2"
    local config_file="${TEST_CONFIG_FILE:-$DEFAULT_CONFIG_FILE}"

    if [ -f "$config_file" ]; then
        local ext="${config_file##*.}"
        case "$ext" in
            env)
                local value=$(grep -E "^${key}=" "$config_file" 2>/dev/null | cut -d'=' -f2- | tr -d '"' | tr -d "'")
                [ -n "$value" ] && echo "$value" || echo "$default"
                ;;
            toml)
                # 支持 TOML 格式: key = "value" 或 key = value
                local value=$(grep -E "^${key}\s*=" "$config_file" 2>/dev/null | sed 's/.*=\s*//' | tr -d ' "' | tr -d "'")
                [ -n "$value" ] && echo "$value" || echo "$default"
                ;;
            *)
                local value=$(grep -E "^${key}=" "$config_file" 2>/dev/null | cut -d'=' -f2- | tr -d '"' | tr -d "'")
                [ -n "$value" ] && echo "$value" || echo "$default"
                ;;
        esac
    else
        echo "$default"
    fi
}

# 检查配置文件是否存在
config_file_exists() {
    local config_file="${TEST_CONFIG_FILE:-$DEFAULT_CONFIG_FILE}"
    [ -f "$config_file" ]
}

# 获取配置文件路径
get_config_path() {
    local config_file="${TEST_CONFIG_FILE:-$DEFAULT_CONFIG_FILE}"
    echo "$config_file"
}

# 加载 API 基础地址
_load_api_base() {
    if [ -n "$NEBULA_API_BASE" ]; then
        API_BASE="$NEBULA_API_BASE"
    else
        API_BASE=$(read_config 'api_base' 'http://localhost:8080')
    fi
}

# 加载测试配置（多级配置加载机制）
load_test_config() {
    local config_file="${TEST_CONFIG_FILE:-$DEFAULT_CONFIG_FILE}"
    local config_loaded=false

    echo "[INFO] 加载测试配置..."
    echo "[INFO] 配置文件: $config_file"

    # 检查配置文件是否存在
    if [ -f "$config_file" ]; then
        echo "[INFO] 配置文件存在"
    else
        echo "[WARN] 配置文件不存在: $config_file"
        echo "[INFO] 将使用环境变量和默认值"
    fi

    # 优先级1: 环境变量
    if [ -n "$TEST_AUTH_HEADER" ]; then
        AUTH_HEADER="$TEST_AUTH_HEADER"
        echo "[INFO] 使用环境变量提供的认证头 (TEST_AUTH_HEADER)"
        config_loaded=true
    fi

    _load_api_base

    # 优先级2: 配置文件 (仅当环境变量未设置时)
    if [ -z "$AUTH_HEADER" ] && [ -f "$config_file" ]; then
        local header=$(read_config 'auth_header' '')
        if [ -n "$header" ]; then
            AUTH_HEADER="$header"
            echo "[INFO] 使用配置文件提供的认证头"
            config_loaded=true
        fi
    fi

    # 优先级3: 动态生成测试凭据（仅测试环境）
    if [ -z "$AUTH_HEADER" ]; then
        local test_key_id="${TEST_API_KEY_ID:-test-key-id}"
        local test_secret="${TEST_API_SECRET:-test-secret}"
        AUTH_HEADER="Authorization: Basic $(echo -n "${test_key_id}:${test_secret}" | base64)"
        echo "[WARN] 使用默认测试凭据，建议通过环境变量或配置文件提供正式凭据"
        config_loaded=true
    fi

    echo "[INFO] API 地址: $API_BASE"
    echo "[INFO] 配置加载完成"
}

# 获取 API 基础地址
get_api_base() {
    echo "${API_BASE:-http://localhost:8080}"
}

# 构建完整 URL
build_url() {
    local path="$1"
    local base=$(get_api_base)
    base="${base%/}"
    path="${path#/}"
    echo "${base}/${path}"
}

# 初始化配置（在脚本开头调用）
_init_config() {
    load_test_config
}

# 日志颜色
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

# ========== 基础检查 ==========

check_prerequisites() {
    local missing=0
    for cmd in curl jq; do
        if ! command -v $cmd &> /dev/null; then
            echo -e "${RED}❌ 缺少必要工具: $cmd${NC}"
            missing=1
        fi
    done
    if [ $missing -eq 1 ]; then
        exit 1
    fi
}

check_api_health() {
    echo -e "\n${YELLOW}【健康检查】${NC}"
    local health=$(curl -s "$(get_api_base)/health")
    local status=$(echo "$health" | jq -r '.status')
    echo "系统状态: $status"

    if [ "$status" != "healthy" ]; then
        echo -e "${YELLOW}⚠️  服务未就绪，请确保 nebula-id 服务正在运行${NC}"
        return 1
    fi
    echo -e "${GREEN}✅ 服务健康${NC}"
    return 0
}

# ========== API 调用 ==========

switch_algorithm() {
    local biz_tag="$1"
    local algorithm="$2"
    curl -s -X POST "$(get_api_base)/api/v1/config/algorithm" \
        -H "Content-Type: application/json" \
        -H "$AUTH_HEADER" \
        -d "{\"biz_tag\": \"${biz_tag}\", \"algorithm\": \"${algorithm}\"}"
}

generate_id() {
    local workspace="$1"
    local group="$2"
    local biz_tag="$3"
    curl -s -X POST "$(get_api_base)/api/v1/generate" \
        -H "Content-Type: application/json" \
        -H "$AUTH_HEADER" \
        -d "{\"workspace\": \"${workspace}\", \"group\": \"${group}\", \"biz_tag\": \"${biz_tag}\"}"
}

generate_id_with_algo() {
    local workspace="$1"
    local group="$2"
    local biz_tag="$3"
    local algorithm="$4"
    curl -s -X POST "$(get_api_base)/api/v1/generate" \
        -H "Content-Type: application/json" \
        -H "$AUTH_HEADER" \
        -d "{\"workspace\": \"${workspace}\", \"group\": \"${group}\", \"biz_tag\": \"${biz_tag}\", \"algorithm\": \"${algorithm}\"}"
}

generate_batch() {
    local workspace="$1"
    local group="$2"
    local biz_tag="$3"
    local size="$4"
    curl -s -X POST "$(get_api_base)/api/v1/generate/batch" \
        -H "Content-Type: application/json" \
        -H "$AUTH_HEADER" \
        -d "{\"workspace\": \"${workspace}\", \"group\": \"${group}\", \"biz_tag\": \"${biz_tag}\", \"size\": ${size}}"
}

parse_id() {
    local id="$1"
    local workspace="$2"
    local group="$3"
    local biz_tag="$4"
    local algorithm="$5"
    curl -s -X POST "$(get_api_base)/api/v1/parse" \
        -H "Content-Type: application/json" \
        -H "$AUTH_HEADER" \
        -d "{\"id\": \"${id}\", \"workspace\": \"${workspace}\", \"group\": \"${group}\", \"biz_tag\": \"${biz_tag}\", \"algorithm\": \"${algorithm}\"}"
}

# ========== 响应解析 ==========

get_id_from_response() {
    echo "$1" | jq -r '.id'
}

get_algorithm_from_response() {
    echo "$1" | jq -r '.algorithm'
}

get_ids_from_batch_response() {
    echo "$1" | jq -r '.ids[]'
}

get_id_count_from_batch() {
    echo "$1" | jq -r '.ids | length'
}

# ========== 雪花ID解析 ==========

parse_snowflake_id() {
    local id="$1"
    local timestamp=$((id >> 22))
    local datacenter=$(((id >> 17) & 31))
    local worker=$(((id >> 12) & 31))
    local sequence=$((id & 4095))
    
    echo "timestamp:${timestamp},datacenter:${datacenter},worker:${worker},sequence:${sequence}"
}

# ========== 格式验证 ==========

verify_id_not_null() {
    local id="$1"
    if [ "$id" == "null" ] || [ -z "$id" ]; then
        return 1
    fi
    return 0
}

verify_numeric_format() {
    local id="$1"
    if [[ "$id" =~ ^[0-9]+$ ]]; then
        return 0
    fi
    return 1
}

verify_uuid_format() {
    local id="$1"
    if echo "$id" | grep -Eq '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'; then
        return 0
    fi
    return 1
}

verify_uuid_v7_format() {
    local id="$1"
    if [[ "$id" =~ ^\{?[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[0-9a-f]{4}-[0-9a-f]{12}\}?$ ]]; then
        return 0
    fi
    return 1
}

# ========== 输出格式化 ==========

print_header() {
    echo ""
    echo -e "${BLUE}========================================${NC}"
    echo -e "${BLUE}$1${NC}"
    echo -e "${BLUE}========================================${NC}"
}

print_section() {
    echo ""
    echo -e "${YELLOW}【$1】${NC}"
    echo -e "${YELLOW}----------------------------------------${NC}"
}

print_pass() {
    echo -e "  ${GREEN}[PASS]${NC} $1"
}

print_fail() {
    echo -e "  ${RED}[FAIL]${NC} $1"
}

print_skip() {
    echo -e "  ${YELLOW}[SKIP]${NC} $1"
}

print_info() {
    echo -e "  ${YELLOW}[INFO]${NC} $1"
}

# ========== 测试统计 ==========

# 初始化测试计数器
init_test_counters() {
    TEST_TOTAL=0
    TEST_PASSED=0
    TEST_FAILED=0
    TEST_SKIPPED=0
}

# 记录通过
test_pass() {
    local message="$1"
    print_pass "$message"
    TEST_PASSED=$((TEST_PASSED + 1))
    TEST_TOTAL=$((TEST_TOTAL + 1))
}

# 记录失败
test_fail() {
    local message="$1"
    print_fail "$message"
    TEST_FAILED=$((TEST_FAILED + 1))
    TEST_TOTAL=$((TEST_TOTAL + 1))
}

# 记录跳过
test_skip() {
    local message="$1"
    print_skip "$message"
    TEST_SKIPPED=$((TEST_SKIPPED + 1))
    TEST_TOTAL=$((TEST_TOTAL + 1))
}

# 打印测试摘要
print_test_summary() {
    print_header "测试摘要"
    echo "  总测试数: $TEST_TOTAL"
    echo "  通过: $TEST_PASSED"
    echo "  失败: $TEST_FAILED"
    echo "  跳过: $TEST_SKIPPED"

    if [ -n "$REPORT_FILE" ] && [ -f "$REPORT_FILE" ]; then
        echo "" >> "$REPORT_FILE"
        echo "========================================" >> "$REPORT_FILE"
        echo "测试摘要" >> "$REPORT_FILE"
        echo "========================================" >> "$REPORT_FILE"
        echo "总测试数: $TEST_TOTAL" >> "$REPORT_FILE"
        echo "通过: $TEST_PASSED" >> "$REPORT_FILE"
        echo "失败: $TEST_FAILED" >> "$REPORT_FILE"
        echo "跳过: $TEST_SKIPPED" >> "$REPORT_FILE"
        echo "报告文件: $REPORT_FILE" >> "$REPORT_FILE"
    fi
}

# ========== 报告生成 ==========

# 初始化测试报告（统一格式）
init_report() {
    local test_name="${1:-Test}"
    local timestamp=$(date +%Y%m%d_%H%M%S)
    REPORT_FILE="${test_name,,}_test_results_${timestamp}.txt"

    # 重置计数器
    init_test_counters

    # 创建报告文件
    cat > "$REPORT_FILE" << EOF
========================================
${test_name} 测试报告
========================================
生成时间: $(date)
API 地址: $(get_api_base)
========================================

EOF
    echo "[INFO] 报告文件: $REPORT_FILE"
}

# 记录测试结果到报告
log_result() {
    local test_name="$1"
    local status="$2"
    local details="$3"
    local timestamp=$(date '+%Y-%m-%d %H:%M:%S')
    echo "[$timestamp] [$status] $test_name: $details" >> "$REPORT_FILE"
}

# 打印测试结果
log_test_result() {
    local test_name="$1"
    local status="$2"
    local details="$3"

    case "$status" in
        PASS)
            test_pass "$details"
            ;;
        FAIL)
            test_fail "$details"
            ;;
        SKIP)
            test_skip "$details"
            ;;
    esac
    log_result "$test_name" "$status" "$details"
}

# 完成报告
finalize_report() {
    local summary="$1"

    if [ -n "$REPORT_FILE" ] && [ -f "$REPORT_FILE" ]; then
        echo "" >> "$REPORT_FILE"
        echo "========================================" >> "$REPORT_FILE"
        echo "测试详情" >> "$REPORT_FILE"
        echo "========================================" >> "$REPORT_FILE"
        echo "$summary" >> "$REPORT_FILE"
        print_test_summary
        echo "[INFO] 完整报告: $REPORT_FILE"
    fi
}

# ========== 并发测试辅助 ==========

run_concurrent_test() {
    local name="$1"
    local worker_count="$2"
    local requests_per_worker="$3"
    local workspace="$4"
    local group="$5"
    local biz_tag="$6"
    local algorithm="$7"

    local tmpdir=$(mktemp -d)
    local pids=()
    local total_success=0

    for w in $(seq 1 $worker_count); do
        (
            local local_success=0
            for r in $(seq 1 $requests_per_worker); do
                local http_code=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$(get_api_base)/api/v1/generate" \
                    -H "Content-Type: application/json" \
                    -H "$AUTH_HEADER" \
                    -d "{\"workspace\": \"${workspace}\", \"group\": \"${group}\", \"biz_tag\": \"${biz_tag}\", \"algorithm\": \"${algorithm}\"}")
                if [ "$http_code" = "200" ]; then
                    local_success=$((local_success + 1))
                fi
            done
            echo "$local_success" > "${tmpdir}/worker_${w}.tmp"
        ) &
        pids+=($!)
    done

    for pid in "${pids[@]}"; do
        wait $pid 2>/dev/null || true
    done

    for w in $(seq 1 $worker_count); do
        if [ -f "${tmpdir}/worker_${w}.tmp" ]; then
            local count=$(cat "${tmpdir}/worker_${w}.tmp")
            total_success=$((total_success + count))
            rm -f "${tmpdir}/worker_${w}.tmp"
        fi
    done

    rm -rf "$tmpdir"
    echo "$total_success"
}

# ========== 性能测试辅助 ==========

measure_performance() {
    local name="$1"
    local request_count="$2"
    local workspace="$3"
    local group="$4"
    local biz_tag="$5"

    local start_time=$(date +%s%N)
    local success=0

    for i in $(seq 1 $request_count); do
        local http_code=$(curl -s -o /dev/null -w "%{http_code}" -X POST "$(get_api_base)/api/v1/generate" \
            -H "Content-Type: application/json" \
            -H "$AUTH_HEADER" \
            -d "{\"workspace\": \"${workspace}\", \"group\": \"${group}\", \"biz_tag\": \"${biz_tag}\"}")
        if [ "$http_code" = "200" ]; then
            success=$((success + 1))
        fi
    done

    local end_time=$(date +%s%N)
    local duration=$((($end_time - $start_time) / 1000000))
    local tps=$((request_count * 1000 / duration))

    echo "$success/$request_count, ${duration}ms, TPS: $tps"
}
