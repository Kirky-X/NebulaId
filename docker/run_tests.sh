#!/bin/bash

set -e

# ============================================
# Nebula ID 统一测试脚本
# ============================================

# 颜色输出
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
MAGENTA='\033[0;35m'
BOLD='\033[1m'
NC='\033[0m'

# 测试配置
BASE_URL=${NEBULA_BASE_URL:-"http://localhost:8080"}
GRPC_HOST=${NEBULA_GRPC_HOST:-"localhost"}
GRPC_PORT=${NEBULA_GRPC_PORT:-9091}
TIMEOUT=10
REPORT_DIR="docker/test_reports"

# 全局测试计数器
TOTAL_TESTS=0
PASSED_TESTS=0
FAILED_TESTS=0
SKIPPED_TESTS=0
WARNINGS=0

# 性能指标
declare -A RESPONSE_TIMES
TOTAL_RESPONSE_TIME=0
MIN_RESPONSE_TIME=999999
MAX_RESPONSE_TIME=0

# 测试结果存储
declare -a TEST_RESULTS=()

# 创建报告目录
mkdir -p "$REPORT_DIR"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
REPORT_FILE="$REPORT_DIR/combined_test_${TIMESTAMP}.txt"

# 日志函数
log_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

log_test() {
    echo -e "${BLUE}[TEST]${NC} $1"
}

log_pass() {
    echo -e "${GREEN}[PASS]${NC} $1"
    ((PASSED_TESTS++))
    ((TOTAL_TESTS++))
}

log_fail() {
    echo -e "${RED}[FAIL]${NC} $1"
    ((FAILED_TESTS++))
    ((TOTAL_TESTS++))
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
    ((WARNINGS++))
}

log_skip() {
    echo -e "${CYAN}[SKIP]${NC} $1"
    ((SKIPPED_TESTS++))
    ((TOTAL_TESTS++))
}

log_perf() {
    echo -e "${MAGENTA}[PERF]${NC} $1"
}

log_section() {
    echo ""
    echo -e "${BOLD}${BLUE}══════════════════════════════════════════════════════════════════${NC}"
    echo -e "${BOLD}${BLUE}  $1${NC}"
    echo -e "${BOLD}${BLUE}══════════════════════════════════════════════════════════════════${NC}"
}

# 记录测试结果
record_test_result() {
    local test_name="$1"
    local status="$2"
    local details="$3"
    local response_time="${4:-N/A}"

    TEST_RESULTS+=("$test_name|$status|$details|$response_time")
}

# 初始化报告
init_report() {
    cat > "$REPORT_FILE" << EOF
================================================================================
                        Nebula ID 综合测试报告
================================================================================
测试时间: $(date '+%Y-%m-%d %H:%M:%S')
测试环境:
  - HTTP API:  $BASE_URL
  - gRPC:      ${GRPC_HOST}:${GRPC_PORT}
超时设置: ${TIMEOUT}秒
报告文件: $REPORT_FILE

--------------------------------------------------------------------------------
                            测试摘要
--------------------------------------------------------------------------------
EOF
}

# HTTP 请求函数
http_request() {
    local method=$1
    local endpoint=$2
    local data=$3

    curl -X "$method" "${BASE_URL}${endpoint}" \
        -H "Content-Type: application/json" \
        -d "$data" \
        --silent \
        --max-time "$TIMEOUT" \
        -w "\n%{http_code}\n%{time_total}" 2>/dev/null
}

# ============================================
# API 测试模块
# ============================================

run_api_tests() {
    log_section "API 功能测试"

    # 测试1: 健康检查
    log_test "测试1: 健康检查接口 /health"
    local response=$(http_request "GET" "/health" "" "")
    local http_code=$(echo "$response" | tail -n1)
    local body=$(echo "$response" | head -n-2)
    local response_time=$(echo "$response" | tail -n2 | head -n1)

    if [ "$http_code" = "200" ]; then
        log_pass "健康检查 - HTTP状态码: $http_code, 响应时间: ${response_time}s"
        record_test_result "健康检查" "通过" "HTTP状态码: $http_code" "${response_time}s"
        update_performance "health" "$response_time"
    else
        log_fail "健康检查 - HTTP状态码: $http_code"
        record_test_result "健康检查" "失败" "HTTP状态码: $http_code" "${response_time}s"
    fi

    # 测试2: 生成单个ID
    log_test "测试2: 生成单个ID接口 /api/v1/generate"
    local request_data='{"workspace":"test-workspace","group":"test-group","biz_tag":"test-tag"}'
    response=$(http_request "POST" "/api/v1/generate" "$request_data" "")
    http_code=$(echo "$response" | tail -n1)
    body=$(echo "$response" | head -n-2)
    response_time=$(echo "$response" | tail -n2 | head -n1)

    if [ "$http_code" = "200" ]; then
        local id=$(echo "$body" | grep -o '"id":"[^"]*"' | cut -d'"' -f4)
        if [ -n "$id" ]; then
            log_pass "生成单个ID - 成功生成ID: $id, 响应时间: ${response_time}s"
            record_test_result "生成单个ID" "通过" "生成的ID: $id" "${response_time}s"
            update_performance "generate_single" "$response_time"
        else
            log_fail "生成单个ID - 响应中未找到ID字段"
            record_test_result "生成单个ID" "失败" "响应格式错误" "${response_time}s"
        fi
    else
        log_fail "生成单个ID - HTTP状态码: $http_code"
        record_test_result "生成单个ID" "失败" "HTTP状态码: $http_code" "${response_time}s"
    fi

    # 测试3: 批量生成ID
    log_test "测试3: 批量生成ID接口 /api/v1/generate/batch"
    request_data='{"workspace":"test-workspace","group":"test-group","biz_tag":"test-tag","size":10}'
    response=$(http_request "POST" "/api/v1/generate/batch" "$request_data" "")
    http_code=$(echo "$response" | tail -n1)
    body=$(echo "$response" | head -n-2)
    response_time=$(echo "$response" | tail -n2 | head -n1)

    if [ "$http_code" = "200" ]; then
        local ids_count=$(echo "$body" | grep -o '"ids":\[' | head -1 || echo "")
        if [ -n "$ids_count" ]; then
            log_pass "批量生成ID - 批量生成成功, 响应时间: ${response_time}s"
            record_test_result "批量生成ID" "通过" "批量大小: 10" "${response_time}s"
            update_performance "generate_batch" "$response_time"
        else
            log_fail "批量生成ID - 响应格式不正确"
            record_test_result "批量生成ID" "失败" "响应格式错误" "${response_time}s"
        fi
    else
        log_fail "批量生成ID - HTTP状态码: $http_code"
        record_test_result "批量生成ID" "失败" "HTTP状态码: $http_code" "${response_time}s"
    fi

    # 测试4: 解析ID
    log_test "测试4: 解析ID接口 /api/v1/parse"
    request_data='{"id":"4200000000000000001","workspace":"test-workspace","group":"test-group","biz_tag":"test-tag"}'
    response=$(http_request "POST" "/api/v1/parse" "$request_data" "")
    http_code=$(echo "$response" | tail -n1)
    body=$(echo "$response" | head -n-2)
    response_time=$(echo "$response" | tail -n2 | head -n1)

    if [ "$http_code" = "200" ] || [ "$http_code" = "400" ]; then
        log_pass "解析ID - HTTP状态码: $http_code, 响应时间: ${response_time}s"
        record_test_result "解析ID" "通过" "HTTP状态码: $http_code" "${response_time}s"
        update_performance "parse" "$response_time"
    else
        log_fail "解析ID - HTTP状态码: $http_code"
        record_test_result "解析ID" "失败" "HTTP状态码: $http_code" "${response_time}s"
    fi

    # 测试5: 指标接口
    log_test "测试5: 指标接口 /metrics"
    response=$(http_request "GET" "/metrics" "" "")
    http_code=$(echo "$response" | tail -n1)
    body=$(echo "$response" | head -n-2)
    response_time=$(echo "$response" | tail -n2 | head -n1)

    if [ "$http_code" = "200" ]; then
        log_pass "指标接口 - HTTP状态码: $http_code, 响应时间: ${response_time}s"
        record_test_result "指标接口" "通过" "返回Prometheus格式指标" "${response_time}s"
        update_performance "metrics" "$response_time"
    else
        log_fail "指标接口 - HTTP状态码: $http_code"
        record_test_result "指标接口" "失败" "HTTP状态码: $http_code" "${response_time}s"
    fi

    # 测试6: 配置接口
    log_test "测试6: 配置接口 /api/v1/config"
    response=$(http_request "GET" "/api/v1/config" "" "")
    http_code=$(echo "$response" | tail -n1)
    body=$(echo "$response" | head -n-2)
    response_time=$(echo "$response" | tail -n2 | head -n1)

    if [ "$http_code" = "200" ]; then
        log_pass "配置接口 - HTTP状态码: $http_code, 响应时间: ${response_time}s"
        record_test_result "配置接口" "通过" "HTTP状态码: $http_code" "${response_time}s"
        update_performance "config" "$response_time"
    else
        log_fail "配置接口 - HTTP状态码: $http_code"
        record_test_result "配置接口" "失败" "HTTP状态码: $http_code" "${response_time}s"
    fi
}

# ============================================
# 批量验证测试模块
# ============================================

run_batch_validation_tests() {
    log_section "批量参数验证测试"

    # 测试1: 批量大小为 0
    log_test "测试1: 批量大小为 0"
    local request_data='{"workspace":"test-workspace","group":"test-group","biz_tag":"test-tag","size":0}'
    local response=$(http_request "POST" "/api/v1/generate/batch" "$request_data")
    local http_code=$(echo "$response" | tail -n1)
    local body=$(echo "$response" | head -n-2)

    if [ "$http_code" = "400" ]; then
        log_pass "批量大小为 0 - 正确拒绝 (HTTP 400)"
        record_test_result "批量大小=0" "通过" "正确拒绝无效值"
    else
        log_fail "批量大小为 0 - 期望HTTP 400，实际: $http_code"
        record_test_result "批量大小=0" "失败" "期望HTTP 400，实际: $http_code"
    fi

    # 测试2: 批量大小为负数
    log_test "测试2: 批量大小为 -1"
    request_data='{"workspace":"test-workspace","group":"test-group","biz_tag":"test-tag","size":-1}'
    response=$(http_request "POST" "/api/v1/generate/batch" "$request_data")
    http_code=$(echo "$response" | tail -n1)
    body=$(echo "$response" | head -n-2)

    if [ "$http_code" = "400" ]; then
        log_pass "批量大小为 -1 - 正确拒绝 (HTTP 400)"
        record_test_result "批量大小=-1" "通过" "正确拒绝负数"
    else
        log_fail "批量大小为 -1 - 期望HTTP 400，实际: $http_code"
        record_test_result "批量大小=-1" "失败" "期望HTTP 400，实际: $http_code"
    fi

    # 测试3: 批量大小超过最大值
    log_test "测试3: 批量大小为 1000000 (超过最大值)"
    request_data='{"workspace":"test-workspace","group":"test-group","biz_tag":"test-tag","size":1000000}'
    response=$(http_request "POST" "/api/v1/generate/batch" "$request_data")
    http_code=$(echo "$response" | tail -n1)
    body=$(echo "$response" | head -n-2)

    if [ "$http_code" = "400" ]; then
        log_pass "批量大小为 1000000 - 正确拒绝 (HTTP 400)"
        record_test_result "批量大小=1000000" "通过" "正确拒绝超限值"
    else
        log_fail "批量大小为 1000000 - 期望HTTP 400，实际: $http_code"
        record_test_result "批量大小=1000000" "失败" "期望HTTP 400，实际: $http_code"
    fi

    # 测试4: 批量大小为边界值
    log_test "测试4: 批量大小为 100 (边界值)"
    request_data='{"workspace":"test-workspace","group":"test-group","biz_tag":"test-tag","size":100}'
    response=$(http_request "POST" "/api/v1/generate/batch" "$request_data")
    http_code=$(echo "$response" | tail -n1)
    body=$(echo "$response" | head -n-2)

    if [ "$http_code" = "200" ]; then
        log_pass "批量大小为 100 - 正确接受 (HTTP 200)"
        record_test_result "批量大小=100" "通过" "正确接受边界值"
    else
        log_fail "批量大小为 100 - 期望HTTP 200，实际: $http_code"
        record_test_result "批量大小=100" "失败" "期望HTTP 200，实际: $http_code"
    fi

    # 测试5: 批量大小为正常值
    log_test "测试5: 批量大小为 10 (正常值)"
    request_data='{"workspace":"test-workspace","group":"test-group","biz_tag":"test-tag","size":10}'
    response=$(http_request "POST" "/api/v1/generate/batch" "$request_data")
    http_code=$(echo "$response" | tail -n1)
    body=$(echo "$response" | head -n-2)

    if [ "$http_code" = "200" ]; then
        log_pass "批量大小为 10 - 正确接受 (HTTP 200)"
        record_test_result "批量大小=10" "通过" "正确接受正常值"
    else
        log_fail "批量大小为 10 - 期望HTTP 200，实际: $http_code"
        record_test_result "批量大小=10" "失败" "期望HTTP 200，实际: $http_code"
    fi

    # 测试6: 缺少 size 参数
    log_test "测试6: 缺少 size 参数"
    request_data='{"workspace":"test-workspace","group":"test-group","biz_tag":"test-tag"}'
    response=$(http_request "POST" "/api/v1/generate/batch" "$request_data")
    http_code=$(echo "$response" | tail -n1)
    body=$(echo "$response" | head -n-2)

    if [ "$http_code" = "400" ]; then
        log_pass "缺少 size 参数 - 正确拒绝 (HTTP 400)"
        record_test_result "缺少size参数" "通过" "正确拒绝缺少参数"
    else
        log_fail "缺少 size 参数 - 期望HTTP 400，实际: $http_code"
        record_test_result "缺少size参数" "失败" "期望HTTP 400，实际: $http_code"
    fi
}

# ============================================
# gRPC 测试模块
# ============================================

run_grpc_tests() {
    log_section "gRPC 端口测试"

    # 测试1: 检查端口是否开放
    log_test "测试1: 检查 gRPC 端口是否开放"
    local result
    if command -v nc &> /dev/null; then
        result=$(nc -zv -w "$TIMEOUT" "$GRPC_HOST" "$GRPC_PORT" 2>&1 || echo "failed")
        if echo "$result" | grep -q "succeeded"; then
            log_pass "端口 ${GRPC_PORT} 是开放的"
            record_test_result "gRPC端口开放" "通过" "端口 ${GRPC_PORT} 已开放"
        else
            log_fail "端口 ${GRPC_PORT} 未开放或连接被拒绝"
            record_test_result "gRPC端口开放" "失败" "端口 ${GRPC_PORT} 未开放"
        fi
    elif command -v timeout &> /dev/null; then
        if timeout "$TIMEOUT" bash -c "cat < /dev/null > /dev/tcp/${GRPC_HOST}/${GRPC_PORT}" 2>/dev/null; then
            log_pass "端口 ${GRPC_PORT} 是开放的"
            record_test_result "gRPC端口开放" "通过" "端口 ${GRPC_PORT} 已开放"
        else
            log_fail "端口 ${GRPC_PORT} 未开放或连接被拒绝"
            record_test_result "gRPC端口开放" "失败" "端口 ${GRPC_PORT} 未开放"
        fi
    else
        log_skip "无法检查端口 (需要 nc 或 timeout)"
        record_test_result "gRPC端口开放" "跳过" "缺少 nc 或 timeout 命令"
    fi

    # 测试2: 检查进程监听状态
    log_test "测试2: 检查进程监听状态"
    local found=false

    if command -v netstat &> /dev/null; then
        if netstat -tuln 2>/dev/null | grep -q ":${GRPC_PORT}"; then
            log_pass "找到监听端口 ${GRPC_PORT} 的进程"
            record_test_result "gRPC进程监听" "通过" "netstat 显示端口 ${GRPC_PORT} 正在监听"
            found=true
        fi
    fi

    if [ "$found" = false ] && command -v ss &> /dev/null; then
        if ss -tuln 2>/dev/null | grep -q ":${GRPC_PORT}"; then
            log_pass "找到监听端口 ${GRPC_PORT} 的进程"
            record_test_result "gRPC进程监听" "通过" "ss 显示端口 ${GRPC_PORT} 正在监听"
            found=true
        fi
    fi

    if [ "$found" = false ]; then
        log_fail "未找到监听端口 ${GRPC_PORT} 的进程"
        record_test_result "gRPC进程监听" "失败" "未找到监听端口 ${GRPC_PORT} 的进程"
    fi

    # 测试3: 使用 grpcurl 测试 gRPC 服务
    log_test "测试3: 使用 grpcurl 测试 gRPC 服务"
    if command -v grpcurl &> /dev/null; then
        result=$(grpcurl -plaintext "${GRPC_HOST}:${GRPC_PORT}" list 2>&1 || echo "failed")

        if [ "$result" != "failed" ]; then
            log_pass "gRPC 服务响应正常"
            log_info "可用的服务: $result"
            record_test_result "gRPC服务响应" "通过" "gRPC 服务响应正常"
        else
            log_fail "gRPC 服务无响应"
            record_test_result "gRPC服务响应" "失败" "gRPC 服务无响应"
        fi
    else
        log_skip "grpcurl 未安装，跳过 gRPC 服务测试"
        record_test_result "gRPC服务响应" "跳过" "grpcurl 未安装"
    fi

    # 测试4: 使用 grpc-health-probe 测试
    log_test "测试4: 使用 grpc-health-probe 测试"
    if command -v grpc-health-probe &> /dev/null; then
        if grpc-health-probe -addr="${GRPC_HOST}:${GRPC_PORT}" 2>/dev/null; then
            log_pass "gRPC 健康检查通过"
            record_test_result "gRPC健康检查" "通过" "grpc-health-probe 检查通过"
        else
            log_fail "gRPC 健康检查失败"
            record_test_result "gRPC健康检查" "失败" "grpc-health-probe 检查失败"
        fi
    else
        log_skip "grpc-health-probe 未安装，跳过健康检查测试"
        record_test_result "gRPC健康检查" "跳过" "grpc-health-probe 未安装"
    fi

    # 测试5: 测试连接延迟
    log_test "测试5: 测试连接延迟"
    if command -v nc &> /dev/null; then
        local start_time end_time latency
        start_time=$(date +%s%N)
        nc -zv -w "$TIMEOUT" "$GRPC_HOST" "$GRPC_PORT" &> /dev/null
        end_time=$(date +%s%N)
        latency=$(( (end_time - start_time) / 1000000 ))

        if [ "$latency" -lt 100 ]; then
            log_pass "连接延迟: ${latency}ms (优秀)"
        elif [ "$latency" -lt 500 ]; then
            log_warn "连接延迟: ${latency}ms (一般)"
        else
            log_warn "连接延迟: ${latency}ms (较高)"
        fi
        record_test_result "gRPC连接延迟" "通过" "延迟: ${latency}ms"
    else
        log_skip "无法测试连接延迟 (需要 nc)"
        record_test_result "gRPC连接延迟" "跳过" "缺少 nc 命令"
    fi
}

# ============================================
# 性能测试模块
# ============================================

run_performance_tests() {
    log_section "性能测试"

    local concurrent_requests=10
    log_test "并发测试: $concurrent_requests 个并发请求"

    local pids=()
    local success_count=0
    local fail_count=0

    for i in $(seq 1 $concurrent_requests); do
        (
            local response=$(http_request "GET" "/health" "" "")
            local http_code=$(echo "$response" | tail -n1)
            if [ "$http_code" = "200" ]; then
                echo "success"
            else
                echo "fail"
            fi
        ) &
        pids+=($!)
    done

    for pid in "${pids[@]}"; do
        local result=$(wait $pid 2>/dev/null || echo "fail")
        if [ "$result" = "success" ]; then
            ((success_count++))
        else
            ((fail_count++))
        fi
    done

    if [ $fail_count -eq 0 ]; then
        log_pass "并发测试 - $concurrent_requests 个请求全部成功"
        record_test_result "并发测试" "通过" "$concurrent_requests 个并发请求全部成功"
    else
        log_warn "并发测试 - 成功: $success_count, 失败: $fail_count"
        record_test_result "并发测试" "警告" "成功: $success_count, 失败: $fail_count"
    fi
}

# 更新性能指标
update_performance() {
    local test_name=$1
    local response_time=$2

    RESPONSE_TIMES[$test_name]=$response_time

    local time_ms=$(echo "$response_time * 1000" | bc 2>/dev/null || echo "0")
    TOTAL_RESPONSE_TIME=$(echo "$TOTAL_RESPONSE_TIME + $response_time" | bc 2>/dev/null || echo "0")

    if [ "$time_ms" != "0" ]; then
        if (( $(echo "$time_ms < $MIN_RESPONSE_TIME" | bc -l 2>/dev/null || echo "0") )); then
            MIN_RESPONSE_TIME=$time_ms
        fi
        if (( $(echo "$time_ms > $MAX_RESPONSE_TIME" | bc -l 2>/dev/null || echo "0") )); then
            MAX_RESPONSE_TIME=$time_ms
        fi
    fi
}

# ============================================
# 边界条件测试模块
# ============================================

run_boundary_tests() {
    log_section "边界条件测试"

    # 测试1: 空工作区
    log_test "测试1: 空工作区"
    local request_data='{"workspace":"","group":"test-group","biz_tag":"test-tag"}'
    local response=$(http_request "POST" "/api/v1/generate" "$request_data" "")
    local http_code=$(echo "$response" | tail -n1)

    if [ "$http_code" = "400" ] || [ "$http_code" = "500" ]; then
        log_pass "空工作区 - 正确拒绝空工作区 (HTTP $http_code)"
        record_test_result "空工作区" "通过" "正确拒绝空工作区"
    else
        log_fail "空工作区 - 期望HTTP 400或500，实际: $http_code"
        record_test_result "空工作区" "失败" "期望HTTP 400或500，实际: $http_code"
    fi

    # 测试2: 无效ID解析
    log_test "测试2: 无效ID解析"
    request_data='{"id":"invalid-id","workspace":"test-workspace","group":"test-group","biz_tag":"test-tag"}'
    response=$(http_request "POST" "/api/v1/parse" "$request_data" "")
    http_code=$(echo "$response" | tail -n1)

    if [ "$http_code" = "400" ]; then
        log_pass "无效ID解析 - 正确拒绝无效ID (HTTP 400)"
        record_test_result "无效ID解析" "通过" "正确拒绝无效ID"
    else
        log_fail "无效ID解析 - 期望HTTP 400，实际: $http_code"
        record_test_result "无效ID解析" "失败" "期望HTTP 400，实际: $http_code"
    fi

    # 测试3: 404不存在的端点
    log_test "测试3: 404不存在的端点"
    response=$(http_request "GET" "/api/v1/nonexistent" "" "")
    http_code=$(echo "$response" | tail -n1)

    if [ "$http_code" = "404" ]; then
        log_pass "404处理 - 正确返回404"
        record_test_result "404处理" "通过" "正确返回HTTP 404"
    else
        log_fail "404处理 - 期望HTTP 404，实际: $http_code"
        record_test_result "404处理" "失败" "期望HTTP 404，实际: $http_code"
    fi

    # 测试4: 方法不允许
    log_test "测试4: 405方法不允许"
    response=$(http_request "GET" "/api/v1/generate" "" "")
    http_code=$(echo "$response" | tail -n1)

    if [ "$http_code" = "405" ]; then
        log_pass "405处理 - 正确返回405"
        record_test_result "405处理" "通过" "正确返回HTTP 405"
    else
        log_warn "405处理 - 期望HTTP 405，实际: $http_code（可能实现不同）"
        record_test_result "405处理" "警告" "HTTP状态码: $http_code"
    fi
}

# ============================================
# 生成报告
# ============================================

generate_report() {
    local pass_rate=0
    if [ $TOTAL_TESTS -gt 0 ]; then
        pass_rate=$(( PASSED_TESTS * 100 / TOTAL_TESTS ))
    fi

    local avg_response_time=0
    if [ $TOTAL_TESTS -gt 0 ] && command -v bc &> /dev/null; then
        avg_response_time=$(echo "scale=3; $TOTAL_RESPONSE_TIME / $TOTAL_TESTS" | bc 2>/dev/null || echo "0")
    fi

    # 写入测试结果到报告
    echo "" >> "$REPORT_FILE"
    echo "================================================================================" >> "$REPORT_FILE"
    echo "                            测试结果汇总" >> "$REPORT_FILE"
    echo "================================================================================" >> "$REPORT_FILE"
    echo "" >> "$REPORT_FILE"
    echo "总测试数:      $TOTAL_TESTS" >> "$REPORT_FILE"
    echo "通过测试数:    $PASSED_TESTS" >> "$REPORT_FILE"
    echo "失败测试数:    $FAILED_TESTS" >> "$REPORT_FILE"
    echo "跳过测试数:    $SKIPPED_TESTS" >> "$REPORT_FILE"
    echo "警告数:        $WARNINGS" >> "$REPORT_FILE"
    echo "通过率:        ${pass_rate}%" >> "$REPORT_FILE"
    echo "" >> "$REPORT_FILE"
    echo "================================================================================" >> "$REPORT_FILE"
    echo "                            性能指标" >> "$REPORT_FILE"
    echo "================================================================================" >> "$REPORT_FILE"
    echo "" >> "$REPORT_FILE"
    echo "平均响应时间:  ${avg_response_time}s" >> "$REPORT_FILE"
    echo "最小响应时间:  ${MIN_RESPONSE_TIME}ms" >> "$REPORT_FILE"
    echo "最大响应时间:  ${MAX_RESPONSE_TIME}ms" >> "$REPORT_FILE"
    echo "" >> "$REPORT_FILE"
    echo "================================================================================" >> "$REPORT_FILE"
    echo "                            测试详情" >> "$REPORT_FILE"
    echo "================================================================================" >> "$REPORT_FILE"
    echo "" >> "$REPORT_FILE"

    for result in "${TEST_RESULTS[@]}"; do
        IFS='|' read -r test_name status details response_time <<< "$result"
        echo "测试名称: $test_name" >> "$REPORT_FILE"
        echo "状态: $status" >> "$REPORT_FILE"
        echo "响应时间: $response_time" >> "$REPORT_FILE"
        echo "详情: $details" >> "$REPORT_FILE"
        echo "--------------------------------------------------------------------------------" >> "$REPORT_FILE"
    done

    # 输出汇总到控制台
    echo ""
    echo "================================================================================"
    echo "                            测试结果汇总"
    echo "================================================================================"
    echo "总测试数:      $TOTAL_TESTS"
    echo "通过测试数:    $PASSED_TESTS"
    echo "失败测试数:    $FAILED_TESTS"
    echo "跳过测试数:    $SKIPPED_TESTS"
    echo "警告数:        $WARNINGS"
    echo "通过率:        ${pass_rate}%"
    echo "================================================================================"
    echo ""
    log_info "测试报告已保存到: $REPORT_FILE"
}

# 显示帮助信息
show_help() {
    cat << EOF
用法: $0 [测试类型] [选项]

测试类型:
  all             运行所有测试（默认）
  api             仅运行 API 功能测试
  batch           仅运行批量参数验证测试
  grpc            仅运行 gRPC 端口测试
  performance     仅运行性能测试
  boundary        仅运行边界条件测试

选项:
  -h, --help      显示此帮助信息

环境变量:
  NEBULA_BASE_URL     HTTP API 基础 URL (默认: http://localhost:8080)
  NEBULA_GRPC_HOST    gRPC 服务主机 (默认: localhost)
  NEBULA_GRPC_PORT    gRPC 服务端口 (默认: 9091)

示例:
  $0                          # 运行所有测试
  $0 api                      # 仅运行 API 测试
  $0 grpc                     # 仅运行 gRPC 测试
  $0 all                      # 运行所有测试
  NEBULA_BASE_URL=http://192.168.1.100:8080 $0 api  # 测试远程服务

测试报告:
  报告保存在 docker/test_reports/ 目录
  报告文件名格式: combined_test_YYYYMMDD_HHMMSS.txt

EOF
}

# 主函数
main() {
    local test_type=${1:-"all"}

    # 检查是否请求帮助
    if [ "$test_type" = "-h" ] || [ "$test_type" = "--help" ]; then
        show_help
        exit 0
    fi

    echo "🧪 Nebula ID 统一测试脚本"
    echo "=========================="
    echo ""
    log_info "测试环境:"
    log_info "  HTTP API:  $BASE_URL"
    log_info "  gRPC:      ${GRPC_HOST}:${GRPC_PORT}"
    log_info "  超时设置:  ${TIMEOUT}秒"
    echo ""

    init_report

    # 根据测试类型运行相应的测试
    case $test_type in
        all)
            run_api_tests
            run_batch_validation_tests
            run_grpc_tests
            run_performance_tests
            run_boundary_tests
            ;;
        api)
            run_api_tests
            ;;
        batch)
            run_batch_validation_tests
            ;;
        grpc)
            run_grpc_tests
            ;;
        performance)
            run_performance_tests
            ;;
        boundary)
            run_boundary_tests
            ;;
        *)
            log_error "未知的测试类型: $test_type"
            echo ""
            show_help
            exit 1
            ;;
    esac

    generate_report

    echo ""
    echo "✅ 测试完成"

    # 返回退出码
    if [ $FAILED_TESTS -gt 0 ]; then
        exit 1
    fi
}

main "$@"